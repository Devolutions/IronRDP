use ironrdp_pdu::{gcc::ChannelName, PduEncode, PduResult};
use ironrdp_svc::{AsAny, CompressionCondition, StaticVirtualChannel};
use std::any::Any;

/// We currently don't implement any of rdpsnd, however it's required
/// for rdpdr to work: [\[MS-RDPEFS\] Appendix A<1>](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1)
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

impl AsAny for Rdpsnd {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl StaticVirtualChannel for Rdpsnd {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, _payload: &[u8]) -> PduResult<Vec<Box<dyn PduEncode>>> {
        Err(ironrdp_pdu::other_err!(
            "RDPSND",
            "ironrdp-rdpsnd::Rdpsnd implemention is not implemented"
        ))
    }
}
