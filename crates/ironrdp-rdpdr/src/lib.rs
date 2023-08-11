use ironrdp_pdu::{gcc::ChannelName, write_buf::WriteBuf, PduResult};
use ironrdp_svc::{CompressionCondition, StaticVirtualChannel};
use tracing::warn;

/// The RDPDR channel as specified in [MS-RDPEFS].
///
/// This channel must always be advertised with the "rdpsnd"
/// channel in order for the server to send anything back to it,
/// see: https://tinyurl.com/2fvrtfjd.
#[derive(Debug)]
pub struct Rdpdr;

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    pub fn new() -> Self {
        Self
    }
}

impl Default for Rdpdr {
    fn default() -> Self {
        Self::new()
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
        warn!("rdpdr channel received data, protocol is unimplemented");
        Ok(())
    }
}
