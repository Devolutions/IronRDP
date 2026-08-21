//! Variable-length integer encodings from [MS-RDPEI] 2.2.2.
//!
//! [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/1aab43bf-cab8-4f0a-9eb3-b83b8365e237

// Varint packing is defined in terms of byte splits after explicit range checks.
#![allow(clippy::as_conversions)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};

const TWO_BYTE_UNSIGNED_MAX: u16 = 0x7FFF;
const TWO_BYTE_SIGNED_MAX: i16 = 0x3FFF;
const FOUR_BYTE_UNSIGNED_MAX: u32 = 0x3FFF_FFFF;
const FOUR_BYTE_SIGNED_MAX: i32 = 0x1FFF_FFFF;
const EIGHT_BYTE_UNSIGNED_MAX: u64 = 0x1FFF_FFFF_FFFF_FFFF;

/// TWO_BYTE_UNSIGNED_INTEGER — range `0x0000`..=`0x7FFF`.
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/c8d5dc69-e4d4-4d72-a1be-d3a062a3a7d0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoByteUnsigned(u16);

impl TwoByteUnsigned {
    pub const MAX: u16 = TWO_BYTE_UNSIGNED_MAX;

    pub fn new(value: u16) -> DecodeResult<Self> {
        if value > Self::MAX {
            return Err(invalid_field_err!("TWO_BYTE_UNSIGNED_INTEGER", "value out of range"));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u16 {
        self.0
    }

    pub fn encoded_size(value: u16) -> usize {
        if value <= 0x7F { 1 } else { 2 }
    }
}

impl Encode for TwoByteUnsigned {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = Self::encoded_size(self.0);
        ensure_size!(in: dst, size: size);
        if size == 1 {
            dst.write_u8(self.0 as u8);
        } else {
            let val1 = ((self.0 >> 8) & 0x7F) as u8;
            let val2 = (self.0 & 0xFF) as u8;
            dst.write_u8(0x80 | val1);
            dst.write_u8(val2);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TWO_BYTE_UNSIGNED_INTEGER"
    }

    fn size(&self) -> usize {
        Self::encoded_size(self.0)
    }
}

impl<'de> Decode<'de> for TwoByteUnsigned {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        if first & 0x80 == 0 {
            Ok(Self(u16::from(first)))
        } else {
            ensure_size!(in: src, size: 1);
            let second = src.read_u8();
            let value = (u16::from(first & 0x7F) << 8) | u16::from(second);
            Ok(Self(value))
        }
    }
}

/// TWO_BYTE_SIGNED_INTEGER — range `-0x3FFF`..=`0x3FFF`.
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/1aab43bf-344a-467a-b9d4-dfe196b46c9d
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoByteSigned(i16);

impl TwoByteSigned {
    pub const MAX: i16 = TWO_BYTE_SIGNED_MAX;
    pub const MIN: i16 = -TWO_BYTE_SIGNED_MAX;

