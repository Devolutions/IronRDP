//! RDP_TUNNEL_HEADER: common header for all RDPEMT PDUs.
//!
//! MS-RDPEMT Section 2.2.1.1.
//!
//! Wire layout (4+ bytes):
//! ```text
//! Byte 0:   [Action:4 bits (low nibble)] [Flags:4 bits (high nibble)]
//! Byte 1-2: PayloadLength (u16 LE)
//! Byte 3:   HeaderLength (u8)
//! Byte 4+:  SubHeaders (variable, present if HeaderLength > 4)
//! ```

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, InvalidFieldErr as _, ReadCursor, UnexpectedMessageTypeErr as _,
    WriteCursor,
};

use crate::pdu::subheader::TunnelSubHeader;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Tunnel PDU action type (low 4 bits of byte 0).
///
/// MS-RDPEMT Section 2.2.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TunnelAction {
    /// Client → server tunnel creation request.
    CreateRequest = 0x0,
    /// Server → client tunnel creation response.
    CreateResponse = 0x1,
    /// Bidirectional higher-layer data transfer.
    Data = 0x2,
}

impl TunnelAction {
    /// Attempt to parse an action value from the low nibble of byte 0.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x0 => Some(Self::CreateRequest),
            0x1 => Some(Self::CreateResponse),
            0x2 => Some(Self::Data),
            _ => None,
        }
    }

    /// Wire representation of this action (low nibble of byte 0).
    #[expect(clippy::as_conversions, reason = "repr(u8) enum to u8 is safe")]
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Parsed RDP_TUNNEL_HEADER.
///
/// The header is at least 4 bytes. When `header_length > 4`, the remaining
/// bytes between position 4 and `header_length` contain SubHeader structures
/// used for auto-detect embedding (MS-RDPBCGR Section 2.2.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelHeader {
    /// PDU type discriminator (low nibble of byte 0).
    pub action: TunnelAction,
    /// Bytes of payload following the header (does NOT include header size).
    pub payload_length: u16,
    /// Total header size as last decoded from the wire, including any
    /// SubHeaders. Minimum 4. `encode` does not trust this field: it derives
    /// the real value fresh from `sub_headers`, since a stored value that
    /// disagrees with `sub_headers` would encode a header the following
    /// bytes contradict. This field exists for inspecting a decoded header,
    /// not for controlling what `encode` writes.
    pub header_length: u8,
    /// Parsed sub-headers. Empty for CreateRequest/CreateResponse.
    pub sub_headers: Vec<TunnelSubHeader>,
}

impl TunnelHeader {
    /// Minimum header size: 1 (action+flags) + 2 (payload_length) + 1 (header_length).
    pub(crate) const MIN_SIZE: usize = 4;

    const NAME: &'static str = "RDP_TUNNEL_HEADER";

    /// The header length `encode` will actually write: `MIN_SIZE` plus the
    /// encoded size of every sub-header, checked to fit the one-byte field.
    fn derived_header_length(&self) -> EncodeResult<u8> {
        let total = Self::MIN_SIZE + self.sub_headers.iter().map(TunnelSubHeader::size).sum::<usize>();
        u8::try_from(total).map_err(|_| {
            ironrdp_core::invalid_field_err!(Self::NAME, "HeaderLength", "sub-headers exceed one byte of length")
        })
    }
}

impl Encode for TunnelHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        let header_length = self.derived_header_length()?;

        // Byte 0: (flags << 4) | action, flags are always 0
        dst.write_u8(self.action.to_u8());
        dst.write_u16(self.payload_length);
        dst.write_u8(header_length);

        for sub in &self.sub_headers {
            sub.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::MIN_SIZE + self.sub_headers.iter().map(TunnelSubHeader::size).sum::<usize>()
    }
}

impl Decode<'_> for TunnelHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_size!(in: src, size: Self::MIN_SIZE);

        let byte0 = src.read_u8();
        let action_raw = byte0 & 0x0F;
        let flags = byte0 >> 4;

        // MS-RDPEMT Section 2.2.1.1: Flags "MUST be set to zero"
        if flags != 0 {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "Flags",
                "flags must be zero",
            ));
        }

        let action = TunnelAction::from_u8(action_raw)
            .ok_or_else(|| ironrdp_core::DecodeError::unexpected_message_type(Self::NAME, action_raw))?;

        let payload_length = src.read_u16();
        let header_length = src.read_u8();

        if header_length < 4 {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "HeaderLength",
                "header length must be at least 4",
            ));
        }

        // Parse sub-headers from the remaining header bytes
        let sub_header_bytes = usize::from(header_length) - Self::MIN_SIZE;
        let mut sub_headers = Vec::new();

        if sub_header_bytes > 0 {
            ironrdp_core::ensure_size!(in: src, size: sub_header_bytes);
            let sub_data = src.read_slice(sub_header_bytes);
            let mut sub_cursor = ReadCursor::new(sub_data);

            while !sub_cursor.is_empty() {
                let sub = TunnelSubHeader::decode(&mut sub_cursor)?;
                sub_headers.push(sub);
            }
        }

        Ok(Self {
            action,
            payload_length,
            header_length,
            sub_headers,
        })
    }
}
