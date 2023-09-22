use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::PduResult;
use ironrdp_svc::{impl_as_any, ChunkProcessor, CompressionCondition, StaticVirtualChannel, SvcMessage};

/// We currently don't implement any of rdpsnd, however it's required
/// for rdpdr to work: [\[MS-RDPEFS\] Appendix A<1>]
///
/// [\[MS-RDPEFS\] Appendix A<1>]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
#[derive(Debug)]
pub struct Rdpsnd {
    preprocessor: ChunkProcessor,
}

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new() -> Self {
        Self {
            preprocessor: ChunkProcessor::new(),
        }
    }
}

impl Default for Rdpsnd {
    fn default() -> Self {
        Self::new()
    }
}

impl_as_any!(Rdpsnd);

impl StaticVirtualChannel for Rdpsnd {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn preprocessor(&self) -> &ChunkProcessor {
        &self.preprocessor
    }

    fn preprocessor_mut(&mut self) -> &mut ChunkProcessor {
        &mut self.preprocessor
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, _payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        Err(ironrdp_pdu::other_err!(
            "RDPSND",
            "ironrdp-rdpsnd::Rdpsnd is not implemented"
        ))
    }
}
