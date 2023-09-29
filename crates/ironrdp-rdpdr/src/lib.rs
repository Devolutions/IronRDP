#![doc = include_str!("../README.md")]
#![allow(clippy::arithmetic_side_effects)] // FIXME: remove
#![allow(clippy::cast_lossless)] // FIXME: remove
#![allow(clippy::cast_possible_truncation)] // FIXME: remove
#![allow(clippy::cast_possible_wrap)] // FIXME: remove
#![allow(clippy::cast_sign_loss)] // FIXME: remove

pub mod pdu;

use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{decode, other_err, PduResult};
use ironrdp_svc::{impl_as_any, CompressionCondition, StaticVirtualChannelProcessor, SvcMessage};
use tracing::{trace, warn};

use crate::pdu::efs::{
    Capabilities, ClientDeviceListAnnounce, ClientNameRequest, ClientNameRequestUnicodeFlag, CoreCapability,
    CoreCapabilityKind, Devices, VersionAndIdPdu, VersionAndIdPduKind,
};
use crate::pdu::RdpdrPdu;

/// The RDPDR channel as specified in [\[MS-RDPEFS\]].
///
/// This channel must always be advertised with the "rdpsnd"
/// channel in order for the server to send anything back to it,
/// see: [\[MS-RDPEFS\] Appendix A<1>]
///
/// [\[MS-RDPEFS\]]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5
/// [\[MS-RDPEFS\] Appendix A<1>]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
#[derive(Debug)]
pub struct Rdpdr {
    /// TODO: explain what this is
    computer_name: String,
    capabilities: Capabilities,
    /// Pre-configured list of devices to announce to the server.
    ///
    /// All devices not of the type [`DeviceType::Filesystem`] must be declared here.
    device_list: Devices,
}

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    /// Creates a new [`Rdpdr`].
    pub fn new(computer_name: String) -> Self {
        Self {
            computer_name,
            capabilities: Capabilities::new(),
            device_list: Devices::new(),
        }
    }

    #[must_use]
    pub fn with_smartcard(mut self, device_id: u32) -> Self {
        self.device_list.add_smartcard(device_id);
        self.capabilities.add_smartcard();
        self
    }

    fn handle_server_announce(&mut self, req: VersionAndIdPdu) -> PduResult<Vec<SvcMessage>> {
        let client_announce_reply = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu::new_client_announce_reply(req)?);
        trace!("sending {:?}", client_announce_reply);

        let client_name_request = RdpdrPdu::ClientNameRequest(ClientNameRequest::new(
            self.computer_name.clone(),
            ClientNameRequestUnicodeFlag::Unicode,
        ));
        trace!("sending {:?}", client_name_request);

        Ok(vec![
            SvcMessage::from(client_announce_reply),
            SvcMessage::from(client_name_request),
        ])
    }

    fn handle_server_capability(&mut self, _req: CoreCapability) -> PduResult<Vec<SvcMessage>> {
        let res = RdpdrPdu::CoreCapability(CoreCapability::new_response(self.capabilities.clone_inner()));
        trace!("sending {:?}", res);
        Ok(vec![SvcMessage::from(res)])
    }

    fn handle_client_id_confirm(&mut self) -> PduResult<Vec<SvcMessage>> {
        let res = RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce {
            device_list: self.device_list.clone_inner(),
        });
        trace!("sending {:?}", res);
        Ok(vec![SvcMessage::from(res)])
    }
}

impl_as_any!(Rdpdr);

impl StaticVirtualChannelProcessor for Rdpdr {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode::<RdpdrPdu>(payload)?;
        trace!("received {:?}", pdu);

        match pdu {
            RdpdrPdu::VersionAndIdPdu(pdu) if pdu.kind == VersionAndIdPduKind::ServerAnnounceRequest => {
                self.handle_server_announce(pdu)
            }
            RdpdrPdu::CoreCapability(pdu) if pdu.kind == CoreCapabilityKind::ServerCoreCapabilityRequest => {
                self.handle_server_capability(pdu)
            }
            RdpdrPdu::VersionAndIdPdu(pdu) if pdu.kind == VersionAndIdPduKind::ServerClientIdConfirm => {
                self.handle_client_id_confirm()
            }
            RdpdrPdu::Unimplemented => {
                warn!("received unimplemented packet: {:?}", pdu);
                Ok(Vec::new())
            }
            _ => Err(other_err!("rdpdr", "internal error")),
        }
    }
}
