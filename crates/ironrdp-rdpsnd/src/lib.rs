use ironrdp_pdu::{gcc::ChannelName, write_buf::WriteBuf, PduResult};
use ironrdp_svc::{CompressionCondition, StaticVirtualChannel};
use tracing::warn;

/// We currently don't implement any of rdpsnd, however it's required
/// for rdpdr to work: https://tinyurl.com/2fvrtfjd
#[derive(Debug)]
pub struct Rdpsnd;

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new() -> Self {
        Self
    }
}

impl Default for Rdpsnd {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticVirtualChannel for Rdpsnd {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, initiator_id: u16, channel_id: u16, payload: &[u8], output: &mut WriteBuf) -> PduResult<()> {
        warn!("rdpsnd channel received data, protocol is unimplemented");
        Ok(())
    }
}
