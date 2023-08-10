use ironrdp_pdu::{gcc::ChannelName, write_buf::WriteBuf, PduResult};
use ironrdp_svc::{CompressionCondition, MakeStaticVirtualChannel, StaticVirtualChannel};
use tracing::{debug, warn};

#[derive(Debug)]
pub struct WithRdpdr;

impl WithRdpdr {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for WithRdpdr {
    fn default() -> Self {
        Self::new()
    }
}

impl MakeStaticVirtualChannel for WithRdpdr {
    fn channel_name(&self) -> ChannelName {
        Rdpdr::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn make_static_channel(&self, channel_id: u16) -> Box<dyn StaticVirtualChannel> {
        debug!(channel_id, "rdpdr static channel created");
        Box::new(Rdpdr {})
    }
}

/// The RDPDR channel as specified in [MS-RDPEFS].
///
/// This channel must always be advertised with the "rdpsnd"
/// channel in order for the server to send anything back to it,
/// see: https://tinyurl.com/2fvrtfjd.
#[derive(Debug)]
pub struct Rdpdr;

impl Rdpdr {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");
}

impl StaticVirtualChannel for Rdpdr {
    fn process(&mut self, initiator_id: u16, channel_id: u16, payload: &[u8], output: &mut WriteBuf) -> PduResult<()> {
        warn!("rdpdr channel received data, protocol is unimplemented");
        Ok(())
    }
}
