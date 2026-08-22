//! RDP_TUNNEL_SUBHEADER: variable-length sub-header embedded in Data PDU headers.
//!
//! MS-RDPEMT Section 2.2.1.1.1.
//!
//! Wire layout:
//! ```text
//! Byte 0:  SubHeaderLength (u8, minimum 2, counting this byte and SubHeaderType)
//! Byte 1:  SubHeaderType (u8)
//! Byte 2+: SubHeaderData (SubHeaderLength - 2 bytes)
//! ```
//!
//! The inner SubHeaderData is passed through as opaque bytes. Auto-detect
//! request/response structures (MS-RDPBCGR Section 2.2.14) are not parsed
//! by this crate.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, InvalidFieldErr as _, ReadCursor, UnexpectedMessageTypeErr as _,
    WriteCursor,
};

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Sub-header type discriminator.
///
/// MS-RDPEMT Section 2.2.1.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SubHeaderType {
    /// Bandwidth measurement request from server.
    AutoDetectRequest = 0x00,
    /// Bandwidth measurement response from client.
    AutoDetectResponse = 0x01,
}

impl SubHeaderType {
    /// Parse from wire byte.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::AutoDetectRequest),
            0x01 => Some(Self::AutoDetectResponse),
            _ => None,
        }
    }

    /// Wire representation.
    #[expect(clippy::as_conversions, reason = "repr(u8) enum to u8 is safe")]
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// A parsed RDP_TUNNEL_SUBHEADER.
///
/// The `data` field contains the raw SubHeaderData bytes (opaque to this crate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelSubHeader {
    /// High-level type identifier.
    pub sub_header_type: SubHeaderType,
    /// Opaque payload (SubHeaderLength - 2 bytes).
    pub data: Vec<u8>,
}

impl TunnelSubHeader {
    /// Minimum SubHeaderLength: 1 (length byte) + 1 (type byte) = 2.
    const MIN_WIRE_SIZE: usize = 2;

    const NAME: &'static str = "RDP_TUNNEL_SUBHEADER";

    /// Total encoded size on the wire.
    pub fn wire_size(&self) -> usize {
        Self::MIN_WIRE_SIZE + self.data.len()
    }
}

impl Encode for TunnelSubHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let total = self.wire_size();
        ironrdp_core::ensure_size!(in: dst, size: total);

        // SubHeaderLength includes itself and SubHeaderType. A truncating
        // cast here would silently wrap once `data` pushes `total` past 255,
        // writing a length byte that disagrees with the bytes that follow.
        let sub_header_length = u8::try_from(total).map_err(|_| {
            ironrdp_core::invalid_field_err!(Self::NAME, "SubHeaderLength", "sub-header exceeds 255 bytes")
        })?;
        dst.write_u8(sub_header_length);
        dst.write_u8(self.sub_header_type.to_u8());
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.wire_size()
    }
}

impl Decode<'_> for TunnelSubHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_size!(in: src, size: Self::MIN_WIRE_SIZE);

        let sub_header_length = src.read_u8();

        if sub_header_length < 2 {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "SubHeaderLength",
                "sub-header length must be at least 2",
            ));
        }

        let type_raw = src.read_u8();
        let sub_header_type = SubHeaderType::from_u8(type_raw)
            .ok_or_else(|| ironrdp_core::DecodeError::unexpected_message_type(Self::NAME, type_raw))?;

        let data_len = usize::from(sub_header_length) - Self::MIN_WIRE_SIZE;
        ironrdp_core::ensure_size!(in: src, size: data_len);
        let data = src.read_slice(data_len).to_vec();

        Ok(Self { sub_header_type, data })
    }
}
