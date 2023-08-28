//! Implements the RDPDR static virtual channel as described in
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]
//!
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

mod pdu;

use std::vec;

use ironrdp_pdu::cursor::ReadCursor;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::PduResult;
use ironrdp_svc::{impl_as_any, CompressionCondition, StaticVirtualChannel, SvcMessage};
use tracing::{trace, warn};

use crate::pdu::efs::{
    ClientNameRequest, ClientNameRequestUnicodeFlag, Component, PacketId, SharedHeader, VersionAndIdPdu,
    VersionAndIdPduKind,
};

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
}

impl Default for Rdpdr {
    fn default() -> Self {
        Self::new("IronRDP".to_string())
    }
}

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    pub fn new(computer_name: String) -> Self {
        Self { computer_name }
    }

    fn handle_server_announce(&mut self, payload: &mut ReadCursor<'_>) -> PduResult<Vec<SvcMessage>> {
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

        Ok(vec![
            SvcMessage::from(client_announce_reply),
            SvcMessage::from(client_name_request),
        ])
    }
}

impl_as_any!(Rdpdr);

impl StaticVirtualChannel for Rdpdr {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
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
            _ => {
                warn!("received unimplemented packet: {:?}", header.packet_id);
                Ok(vec![])
            }
        }
    }
}