    pub fn new(value: i16) -> DecodeResult<Self> {
        if !(-Self::MAX..=Self::MAX).contains(&value) {
            return Err(invalid_field_err!("TWO_BYTE_SIGNED_INTEGER", "value out of range"));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i16 {
        self.0
    }

    pub fn encoded_size(value: i16) -> usize {
        let magnitude = value.unsigned_abs();
        if magnitude <= 0x3F { 1 } else { 2 }
    }
}

impl Encode for TwoByteSigned {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = Self::encoded_size(self.0);
        ensure_size!(in: dst, size: size);
        let negative = self.0 < 0;
        let magnitude = self.0.unsigned_abs();
        if size == 1 {
            let mut first = (magnitude & 0x3F) as u8;
            if negative {
                first |= 0x40;
            }
            dst.write_u8(first);
        } else {
            let mut first = 0x80 | (((magnitude >> 8) & 0x3F) as u8);
            if negative {
                first |= 0x40;
            }
            dst.write_u8(first);
            dst.write_u8((magnitude & 0xFF) as u8);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TWO_BYTE_SIGNED_INTEGER"
    }

    fn size(&self) -> usize {
        Self::encoded_size(self.0)
    }
}

impl<'de> Decode<'de> for TwoByteSigned {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        let negative = first & 0x40 != 0;
        let two_bytes = first & 0x80 != 0;
        let magnitude = if two_bytes {
            ensure_size!(in: src, size: 1);
            let second = src.read_u8();
            (u16::from(first & 0x3F) << 8) | u16::from(second)
        } else {
            u16::from(first & 0x3F)
        };
        if magnitude > TWO_BYTE_SIGNED_MAX as u16 {
            return Err(invalid_field_err!("TWO_BYTE_SIGNED_INTEGER", "value out of range"));
        }
        let value = if negative {
            -(magnitude as i16)
        } else {
            magnitude as i16
        };
        Ok(Self(value))
    }
}

/// FOUR_BYTE_UNSIGNED_INTEGER — range `0x00000000`..=`0x3FFFFFFF`.
///
/// [2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/c8d5dc69-e4d4-4d72-a1be-d3a062a3a7d0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourByteUnsigned(u32);

impl FourByteUnsigned {
    pub const MAX: u32 = FOUR_BYTE_UNSIGNED_MAX;

