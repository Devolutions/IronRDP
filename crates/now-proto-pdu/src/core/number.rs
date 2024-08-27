//! Variable-length number types.
use ironrdp_core::{
    ensure_size, invalid_field_err, DecodeError, DecodeResult, EncodeError, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_core::{Decode, Encode};

/// Variable-length encoded u16.
/// Value range:`[0..0x7FFF]`
///
/// NOW-PROTO: NOW_VARU16
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarU16(u16);

impl VarU16 {
    pub const MIN: u16 = 0x0000;
    pub const MAX: u16 = 0x7FFF;

    const NAME: &'static str = "NOW_VARU16";

    pub fn new(value: u16) -> DecodeResult<Self> {
        if value > Self::MAX {
            return Err(invalid_field_err!("value", "too large number"));
        }

        Ok(VarU16(value))
    }

    pub fn value(&self) -> u16 {
        self.0
    }
}

impl Encode for VarU16 {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();

        ensure_size!(in: dst, size: encoded_size);

        // LINTS: encoded_size will always be 1 or 2, therefore following arithmetic is safe
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = (encoded_size - 1) * 8;
        let mut bytes = [0u8; 2];

        for byte in bytes.iter_mut().take(encoded_size) {
            *byte = ((self.0 >> shift) & 0xFF).try_into().unwrap();

            // LINTS: as per code above, shift is always 8 or 16
            #[allow(clippy::arithmetic_side_effects)]
            if shift != 0 {
                shift -= 8;
            }
        }

        // LINTS: encoded_size is always >= 1
        #[allow(clippy::arithmetic_side_effects)]
        let c: u8 = (encoded_size - 1).try_into().unwrap();
        bytes[0] |= c << 7;

        dst.write_slice(&bytes[..encoded_size]);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self.0 {
            0x00..=0x7F => 1,
            0x80..=0x7FFF => 2,
            _ => unreachable!("BUG: value is out of range!"),
        }
    }
}

impl Decode<'_> for VarU16 {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Ensure we have at least 1 byte available to determine the size of the value
        ensure_size!(in: src, size: 1);

        let header = src.read_u8();
        let c: usize = ((header >> 7) & 0x01).into();

        if c == 0 {
            return Ok(VarU16((header & 0x7F).into()));
        }

        ensure_size!(in: src, size: c);
        let bytes = src.read_slice(c);

        let val1 = header & 0x7F;
        // LINTS: c is always 1 or 2
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = c * 8;
        let mut num = u16::from(val1) << shift;

        // Read val2..valN
        // LINTS: shift is always 8 or 16
        #[allow(clippy::arithmetic_side_effects)]
        for val in bytes.iter().take(c) {
            shift -= 8;
            num |= (u16::from(*val)) << shift;
        }

        Ok(VarU16(num))
    }
}

impl From<VarU16> for u16 {
    fn from(value: VarU16) -> Self {
        value.value()
    }
}

impl TryFrom<u16> for VarU16 {
    type Error = DecodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Variable-length encoded i16.
/// Value range:`[-0x3FFF..0x3FFF]`
///
/// NOW-PROTO: NOW_VARI16
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarI16(i16);

impl VarI16 {
    pub const MIN: i16 = -0x3FFF;
    pub const MAX: i16 = 0x3FFF;

    const NAME: &'static str = "NOW_VARI16";

    pub fn new(value: i16) -> DecodeResult<Self> {
        if value.abs() > Self::MAX {
            return Err(invalid_field_err!("value", "too large number"));
        }

        Ok(VarI16(value))
    }

    pub fn value(&self) -> i16 {
        self.0
    }
}

impl Encode for VarI16 {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();

        ensure_size!(in: dst, size: encoded_size);

        // LINTS: encoded_size will always be 1 or 2, therefore following arithmetic is safe
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = (encoded_size - 1) * 8;
        let mut bytes = [0u8; 2];

        let abs_value = self.0.unsigned_abs();

