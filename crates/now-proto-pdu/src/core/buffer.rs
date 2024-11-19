//! Buffer types for NOW protocol.

use alloc::vec::Vec;

use ironrdp_core::{
    cast_length, ensure_fixed_part_size, ensure_size, invalid_field_err, Decode, DecodeResult, Encode, EncodeResult,
    ReadCursor, WriteCursor,
};

use crate::VarU32;

/// String value up to 2^32 bytes long.
///
/// NOW-PROTO: NOW_LRGBUF
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowLrgBuf(Vec<u8>);

impl NowLrgBuf {
    const NAME: &'static str = "NOW_LRGBUF";
    const FIXED_PART_SIZE: usize = 4;

    /// Create a new `NowLrgBuf` instance. Returns an error if the provided value is too large.
    pub fn new(value: impl Into<Vec<u8>>) -> DecodeResult<Self> {
        let value: Vec<u8> = value.into();

        if value.len() > VarU32::MAX as usize {
            return Err(invalid_field_err!("data", "data is too large for NOW_LRGBUF"));
        }

        Self::ensure_message_size(value.len())?;

        Ok(NowLrgBuf(value))
    }

    /// Get the buffer value.
    pub fn value(&self) -> &[u8] {
        self.0.as_slice()
    }

    fn ensure_message_size(buffer_size: usize) -> DecodeResult<()> {
        if buffer_size > usize::try_from(VarU32::MAX).expect("BUG: too small usize") {
            return Err(invalid_field_err!("data", "data is too large for NOW_LRGBUF"));
        }

        if buffer_size > usize::MAX - Self::FIXED_PART_SIZE {
            return Err(invalid_field_err!(
                "data",
                "data size is too large to fit in 32-bit usize"
            ));
        }

        Ok(())
    }
}

impl Encode for NowLrgBuf {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();
        ensure_size!(in: dst, size: encoded_size);

        let len: u32 = self.0.len().try_into().expect("BUG: validated in constructor");

        dst.write_u32(len);
        dst.write_slice(self.0.as_slice());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        // <u32 size> + <data bytes>
        Self::FIXED_PART_SIZE
            .checked_add(self.0.len())
            .expect("BUG: size overflow")
    }
}

impl Decode<'_> for NowLrgBuf {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let len: usize = cast_length!("len", src.read_u32())?;

        Self::ensure_message_size(len)?;

        ensure_size!(in: src, size: len);
        let bytes = src.read_slice(len);

        Ok(NowLrgBuf(bytes.to_vec()))
    }
}

impl From<NowLrgBuf> for Vec<u8> {
    fn from(buf: NowLrgBuf) -> Self {
        buf.0
    }
}

/// Buffer up to 2^31 bytes long (Length has compact variable length encoding).
///
/// NOW-PROTO: NOW_VARBUF
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowVarBuf(Vec<u8>);

impl NowVarBuf {
    const NAME: &'static str = "NOW_VARBUF";

    /// Create a new `NowVarBuf` instance. Returns an error if the provided value is too large.
    pub fn new(value: impl Into<Vec<u8>>) -> DecodeResult<Self> {
        let value = value.into();

        let _: u32 = value
            .len()
            .try_into()
            .ok()
            .and_then(|val| if val <= VarU32::MAX { Some(val) } else { None })
            .ok_or_else(|| invalid_field_err!("data", "too large buffer"))?;

        Ok(NowVarBuf(value))
    }

    /// Get the buffer value.
    pub fn value(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl Encode for NowVarBuf {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();
        ensure_size!(in: dst, size: encoded_size);

        let len: u32 = self.0.len().try_into().expect("BUG: validated in constructor");

        VarU32::new(len)?.encode(dst)?;
        dst.write_slice(self.0.as_slice());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        // <variable-length size> + <data bytes>
        // NOTE: Wrapping add will not overflow because the size is limited by VarU32::MAX
        VarU32::new(self.0.len().try_into().unwrap())
            .unwrap()
            .size()
            .wrapping_add(self.0.len())
    }
}

impl Decode<'_> for NowVarBuf {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let len_u32 = VarU32::decode(src)?.value();
        let len: usize = cast_length!("len", len_u32)?;

        ensure_size!(in: src, size: len);
        let bytes = src.read_slice(len);

        Ok(NowVarBuf(bytes.to_vec()))
    }
}

impl From<NowVarBuf> for Vec<u8> {
    fn from(buf: NowVarBuf) -> Self {
        buf.0
    }
}