    pub fn new(value: u32) -> DecodeResult<Self> {
        if value > Self::MAX {
            return Err(invalid_field_err!("FOUR_BYTE_UNSIGNED_INTEGER", "value out of range"));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u32 {
        self.0
    }

    pub fn encoded_size(value: u32) -> usize {
        if value <= 0x3F {
            1
        } else if value <= 0x3FFF {
            2
        } else if value <= 0x3F_FFFF {
            3
        } else {
            4
        }
    }
}

impl Encode for FourByteUnsigned {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = Self::encoded_size(self.0);
        ensure_size!(in: dst, size: size);
        let c = (size - 1) as u8;
        match size {
            1 => dst.write_u8(self.0 as u8),
            2 => {
                dst.write_u8((c << 6) | (((self.0 >> 8) & 0x3F) as u8));
                dst.write_u8((self.0 & 0xFF) as u8);
            }
            3 => {
                dst.write_u8((c << 6) | (((self.0 >> 16) & 0x3F) as u8));
                dst.write_u8(((self.0 >> 8) & 0xFF) as u8);
                dst.write_u8((self.0 & 0xFF) as u8);
            }
            _ => {
                dst.write_u8((c << 6) | (((self.0 >> 24) & 0x3F) as u8));
                dst.write_u8(((self.0 >> 16) & 0xFF) as u8);
                dst.write_u8(((self.0 >> 8) & 0xFF) as u8);
                dst.write_u8((self.0 & 0xFF) as u8);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "FOUR_BYTE_UNSIGNED_INTEGER"
    }

    fn size(&self) -> usize {
        Self::encoded_size(self.0)
    }
}

impl<'de> Decode<'de> for FourByteUnsigned {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        let c = (first >> 6) as usize;
        let val1 = u32::from(first & 0x3F);
        let extra = c;
        ensure_size!(in: src, size: extra);
        let mut value = val1;
        for _ in 0..extra {
            value = (value << 8) | u32::from(src.read_u8());
        }
        Ok(Self(value))
    }
}

/// FOUR_BYTE_SIGNED_INTEGER — range `-0x1FFFFFFF`..=`0x1FFFFFFF`.
///
/// [2.2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourByteSigned(i32);

impl FourByteSigned {
    pub const MAX: i32 = FOUR_BYTE_SIGNED_MAX;
    pub const MIN: i32 = -FOUR_BYTE_SIGNED_MAX;

    pub fn new(value: i32) -> DecodeResult<Self> {
        if !(-Self::MAX..=Self::MAX).contains(&value) {
            return Err(invalid_field_err!("FOUR_BYTE_SIGNED_INTEGER", "value out of range"));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i32 {
        self.0
    }

    pub fn encoded_size(value: i32) -> usize {
        let magnitude = value.unsigned_abs();
        if magnitude <= 0x1F {
            1
        } else if magnitude <= 0x1FFF {
            2
        } else if magnitude <= 0x1F_FFFF {
            3
        } else {
            4
        }
    }
}

impl Encode for FourByteSigned {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = Self::encoded_size(self.0);
        ensure_size!(in: dst, size: size);
        let negative = self.0 < 0;
        let magnitude = self.0.unsigned_abs();
        let c = (size - 1) as u8;
        let sign_bit: u8 = if negative { 0x20 } else { 0 };
        match size {
            1 => dst.write_u8(sign_bit | ((magnitude & 0x1F) as u8)),
            2 => {
                dst.write_u8((c << 6) | sign_bit | (((magnitude >> 8) & 0x1F) as u8));
                dst.write_u8((magnitude & 0xFF) as u8);
            }
            3 => {
                dst.write_u8((c << 6) | sign_bit | (((magnitude >> 16) & 0x1F) as u8));
                dst.write_u8(((magnitude >> 8) & 0xFF) as u8);
                dst.write_u8((magnitude & 0xFF) as u8);
            }
            _ => {
                dst.write_u8((c << 6) | sign_bit | (((magnitude >> 24) & 0x1F) as u8));
                dst.write_u8(((magnitude >> 16) & 0xFF) as u8);
                dst.write_u8(((magnitude >> 8) & 0xFF) as u8);
                dst.write_u8((magnitude & 0xFF) as u8);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "FOUR_BYTE_SIGNED_INTEGER"
    }

    fn size(&self) -> usize {
        Self::encoded_size(self.0)
    }
}

impl<'de> Decode<'de> for FourByteSigned {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        let c = (first >> 6) as usize;
        let negative = first & 0x20 != 0;
        let val1 = u32::from(first & 0x1F);
        let extra = c;
        ensure_size!(in: src, size: extra);
        let mut magnitude = val1;
        for _ in 0..extra {
            magnitude = (magnitude << 8) | u32::from(src.read_u8());
        }
        if magnitude > FOUR_BYTE_SIGNED_MAX as u32 {
            return Err(invalid_field_err!("FOUR_BYTE_SIGNED_INTEGER", "value out of range"));
        }
        let value = if negative {
            -(magnitude as i32)
        } else {
            magnitude as i32
        };
        Ok(Self(value))
    }
}

/// EIGHT_BYTE_UNSIGNED_INTEGER — range `0`..=`0x1FFFFFFFFFFFFFFF`.
///
/// [2.2.2.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/1aab43bf-cab8-4f0a-9eb3-b83b8365e237
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EightByteUnsigned(u64);

impl EightByteUnsigned {
    pub const MAX: u64 = EIGHT_BYTE_UNSIGNED_MAX;

    pub fn new(value: u64) -> DecodeResult<Self> {
        if value > Self::MAX {
            return Err(invalid_field_err!("EIGHT_BYTE_UNSIGNED_INTEGER", "value out of range"));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn encoded_size(value: u64) -> usize {
        if value <= 0x1F {
            1
        } else if value <= 0x1FFF {
            2
        } else if value <= 0x1F_FFFF {
            3
        } else if value <= 0x1FFF_FFFF {
            4
        } else if value <= 0x1F_FFFF_FFFF {
            5
        } else if value <= 0x1FFF_FFFF_FFFF {
            6
        } else if value <= 0x1F_FFFF_FFFF_FFFF {
            7
        } else {
            8
        }
    }
}

impl Encode for EightByteUnsigned {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = Self::encoded_size(self.0);
        ensure_size!(in: dst, size: size);
        let c = (size - 1) as u8;
        let shift = (size - 1) * 8;
        let first_val = if shift == 0 {
            self.0 & 0x1F
        } else {
            (self.0 >> shift) & 0x1F
        };
        dst.write_u8((c << 5) | (first_val as u8));
        let mut remaining_shift = shift;
        while remaining_shift > 0 {
            remaining_shift -= 8;
            dst.write_u8(((self.0 >> remaining_shift) & 0xFF) as u8);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "EIGHT_BYTE_UNSIGNED_INTEGER"
    }

    fn size(&self) -> usize {
        Self::encoded_size(self.0)
    }
}

impl<'de> Decode<'de> for EightByteUnsigned {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        let c = (first >> 5) as usize;
        let val1 = u64::from(first & 0x1F);
        let extra = c;
        ensure_size!(in: src, size: extra);
        let mut value = val1;
        for _ in 0..extra {
            value = (value << 8) | u64::from(src.read_u8());
        }
        if value > Self::MAX {
            return Err(invalid_field_err!("EIGHT_BYTE_UNSIGNED_INTEGER", "value out of range"));
        }
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::*;

    #[test]
    fn two_byte_unsigned_spec_example() {
        let value = TwoByteUnsigned::new(0x1A1B).unwrap();
        assert_eq!(encode_vec(&value).unwrap(), vec![0x9A, 0x1B]);
        assert_eq!(decode::<TwoByteUnsigned>(&[0x9A, 0x1B]).unwrap().get(), 0x1A1B);
    }

    #[test]
    fn two_byte_signed_spec_examples() {
        let neg_large = TwoByteSigned::new(-0x1A1B).unwrap();
        assert_eq!(encode_vec(&neg_large).unwrap(), vec![0xDA, 0x1B]);
        assert_eq!(decode::<TwoByteSigned>(&[0xDA, 0x1B]).unwrap().get(), -0x1A1B);

        let neg_small = TwoByteSigned::new(-2).unwrap();
        assert_eq!(encode_vec(&neg_small).unwrap(), vec![0x42]);
        assert_eq!(decode::<TwoByteSigned>(&[0x42]).unwrap().get(), -2);
    }

    #[test]
    fn four_byte_unsigned_spec_example() {
        let value = FourByteUnsigned::new(0x001A_1B1C).unwrap();
        assert_eq!(encode_vec(&value).unwrap(), vec![0x9A, 0x1B, 0x1C]);
        assert_eq!(
            decode::<FourByteUnsigned>(&[0x9A, 0x1B, 0x1C]).unwrap().get(),
            0x001A_1B1C
        );
    }

    #[test]
    fn four_byte_signed_spec_examples() {
        let neg_large = FourByteSigned::new(-0x001A_1B1C).unwrap();
        assert_eq!(encode_vec(&neg_large).unwrap(), vec![0xBA, 0x1B, 0x1C]);
        assert_eq!(
            decode::<FourByteSigned>(&[0xBA, 0x1B, 0x1C]).unwrap().get(),
            -0x001A_1B1C
        );

        let neg_small = FourByteSigned::new(-2).unwrap();
        assert_eq!(encode_vec(&neg_small).unwrap(), vec![0x22]);
        assert_eq!(decode::<FourByteSigned>(&[0x22]).unwrap().get(), -2);
    }

    #[test]
    fn eight_byte_unsigned_spec_example() {
        let value = EightByteUnsigned::new(0x001A_1B1C_1D1E_1F2A).unwrap();
        assert_eq!(
            encode_vec(&value).unwrap(),
            vec![0xDA, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x2A]
        );
        assert_eq!(
            decode::<EightByteUnsigned>(&[0xDA, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x2A])
                .unwrap()
                .get(),
            0x001A_1B1C_1D1E_1F2A
        );
    }

    #[test]
    fn round_trip_small_values() {
        for v in [0u16, 1, 0x7F, 0x80, 0x7FFF] {
            let encoded = encode_vec(&TwoByteUnsigned::new(v).unwrap()).unwrap();
            assert_eq!(decode::<TwoByteUnsigned>(&encoded).unwrap().get(), v);
        }
        for v in [
            0i32,
            1,
            -1,
            0x1F,
            -0x1F,
            0x20,
            -0x20,
            FOUR_BYTE_SIGNED_MAX,
            -FOUR_BYTE_SIGNED_MAX,
        ] {
            let encoded = encode_vec(&FourByteSigned::new(v).unwrap()).unwrap();
            assert_eq!(decode::<FourByteSigned>(&encoded).unwrap().get(), v);
        }
    }
}