        for byte in bytes.iter_mut().take(encoded_size) {
            *byte = ((abs_value >> shift) & 0xFF).try_into().unwrap();

            // LINTS: as per code above, shift is always 8 or 16
            #[allow(clippy::arithmetic_side_effects)]
            if shift != 0 {
                shift -= 8;
            }
        }

        // LINTS: encoded_size is always >= 1
        #[allow(clippy::arithmetic_side_effects)]
        let c: u8 = (encoded_size - 1).try_into().unwrap();
        bytes[0] |= c << 7;
        if self.0 < 0 {
            // set sign bit
            bytes[0] |= 0x40;
        }

        dst.write_slice(&bytes[..encoded_size]);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self.0.unsigned_abs() {
            0..=0x3F => 1,
            0x40..=0x3FFF => 2,
            _ => unreachable!("BUG: value is out of range!"),
        }
    }
}

impl Decode<'_> for VarI16 {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Ensure we have at least 1 byte available to determine the size of the value
        ensure_size!(in: src, size: 1);

        let header = src.read_u8();
        let c: usize = ((header >> 7) & 0x01).into();
        let is_negative = (header & 0x40) != 0;

        if c == 0 {
            let val = i16::from(header & 0x3F);
            // LINTS: Variable integer range is always smaller than underlying type range,
            // therefore negation is always safe
            #[allow(clippy::arithmetic_side_effects)]
            return Ok(VarI16(if is_negative { -val } else { val }));
        }

        ensure_size!(in: src, size: c);
        let bytes = src.read_slice(c);

        let val1 = header & 0x3F;

        // LINTS: c is always 1 or 2
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = c * 8;
        let mut num = i16::from(val1) << shift;

        // Read val2..valN
        // LINTS: shift is always 8 or 16
        #[allow(clippy::arithmetic_side_effects)]
        for val in bytes.iter().take(c) {
            shift -= 8;
            num |= (i16::from(*val)) << shift;
        }

        // LINTS: Variable integer range is always smaller than underlying type range,
        // therefore negation is always safe
        #[allow(clippy::arithmetic_side_effects)]
        Ok(VarI16(if is_negative { -num } else { num }))
    }
}

impl From<VarI16> for i16 {
    fn from(value: VarI16) -> Self {
        value.value()
    }
}

impl TryFrom<i16> for VarI16 {
    type Error = DecodeError;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Variable-length encoded u32.
/// Value range: `[0..0x3FFFFFFF]`
///
/// NOW-PROTO: NOW_VARU32
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarU32(u32);

impl VarU32 {
    pub const MIN: u32 = 0x00000000;
    pub const MAX: u32 = 0x3FFFFFFF;

    const NAME: &'static str = "NOW_VARU32";

    pub fn new(value: u32) -> EncodeResult<Self> {
        if value > Self::MAX {
            return Err(invalid_field_err!("value", "too large number"));
        }

        Ok(VarU32(value))
    }

    pub fn value(&self) -> u32 {
        self.0
    }
}

impl Encode for VarU32 {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();

        ensure_size!(in: dst, size: encoded_size);

        // LINTS: encoded_size will always be [1..4], therefore following arithmetic is safe
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = (encoded_size - 1) * 8;
        let mut bytes = [0u8; 4];

        for byte in bytes.iter_mut().take(encoded_size) {
            *byte = ((self.0 >> shift) & 0xFF).try_into().unwrap();

            // LINTS: as per code above, shift is always 8, 16, 24
            #[allow(clippy::arithmetic_side_effects)]
            if shift != 0 {
                shift -= 8;
            }
        }

        // LINTS: encoded_size is always >= 1
        #[allow(clippy::arithmetic_side_effects)]
        let c: u8 = (encoded_size - 1).try_into().unwrap();
        bytes[0] |= c << 6;

