use ironrdp_core::{ensure_size, Encode, EncodeResult};
use ironrdp_dvc::DvcEncode;

pub(crate) struct RawDataDvcMessage(pub Vec<u8>);

impl Encode for RawDataDvcMessage {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RawDataDvcMessage"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawDataDvcMessage {}
