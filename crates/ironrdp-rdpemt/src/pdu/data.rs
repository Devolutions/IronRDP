//! RDP_TUNNEL_DATA: bidirectional higher-layer data transport.
//!
//! MS-RDPEMT Section 2.2.2.3.
//!
//! Wire layout (variable size):
//! ```text
//! Bytes 0-3:             TunnelHeader (Action=0x2, HeaderLength >= 0x04)
//! Bytes 4..HeaderLength: SubHeaders (if HeaderLength > 4)
//! Bytes HeaderLength..:  HigherLayerData (PayloadLength bytes)
//! ```

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, UnexpectedMessageTypeErr as _, WriteCursor,
};

use crate::pdu::header::{TunnelAction, TunnelHeader};
use crate::pdu::subheader::TunnelSubHeader;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Tunnel Data PDU.
///
/// Wraps higher-layer data (DVC traffic) with the tunnel header.
/// Optional SubHeaders may be present for auto-detect piggy-backing
/// (bandwidth measurement per MS-RDPBCGR Section 2.2.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelData {
    /// Optional sub-headers for auto-detect embedding.
    pub sub_headers: Vec<TunnelSubHeader>,
    /// Opaque higher-layer data (DVC payload).
    pub higher_layer_data: Vec<u8>,
}

impl TunnelData {
    const NAME: &'static str = "RDP_TUNNEL_DATA";

    /// Header bytes, untruncated.
    ///
    /// `HeaderLength` is a single byte on the wire, so this can exceed what the
    /// field holds. [`Self::header_length`] is the checked form that `encode`
    /// uses; this one exists so `size` can report the true length rather than a
    /// wrapped one.
    fn header_length_untruncated(&self) -> usize {
        TunnelHeader::MIN_SIZE + self.sub_headers.iter().map(TunnelSubHeader::wire_size).sum::<usize>()
    }

    /// Header bytes as they go on the wire, or `None` when the sub-headers do
    /// not fit the one-byte `HeaderLength` field.
    fn header_length(&self) -> Option<u8> {
        u8::try_from(self.header_length_untruncated()).ok()
    }
}

impl Encode for TunnelData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        // Both length fields are narrower than the values they describe, and a
        // truncated one produces a header that disagrees with the bytes behind
        // it. Refuse rather than emit that.
        let header_length = self.header_length().ok_or_else(|| {
            ironrdp_core::invalid_field_err!(Self::NAME, "HeaderLength", "sub-headers exceed one byte of length")
        })?;
        let payload_length = u16::try_from(self.higher_layer_data.len()).map_err(|_| {
            ironrdp_core::invalid_field_err!(Self::NAME, "PayloadLength", "payload exceeds 65535 bytes")
        })?;

        let header = TunnelHeader {
            action: TunnelAction::Data,
            payload_length,
            header_length,
            sub_headers: self.sub_headers.clone(),
        };
        header.encode(dst)?;

        dst.write_slice(&self.higher_layer_data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.header_length_untruncated() + self.higher_layer_data.len()
    }
}

impl Decode<'_> for TunnelData {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = TunnelHeader::decode(src)?;

        if header.action != TunnelAction::Data {
            return Err(ironrdp_core::DecodeError::unexpected_message_type(
                Self::NAME,
                header.action.to_u8(),
                Some(src.pos()),
            ));
        }

        let payload_len = usize::from(header.payload_length);
        ironrdp_core::ensure_size!(in: src, size: payload_len);
        let higher_layer_data = src.read_slice(payload_len).to_vec();

        Ok(Self {
            sub_headers: header.sub_headers,
            higher_layer_data,
        })
    }
}