        dst.write_slice(&bytes[..encoded_size]);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self.0 {
            0x00..=0x3F => 1,
            0x40..=0x3FFF => 2,
            0x4000..=0x3FFFFF => 3,
            0x400000..=0x3FFFFFFF => 4,
            _ => unreachable!("BUG: value is out of range!"),
        }
    }
}

impl Decode<'_> for VarU32 {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Ensure we have at least 1 byte available to determine the size of the value
        ensure_size!(in: src, size: 1);

        let header = src.read_u8();
        let c: usize = ((header >> 6) & 0x03).into();

        if c == 0 {
            return Ok(VarU32((header & 0x3F).into()));
        }

        ensure_size!(in: src, size: c);
        let bytes = src.read_slice(c);

        let val1 = header & 0x3F;

        // LINTS: c is always [1..4]
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = c * 8;
        let mut num = u32::from(val1) << shift;

        // Read val2..valN
        // LINTS: shift is always 8, 16, 24
        #[allow(clippy::arithmetic_side_effects)]
        for val in bytes.iter().take(c) {
            shift -= 8;
            num |= (u32::from(*val)) << shift;
        }

        Ok(VarU32(num))
    }
}

impl From<VarU32> for u32 {
    fn from(value: VarU32) -> Self {
        value.value()
    }
}

impl TryFrom<u32> for VarU32 {
    type Error = EncodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Variable-length encoded i32.
/// Value range: `[-0x1FFFFFFF..0x1FFFFFFF]`
///
/// NOW-PROTO: NOW_VARI32
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarI32(i32);

impl VarI32 {
    pub const MIN: i32 = -0x1FFFFFFF;
    pub const MAX: i32 = 0x1FFFFFFF;

    const NAME: &'static str = "NOW_VARI32";

    pub fn new(value: i32) -> DecodeResult<Self> {
        if value.abs() > Self::MAX {
            return Err(invalid_field_err!("value", "too large number"));
        }

        Ok(VarI32(value))
    }

    pub fn value(&self) -> i32 {
        self.0
    }
}

impl Encode for VarI32 {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();

        ensure_size!(in: dst, size: encoded_size);

        // LINTS: encoded_size will always be [1..4], therefore following arithmetic is safe
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = (encoded_size - 1) * 8;
        let mut bytes = [0u8; 4];

        let abs_value = self.0.unsigned_abs();

        for byte in bytes.iter_mut().take(encoded_size) {
            *byte = ((abs_value >> shift) & 0xFF).try_into().unwrap();

            // LINTS: as per code above, shift is always 8, 16, 24
            #[allow(clippy::arithmetic_side_effects)]
            if shift != 0 {
                shift -= 8;
            }
        }

        // LINTS: encoded_size is always >= 1
        #[allow(clippy::arithmetic_side_effects)]
        let c: u8 = (encoded_size - 1).try_into().unwrap();
        bytes[0] |= c << 6;
        if self.0 < 0 {
            // set sign bit
            bytes[0] |= 0x20;
        }

        dst.write_slice(&bytes[..encoded_size]);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self.0.unsigned_abs() {
            0..=0x1F => 1,
            0x20..=0x1FFF => 2,
            0x2000..=0x1FFFFF => 3,
            0x200000..=0x1FFFFFFF => 4,
            _ => unreachable!("BUG: value is out of range!"),
        }
    }
}

