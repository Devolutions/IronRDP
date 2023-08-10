use ironrdp_pdu::{gcc::ChannelName, write_buf::WriteBuf, PduResult};
use ironrdp_svc::{CompressionCondition, MakeStaticVirtualChannel, StaticVirtualChannel};
use tracing::{debug, warn};

#[derive(Debug)]
pub struct WithRdpsnd;

impl WithRdpsnd {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for WithRdpsnd {
    fn default() -> Self {
        Self::new()
    }
}

impl MakeStaticVirtualChannel for WithRdpsnd {
    fn channel_name(&self) -> ChannelName {
        Rdpsnd::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn make_static_channel(&self, channel_id: u16) -> Box<dyn StaticVirtualChannel> {
        debug!(channel_id, "rdpsnd static channel created");
        Box::new(Rdpsnd {})
    }
}

/// We currently don't implement any of rdpsnd, however it's required
/// for rdpdr to work: https://tinyurl.com/2fvrtfjd
#[derive(Debug)]
pub struct Rdpsnd;

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");
}

impl StaticVirtualChannel for Rdpsnd {
    fn process(&mut self, initiator_id: u16, channel_id: u16, payload: &[u8], output: &mut WriteBuf) -> PduResult<()> {
        warn!("rdpsnd channel received data, protocol is unimplemented");
        Ok(())
    }
}
