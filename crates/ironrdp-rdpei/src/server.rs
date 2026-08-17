use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{
    CsReadyPdu, DismissHoveringTouchContactPdu, PenEventPdu, RdpInputProtocolVersion, RdpeiPdu, ScReadyFeatures,
    ScReadyPdu, TouchEventPdu,
};

/// Handler callbacks for RDPEI server-side messages from the client.
pub trait RdpeiHandler: Send {
    fn cs_ready(&mut self, pdu: CsReadyPdu) {
        debug!(?pdu, "RDPEI CS_READY");
    }

    fn touch(&mut self, pdu: TouchEventPdu) {
        debug!(frames = pdu.frames.len(), "RDPEI touch event");
    }

    fn pen(&mut self, pdu: PenEventPdu) {
        debug!(frames = pdu.frames.len(), "RDPEI pen event");
    }

    fn dismiss_hovering(&mut self, pdu: DismissHoveringTouchContactPdu) {
        debug!(contact_id = pdu.contact_id, "RDPEI dismiss hovering");
    }
}

/// Server endpoint for the MS-RDPEI dynamic channel.
pub struct RdpeiServer {
    handler: Box<dyn RdpeiHandler>,
    protocol_version: RdpInputProtocolVersion,
    supported_features: Option<ScReadyFeatures>,
}

impl RdpeiServer {
    /// Create a server that advertises the given protocol version on channel start.
    pub fn new(handler: Box<dyn RdpeiHandler>) -> Self {
        Self {
            handler,
            protocol_version: RdpInputProtocolVersion::V200,
            supported_features: None,
        }
    }

    #[must_use]
    pub fn with_protocol_version(mut self, version: RdpInputProtocolVersion) -> Self {
        self.protocol_version = version;
        self
    }

    #[must_use]
    pub fn with_supported_features(mut self, features: ScReadyFeatures) -> Self {
        self.supported_features = Some(features);
        self
    }
}

impl_as_any!(RdpeiServer);

impl DvcProcessor for RdpeiServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let mut sc = ScReadyPdu::new(self.protocol_version);
        if let Some(features) = self.supported_features {
            sc = sc.with_features(features);
        }
        debug!(version = ?self.protocol_version, "Sending RDPEI SC_READY");
        Ok(vec![Box::new(RdpeiPdu::ScReady(sc))])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode(payload).map_err(|e| decode_err!(e))? {
            RdpeiPdu::CsReady(pdu) => self.handler.cs_ready(pdu),
            RdpeiPdu::Touch(pdu) => self.handler.touch(pdu),
            RdpeiPdu::Pen(pdu) => self.handler.pen(pdu),
            RdpeiPdu::DismissHoveringTouchContact(pdu) => self.handler.dismiss_hovering(pdu),
            RdpeiPdu::ScReady(_) | RdpeiPdu::SuspendInput | RdpeiPdu::ResumeInput => {
                return Err(pdu_other_err!(
                    "RdpeiServer::process",
                    "received unexpected server-to-client RDPEI PDU from client"
                ));
            }
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for RdpeiServer {}
