use ironrdp_core::{EncodeResult, decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{
    CsReadyFlags, CsReadyPdu, DismissHoveringTouchContactPdu, PenEventPdu, RdpInputProtocolVersion, RdpeiPdu,
    ScReadyFeatures, TouchEventPdu,
};

/// Client endpoint for the MS-RDPEI dynamic channel.
#[derive(Debug)]
pub struct RdpeiClient {
    max_touch_contacts: u16,
    cs_ready_flags: CsReadyFlags,
    /// Protocol version the client will advertise in CS_READY.
    advertise_version: RdpInputProtocolVersion,
    ready: bool,
    suspended: bool,
    negotiated_version: Option<RdpInputProtocolVersion>,
    server_features: Option<ScReadyFeatures>,
}

impl Default for RdpeiClient {
    fn default() -> Self {
        Self::new(10, CsReadyFlags::empty(), RdpInputProtocolVersion::V200)
    }
}

impl RdpeiClient {
    /// Creates a new client that will reply to `SC_READY` with the given capabilities.
    pub fn new(
        max_touch_contacts: u16,
        cs_ready_flags: CsReadyFlags,
        advertise_version: RdpInputProtocolVersion,
    ) -> Self {
        Self {
            max_touch_contacts,
            cs_ready_flags,
            advertise_version,
            ready: false,
            suspended: false,
            negotiated_version: None,
            server_features: None,
        }
    }

    /// True after a successful SC_READY / CS_READY exchange.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// True while the server has suspended input injection.
    #[must_use]
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Negotiated server protocol version, if known.
    #[must_use]
    pub fn negotiated_version(&self) -> Option<RdpInputProtocolVersion> {
        self.negotiated_version
    }

    /// Whether pen frames may be sent given the negotiated version.
    #[must_use]
    pub fn pen_allowed(&self) -> bool {
        self.negotiated_version
            .is_some_and(RdpInputProtocolVersion::supports_pen)
    }

    /// Encodes a touch event when the channel is ready and not suspended.
    pub fn encode_touch_event(&self, channel_id: u32, event: TouchEventPdu) -> EncodeResult<Vec<SvcMessage>> {
        self.ensure_can_send_input()?;
        encode_dvc_messages(
            channel_id,
            vec![Box::new(RdpeiPdu::Touch(event))],
            ChannelFlags::empty(),
        )
    }

    /// Encodes a pen event when the channel is ready, not suspended, and pen is allowed.
    pub fn encode_pen_event(&self, channel_id: u32, event: PenEventPdu) -> EncodeResult<Vec<SvcMessage>> {
        self.ensure_can_send_input()?;
        if !self.pen_allowed() {
            return Err(ironrdp_core::other_err!(
                "RdpeiClient::encode_pen_event",
                "pen frames not allowed for negotiated protocol version"
            ));
        }
        encode_dvc_messages(channel_id, vec![Box::new(RdpeiPdu::Pen(event))], ChannelFlags::empty())
    }

    /// Encodes a dismiss-hovering-touch-contact PDU when the channel is ready.
    pub fn encode_dismiss_hovering(&self, channel_id: u32, contact_id: u8) -> EncodeResult<Vec<SvcMessage>> {
        if !self.ready {
            return Err(ironrdp_core::other_err!(
                "RdpeiClient::encode_dismiss_hovering",
                "rdpei channel is not ready"
            ));
        }
        encode_dvc_messages(
            channel_id,
            vec![Box::new(RdpeiPdu::DismissHoveringTouchContact(
                DismissHoveringTouchContactPdu::new(contact_id),
            ))],
            ChannelFlags::empty(),
        )
    }

    fn ensure_can_send_input(&self) -> EncodeResult<()> {
        if !self.ready {
            return Err(ironrdp_core::other_err!("RdpeiClient", "rdpei channel is not ready"));
        }
        if self.suspended {
            return Err(ironrdp_core::other_err!("RdpeiClient", "rdpei input is suspended"));
        }
        Ok(())
    }

    fn build_cs_ready(&self, server_version: RdpInputProtocolVersion) -> CsReadyPdu {
        // Advertise our preferred version; when the server only supports touch-only
        // versions, match the server so negotiation stays valid.
        let version = match server_version {
            RdpInputProtocolVersion::V100 | RdpInputProtocolVersion::V101
                if self.advertise_version > server_version =>
            {
                server_version
            }
            _ => self.advertise_version,
        };

        let mut flags = self.cs_ready_flags;
        // Do not send DISABLE_TIMESTAMP_INJECTION to V100-only servers.
        if server_version == RdpInputProtocolVersion::V100 {
            flags.remove(CsReadyFlags::DISABLE_TIMESTAMP_INJECTION);
        }

        CsReadyPdu::new(flags, version, self.max_touch_contacts)
    }
}

impl_as_any!(RdpeiClient);

impl DvcProcessor for RdpeiClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        // Server initiates with SC_READY (MS-RDPEI 3.2.3 / 3.3.3).
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: RdpeiPdu = decode(payload).map_err(|e| decode_err!(e))?;

        match pdu {
            RdpeiPdu::ScReady(sc) => {
                debug!(
                    version = ?sc.protocol_version,
                    features = ?sc.supported_features,
                    "Received RDPEI SC_READY"
                );
                self.server_features = sc.supported_features;
                self.suspended = false;

                let cs = self.build_cs_ready(sc.protocol_version);
                // Effective version is the minimum of what both endpoints advertise.
                self.negotiated_version = Some(core::cmp::min(sc.protocol_version, cs.protocol_version));
                self.ready = true;
                debug!(
                    version = ?cs.protocol_version,
                    negotiated = ?self.negotiated_version,
                    max_touch_contacts = cs.max_touch_contacts,
                    flags = ?cs.flags,
                    "Sending RDPEI CS_READY"
                );
                Ok(vec![Box::new(RdpeiPdu::CsReady(cs))])
            }
            RdpeiPdu::SuspendInput => {
                debug!("RDPEI input suspended by server");
                self.suspended = true;
                Ok(Vec::new())
            }
            RdpeiPdu::ResumeInput => {
                debug!("RDPEI input resumed by server");
                self.suspended = false;
                Ok(Vec::new())
            }
            RdpeiPdu::CsReady(_) | RdpeiPdu::Touch(_) | RdpeiPdu::Pen(_) | RdpeiPdu::DismissHoveringTouchContact(_) => {
                Err(pdu_other_err!(
                    "RdpeiClient::process",
                    "received unexpected client-to-server RDPEI PDU from server"
                ))
            }
        }
    }
}

impl DvcClientProcessor for RdpeiClient {}
