//! Implements the RDPDR static virtual channel as described in
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]
//!
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

mod pdu;
use crate::pdu::{
    efs::{
        CapabilityMessage, ClientNameRequest, ClientNameRequestUnicodeFlag, CoreCapability, CoreCapabilityKind,
        VersionAndIdPdu, VersionAndIdPduKind,
    },
    RdpdrPdu,
};
use ironrdp_pdu::{decode, gcc::ChannelName, other_err, PduResult};
use ironrdp_svc::{impl_as_any, CompressionCondition, StaticVirtualChannel, SvcMessage, SvcPreprocessor};
use tracing::{trace, warn};

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
    capabilities: Vec<CapabilityMessage>,
    preprocessor: SvcPreprocessor,
}

impl Default for Rdpdr {
    fn default() -> Self {
        Self::new("IronRDP".to_string(), vec![CapabilityMessage::new_general(0)])
    }
}

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    pub fn new(computer_name: String, capabilities: Vec<CapabilityMessage>) -> Self {
        Self {
            computer_name,
            capabilities,
            preprocessor: SvcPreprocessor::new(),
        }
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
        let res = RdpdrPdu::CoreCapability(CoreCapability::new_response(self.capabilities.clone()));
        trace!("sending {:?}", res);

        // TODO: Make CoreCapability PduEncode
        Ok(vec![SvcMessage::from(res)])
    }
}

impl_as_any!(Rdpdr);

impl StaticVirtualChannel for Rdpdr {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn preprocessor(&self) -> &SvcPreprocessor {
        &self.preprocessor
    }

    fn preprocessor_mut(&mut self) -> &mut SvcPreprocessor {
        &mut self.preprocessor
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
            RdpdrPdu::Unimplemented => {
                warn!("received unimplemented packet: {:?}", pdu);
                Ok(vec![])
            }
            _ => Err(other_err!("rdpdr", "internal error")),
        }
    }
}
