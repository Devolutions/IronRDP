use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, invalid_field_err,
};

/// [2.2.1.1.2] Window List Capability Set.
///
/// [2.2.1.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp
#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct WindowList {
    pub support_level: WindowSupportLevel,
    pub num_icon_caches: u8,
    pub num_icon_cache_entries: u16,
}

impl WindowList {
    const NAME: &'static str = "WindowList";

    pub(crate) const FIXED_PART_SIZE: usize =
        4 /* WndSupportLevel */ + 1 /* NumIconCaches */ + 2 /* NumIconCacheEntries */;
}

impl Encode for WindowList {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.support_level.as_u32());
        dst.write_u8(self.num_icon_caches);
        dst.write_u16(self.num_icon_cache_entries);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for WindowList {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let support_level = WindowSupportLevel::from_u32(src.read_u32())
            .ok_or_else(|| invalid_field_err!("wndSupportLevel", "invalid window support level"))?;
        let num_icon_caches = src.read_u8();
        let num_icon_cache_entries = src.read_u16();

        Ok(Self {
            support_level,
            num_icon_caches,
            num_icon_cache_entries,
        })
    }
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum WindowSupportLevel {
    NotSupported = 0,
    Supported = 1,
    SupportedEx = 2,
}

impl WindowSupportLevel {
    fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::NotSupported),
            1 => Some(Self::Supported),
            2 => Some(Self::SupportedEx),
            _ => None,
        }
    }

    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u32(self) -> u32 {
        self as u32
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::super::CapabilitySet;
    use super::*;

    const WINDOW_LIST_BUFFER: [u8; 7] = [0x02, 0x00, 0x00, 0x00, 0x03, 0x0c, 0x00];
    const WINDOW_LIST: WindowList = WindowList {
        support_level: WindowSupportLevel::SupportedEx,
        num_icon_caches: 3,
        num_icon_cache_entries: 12,
    };

    #[test]
    fn round_trips_window_list() {
        assert_eq!(WINDOW_LIST, decode(WINDOW_LIST_BUFFER.as_ref()).unwrap());
        assert_eq!(WINDOW_LIST_BUFFER, encode_vec(&WINDOW_LIST).unwrap().as_slice());
    }

    #[test]
    fn rejects_invalid_window_support_level() {
        assert!(decode::<WindowList>(&[3, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn rejects_invalid_window_list_capability_set_length() {
        assert!(decode::<CapabilitySet>(&[0x18, 0, 0x0c, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