impl Decode<'_> for VarI32 {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Ensure we have at least 1 byte available to determine the size of the value
        ensure_size!(in: src, size: 1);

        let header = src.read_u8();
        let c: usize = ((header >> 6) & 0x03).into();
        let is_negative = (header & 0x20) != 0;

        if c == 0 {
            let val = i32::from(header & 0x1F);
            // LINTS: Variable integer range is always smaller than underlying type range,
            // therefore negation is always safe
            #[allow(clippy::arithmetic_side_effects)]
            return Ok(VarI32(if is_negative { -val } else { val }));
        }

        ensure_size!(in: src, size: c);
        let bytes = src.read_slice(c);

        let val1 = header & 0x1F;

        // LINTS: c is always [1..4]
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = c * 8;
        let mut num = i32::from(val1) << shift;

        // Read val2..valN
        // LINTS: shift is always 8, 16, 24
        #[allow(clippy::arithmetic_side_effects)]
        for val in bytes.iter().take(c) {
            shift -= 8;
            num |= (i32::from(*val)) << shift;
        }

        // LINTS: Variable integer range is always smaller than underlying type range,
        // therefore negation is always safe
        #[allow(clippy::arithmetic_side_effects)]
        Ok(VarI32(if is_negative { -num } else { num }))
    }
}

impl From<VarI32> for i32 {
    fn from(value: VarI32) -> Self {
        value.value()
    }
}

impl TryFrom<i32> for VarI32 {
    type Error = DecodeError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Variable-length encoded u64.
/// Value range: `[0..0x1FFFFFFFFFFFFFFF]`
///
/// NOW-PROTO: NOW_VARU64
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarU64(u64);

impl VarU64 {
    pub const MIX: u64 = 0x0000000000000000;
    pub const MAX: u64 = 0x1FFFFFFFFFFFFFFF;

    const NAME: &'static str = "NOW_VARU64";

    pub fn new(value: u64) -> DecodeResult<Self> {
        if value > Self::MAX {
            return Err(invalid_field_err!("value", "too large number"));
        }

        Ok(VarU64(value))
    }

    pub fn value(&self) -> u64 {
        self.0
    }
}

impl Encode for VarU64 {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();

        ensure_size!(in: dst, size: encoded_size);

        // LINTS: encoded_size will always be [1..8], therefore following arithmetic is safe
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = (encoded_size - 1) * 8;
        let mut bytes = [0u8; 8];

        for byte in bytes.iter_mut().take(encoded_size) {
            *byte = ((self.0 >> shift) & 0xFF).try_into().unwrap();

            // LINTS: as per code above, shift is always >= 8
            #[allow(clippy::arithmetic_side_effects)]
            if shift != 0 {
                shift -= 8;
            }
        }

        // LINTS: encoded_size is always >= 1
        #[allow(clippy::arithmetic_side_effects)]
        let c: u8 = (encoded_size - 1).try_into().unwrap();
        bytes[0] |= c << 5;

        dst.write_slice(&bytes[..encoded_size]);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self.0 {
            0x00..=0x1F => 1,
            0x20..=0x1FFF => 2,
            0x2000..=0x1FFFFF => 3,
            0x200000..=0x1FFFFFFF => 4,
            0x20000000..=0x1FFFFFFFFF => 5,
            0x2000000000..=0x1FFFFFFFFFFF => 6,
            0x200000000000..=0x1FFFFFFFFFFFFF => 7,
            0x20000000000000..=0x1FFFFFFFFFFFFFFF => 8,
            _ => unreachable!("BUG: value is out of range!"),
        }
    }
}

impl Decode<'_> for VarU64 {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Ensure we have at least 1 byte available to determine the size of the value
        ensure_size!(in: src, size: 1);

        let header = src.read_u8();
        let c: usize = ((header >> 5) & 0x07).into();

        if c == 0 {
            return Ok(VarU64((header & 0x1F).into()));
        }

        ensure_size!(in: src, size: c);
        let bytes = src.read_slice(c);

        let val1 = header & 0x1F;
        // LINTS: c is always [1..8]
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = c * 8;
        let mut num = u64::from(val1) << shift;

        // Read val2..valN
        // LINTS: shift is always >= 8
        #[allow(clippy::arithmetic_side_effects)]
        for val in bytes.iter().take(c) {
            shift -= 8;
            num |= (u64::from(*val)) << shift;
        }

        Ok(VarU64(num))
    }
}

impl From<VarU64> for u64 {
    fn from(value: VarU64) -> Self {
        value.value()
    }
}

