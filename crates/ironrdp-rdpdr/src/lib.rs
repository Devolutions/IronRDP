//! Implements the RDPDR static virtual channel as described in
//! [[MS-RDPEFS]: Remote Desktop Protocol: File System Virtual Channel Extension](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5)

mod pdu;
use crate::pdu::efs::{Component, PacketId, SharedHeader, VersionAndIdPDU, VersionAndIdPDUKind};
use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    encode_buf,
    gcc::ChannelName,
    write_buf::WriteBuf,
    PduDecode, PduEncode, PduResult,
};
use ironrdp_svc::{AsAny, CompressionCondition, StaticVirtualChannel};
use std::any::Any;
use tracing::{trace, warn};

/// The RDPDR channel as specified in [MS-RDPEFS].
///
/// This channel must always be advertised with the "rdpsnd"
/// channel in order for the server to send anything back to it,
/// see: https://tinyurl.com/2fvrtfjd.
#[derive(Debug)]
pub struct Rdpdr;

impl Default for Rdpdr {
    fn default() -> Self {
        Self::new()
    }
}

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    pub fn new() -> Self {
        Self
    }

    fn handle_server_announce(&mut self, payload: &mut ReadCursor<'_>, output: &mut WriteBuf) -> PduResult<()> {
        let req = VersionAndIdPDU::decode(payload, VersionAndIdPDUKind::ServerAnnounceRequest)?;
        trace!("{:?}", req);
        let _res = VersionAndIdPDU::new(28, 0, req.client_id, VersionAndIdPDUKind::ClientAnnounceReply);
        // let _ = encode_buf(&res, output)?;
        Ok(())
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

    fn process(&mut self, initiator_id: u16, channel_id: u16, payload: &[u8], output: &mut WriteBuf) -> PduResult<()> {
        let mut payload = ReadCursor::new(payload);

        let header = SharedHeader::decode(&mut payload)?;
        trace!("{:?}", header);

        if let Component::RDPDR_CTYP_PRN = header.component {
            warn!(
                "received {:?} RDPDR header from RDP server, printer redirection is unimplemented",
                Component::RDPDR_CTYP_PRN
            );
            return Ok(());
        }

        match header.packet_id {
            PacketId::PAKID_CORE_SERVER_ANNOUNCE => self.handle_server_announce(&mut payload, output)?,
            _ => {
                warn!("received unimplemented packet: {:?}", header.packet_id);
                return Ok(());
            }
        }

        warn!("received data, protocol is unimplemented");
        Ok(())
    }
}
