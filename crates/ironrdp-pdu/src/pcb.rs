//! This module contains the RDP_PRECONNECTION_PDU_V1 and RDP_PRECONNECTION_PDU_V2 structures.

use ironrdp_core::{
    Decode, DecodeOwned as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length,
    ensure_fixed_part_size, ensure_size, invalid_field_err, invalid_field_err_with_source, read_padding, write_padding,
};
pub use ironrdp_str::prefixed::CchString;

use crate::Pdu;

/// Preconnection PDU version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcbVersion(pub u32);

impl PcbVersion {
    pub const V1: Self = Self(0x1);
    pub const V2: Self = Self(0x2);
}

/// RDP preconnection PDU
///
/// The RDP_PRECONNECTION_PDU_V1 is used by the client to let the listening process
/// know which RDP source the connection is intended for.
///
/// The RDP_PRECONNECTION_PDU_V2 extends the RDP_PRECONNECTION_PDU_V1 packet by
/// adding a variable-size Unicode character string. The receiver of this PDU can
/// use this string and the Id field of the RDP_PRECONNECTION_PDU_V1 packet to
/// determine the RDP source. This string is opaque to the protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreconnectionBlob {
    /// Preconnection PDU version
    pub version: PcbVersion,
    /// This field is used to uniquely identify the RDP source. Although the Id can be
    /// as simple as a process ID, it is often client-specific or server-specific and
    /// can be obfuscated.
    pub id: u32,
    /// V2 PCB string ([MS-RDPEPS] §2.2.1.2 `cchPCB`/`wszPCB`)
    pub v2_payload: Option<CchString>,
}

impl PreconnectionBlob {
    pub const FIXED_PART_SIZE: usize = 16;
}

impl Pdu for PreconnectionBlob {
    const NAME: &'static str = "PreconnectionBlob";
}

impl<'de> Decode<'de> for PreconnectionBlob {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pcb_size: usize = cast_length!("cbSize", src.read_u32())?;

        if pcb_size < Self::FIXED_PART_SIZE {
            return Err(invalid_field_err(
                Self::NAME,
                "cbSize",
                "advertised size too small for Preconnection PDU V1",
            ));
        }

        read_padding!(src, 4); // flags

        // The version field SHOULD be initialized by the client and SHOULD be ignored by the server,
        // as specified in sections 3.1.5.1 and 3.2.5.1.
        // That’s why, following code doesn’t depend on the value of this field.
        let version = PcbVersion(src.read_u32());

        let id = src.read_u32();

        let remaining_size = pcb_size - Self::FIXED_PART_SIZE;

        ensure_size!(in: src, size: remaining_size);

        if remaining_size >= 2 {
            // Peek at cchPCB (without consuming) to validate against the outer PDU size before
            // delegating to the typed decode. This produces a specific error distinct from the
            // generic "not enough bytes" that decode_owned would otherwise return.
            let cch_pcb = usize::from(src.peek_u16());
            let cb_pcb = cch_pcb * 2;

            if remaining_size - 2 < cb_pcb {
                return Err(invalid_field_err(
                    Self::NAME,
                    "cchPCB",
                    "PCB string bigger than advertised size",
                ));
            }

            // decode_owned reads the cchPCB prefix, the string content, and the null terminator.
            let v2_payload = CchString::decode_owned(src)
                .map_err(|e| invalid_field_err_with_source(Self::NAME, "wszPCB", "malformed PCB string", e))?;

            let leftover_size = remaining_size - 2 - cb_pcb;
            src.advance(leftover_size); // Consume (unused) leftover data

            Ok(Self {
                version,
                id,
                v2_payload: Some(v2_payload),
            })
        } else {
            Ok(Self {
                version,
                id,
                v2_payload: None,
            })
        }
    }
}

impl Encode for PreconnectionBlob {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.v2_payload.is_some() && self.version == PcbVersion::V1 {
            return Err(invalid_field_err(
                Self::NAME,
                "version",
                "there is no string payload in Preconnection PDU V1",
            ));
        }

        let pcb_size = self.size();

        ensure_size!(in: dst, size: pcb_size);

        dst.write_u32(cast_length!("cbSize", pcb_size)?); // cbSize
        write_padding!(dst, 4); // flags
        dst.write_u32(self.version.0); // version
        dst.write_u32(self.id); // id

        if let Some(v2_payload) = &self.v2_payload {
            // Writes cchPCB prefix (u16, including null), followed by wszPCB and null terminator.
            v2_payload.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let fixed_part_size = Self::FIXED_PART_SIZE;

        let variable_part = self.v2_payload.as_ref().map_or(0, |p| p.size());

        fixed_part_size + variable_part
    }
}
