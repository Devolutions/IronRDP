//! Implements the RDPDR static virtual channel as described in
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]
//!
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

mod pdu;
use crate::pdu::efs::{
    CapabilityMessage, ClientNameRequest, ClientNameRequestUnicodeFlag, Component, CoreCapability, PacketId,
    SharedHeader, VersionAndIdPdu, VersionAndIdPduKind,
};
use ironrdp_pdu::{cursor::ReadCursor, gcc::ChannelName, PduEncode, PduResult};
use ironrdp_svc::{AsAny, CompressionCondition, StaticVirtualChannel};
use std::{any::Any, vec};
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
    /// See [`CapabilityMessage`]
    capabilities: Vec<CapabilityMessage>,
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
        }
    }

    fn handle_server_announce(&mut self, payload: &mut ReadCursor<'_>) -> PduResult<Vec<Box<dyn PduEncode>>> {
        let req = VersionAndIdPdu::decode(payload, VersionAndIdPduKind::ServerAnnounceRequest)?;
        trace!("received {:?}", req);

        let client_announce_reply = VersionAndIdPdu {
            version_major: 28,
            version_minor: 0,
            client_id: req.client_id,
            kind: VersionAndIdPduKind::ClientAnnounceReply,
        };
        trace!("sending {:?}", client_announce_reply);

        let client_name_request =
            ClientNameRequest::new(self.computer_name.clone(), ClientNameRequestUnicodeFlag::Unicode);
        trace!("sending {:?}", client_name_request);

        Ok(vec![Box::new(client_announce_reply), Box::new(client_name_request)])
    }

    fn handle_server_capability(&mut self, payload: &mut ReadCursor<'_>) -> PduResult<Vec<Box<dyn PduEncode>>> {
        let req = CoreCapability::decode(payload)?;
        trace!("received {:?}", req);

        let res = CoreCapability::new_response(self.capabilities.clone());
        trace!("sending {:?}", res);

        // TODO: Make CoreCapability PduEncode
        Ok(vec![Box::new(res)])
    }
}

impl AsAny for Rdpdr {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl StaticVirtualChannel for Rdpdr {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<Box<dyn PduEncode>>> {
        let mut payload = ReadCursor::new(payload);

        let header = SharedHeader::decode(&mut payload)?;
        trace!("received {:?}", header);

        if let Component::RdpdrCtypPrn = header.component {
            warn!(
                "received {:?} RDPDR header from RDP server, printer redirection is unimplemented",
                Component::RdpdrCtypPrn
            );
            return Ok(vec![]);
        }

        match header.packet_id {
            PacketId::CoreServerAnnounce => self.handle_server_announce(&mut payload),
            PacketId::CoreServerCapability => self.handle_server_capability(&mut payload),
            _ => {
                warn!("received unimplemented packet: {:?}", header.packet_id);
                Ok(vec![])
            }
        }
    }
}