impl TryFrom<u64> for VarU64 {
    type Error = DecodeError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Variable-length encoded i64.
/// Value range: `[-0x0FFFFFFFFFFFFFFF..0x0FFFFFFFFFFFFFFF]`
///
/// NOW-PROTO: NOW_VARI64
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VarI64(i64);

impl VarI64 {
    const NAME: &'static str = "NOW_VARI64";
    const MAX: i64 = 0x0FFFFFFFFFFFFFFF;

    pub fn new(value: i64) -> DecodeResult<Self> {
        if value.abs() > Self::MAX {
            return Err(invalid_field_err!("value", "too large number"));
        }

        Ok(VarI64(value))
    }

    pub fn value(&self) -> i64 {
        self.0
    }
}

impl Encode for VarI64 {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let encoded_size = self.size();

        ensure_size!(in: dst, size: encoded_size);

        // LINTS: encoded_size will always be [1..8], therefore following arithmetic is safe
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = (encoded_size - 1) * 8;
        let mut bytes = [0u8; 8];

        let abs_value = self.0.unsigned_abs();

        for byte in bytes.iter_mut().take(encoded_size) {
            *byte = ((abs_value >> shift) & 0xFF).try_into().unwrap();

            // LINTS: as per code above, shift is always >= 8
            #[allow(clippy::arithmetic_side_effects)]
            if shift != 0 {
                shift -= 8;
            }
        }

        // LINTS: encoded_size is always >= 1
        #[allow(clippy::arithmetic_side_effects)]
        let c: u8 = (encoded_size - 1).try_into().unwrap();
        bytes[0] |= c << 5;
        if self.0 < 0 {
            // set sign bit
            bytes[0] |= 0x10;
        }

        dst.write_slice(&bytes[..encoded_size]);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self.0.unsigned_abs() {
            0..=0x0F => 1,
            0x10..=0x0FFF => 2,
            0x1000..=0x0FFFFF => 3,
            0x100000..=0x0FFFFFFF => 4,
            0x10000000..=0x0FFFFFFFFF => 5,
            0x1000000000..=0x0FFFFFFFFFFF => 6,
            0x100000000000..=0x0FFFFFFFFFFFFF => 7,
            0x10000000000000..=0x0FFFFFFFFFFFFFFF => 8,
            _ => unreachable!("BUG: value is out of range!"),
        }
    }
}

impl Decode<'_> for VarI64 {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Ensure we have at least 1 byte available to determine the size of the value
        ensure_size!(in: src, size: 1);

        let header = src.read_u8();
        let c: usize = ((header >> 5) & 0x07).into();
        let is_negative = (header & 0x10) != 0;

        if c == 0 {
            let val = i64::from(header & 0x0F);
            // LINTS: Variable integer range is always smaller than underlying type range,
            // therefore negation is always safe
            #[allow(clippy::arithmetic_side_effects)]
            return Ok(VarI64(if is_negative { -val } else { val }));
        }

        ensure_size!(in: src, size: c);
        let bytes = src.read_slice(c);

        let val1 = header & 0x0F;
        // LINTS: c is always [1..8]
        #[allow(clippy::arithmetic_side_effects)]
        let mut shift = c * 8;
        let mut num = i64::from(val1) << shift;

        // Read val2..valN
        // LINTS: shift is always >= 8
        #[allow(clippy::arithmetic_side_effects)]
        for val in bytes.iter().take(c) {
            shift -= 8;
            num |= (i64::from(*val)) << shift;
        }

        // LINTS: Variable integer range is always smaller than underlying type range,
        // therefore negation is always safe
        #[allow(clippy::arithmetic_side_effects)]
        Ok(VarI64(if is_negative { -num } else { num }))
    }
}

impl From<VarI64> for i64 {
    fn from(value: VarI64) -> Self {
        value.value()
    }
}

impl TryFrom<i64> for VarI64 {
    type Error = DecodeError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
