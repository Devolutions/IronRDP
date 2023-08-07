use ironrdp_pdu::{gcc::ChannelName, write_buf::WriteBuf, PduResult};
use ironrdp_svc::{CompressionCondition, MakeStaticVirtualChannel, StaticVirtualChannel};
use tracing::debug;

#[derive(Debug)]
pub struct WithEfs;

impl WithEfs {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for WithEfs {
    fn default() -> Self {
        Self::new()
    }
}

impl MakeStaticVirtualChannel for WithEfs {
    fn channel_name(&self) -> ChannelName {
        Efs::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn make_static_channel(&self, channel_id: u16) -> Box<dyn StaticVirtualChannel> {
        debug!(channel_id, "RDPDR channel created");
        Box::new(Efs {})
    }
}

#[derive(Debug)]
pub struct Efs;

impl Efs {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");
}

impl StaticVirtualChannel for Efs {
    fn process(&mut self, initiator_id: u16, channel_id: u16, payload: &[u8], output: &mut WriteBuf) -> PduResult<()> {
        Ok(())
    }
}
