//! This module contains the RDP_PRECONNECTION_PDU_V1 and RDP_PRECONNECTION_PDU_V2 structures.

use crate::{cursor::ReadCursor, padding::Padding, Error, Pdu, PduDecode, PduEncode, Result};

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
    /// V2 PCB string
    pub v2_payload: Option<String>,
}

impl PreconnectionBlob {
    pub const FIXED_PART_SIZE: usize = 16;
}

impl Pdu for PreconnectionBlob {
    const NAME: &'static str = "PreconnectionBlob";
}

impl<'de> PduDecode<'de> for PreconnectionBlob {
    fn decode(src: &mut ReadCursor<'de>) -> Result<Self> {
        ensure_fixed_part_size!(in: src);

        let pcb_size: usize = cast_length!(src.read_u32(), "cbSize")?;

        if pcb_size < Self::FIXED_PART_SIZE {
            return Err(Error::InvalidMessage {
                name: Self::NAME,
                field: "cbSize",
                reason: "advertised size too small for Preconnection PDU V1",
            });
        }

        Padding::<4>::read(src); // flags

        // The version field SHOULD be initialized by the client and SHOULD be ignored by the server,
        // as specified in sections 3.1.5.1 and 3.2.5.1.
        // That’s why, following code doesn’t depend on the value of this field.
        let version = PcbVersion(src.read_u32());

        let id = src.read_u32();

        let remaining_size = pcb_size - Self::FIXED_PART_SIZE;

        ensure_size!(in: src, size: remaining_size);

        if remaining_size >= 2 {
            let cch_pcb = usize::from(src.read_u16());
            let cb_pcb = cch_pcb * 2;

            if remaining_size - 2 < cb_pcb {
                return Err(Error::InvalidMessage {
                    name: Self::NAME,
                    field: "cchPCB",
                    reason: "PCB string bigger than advertised size",
                });
            }

            let wsz_pcb_utf16 = src.read_slice(cb_pcb);

            let mut trimmed_pcb_utf16: Vec<u16> = Vec::with_capacity(cch_pcb);

            for chunk in wsz_pcb_utf16.chunks_exact(2) {
                let code_unit = u16::from_le_bytes([chunk[0], chunk[1]]);

                // Stop reading at the null terminator
                if code_unit == 0 {
                    break;
                }

                trimmed_pcb_utf16.push(code_unit);
            }

            let payload = String::from_utf16(&trimmed_pcb_utf16).map_err(|_| Error::InvalidMessage {
                name: Self::NAME,
                field: "wszPCB",
                reason: "invalid UTF-16",
            })?;

            let leftover_size = remaining_size - 2 - cb_pcb;
            src.advance(leftover_size); // Consume (unused) leftover data

            Ok(Self {
                version,
                id,
                v2_payload: Some(payload),
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

impl PduEncode for PreconnectionBlob {
    fn encode(&self, dst: &mut crate::cursor::WriteCursor<'_>) -> Result<()> {
        if self.v2_payload.is_some() && self.version == PcbVersion::V1 {
            return Err(Error::InvalidMessage {
                name: Self::NAME,
                field: "version",
                reason: "there is no string payload in Preconnection PDU V1",
            });
        }

        let pcb_size = self.size();

        ensure_size!(in: dst, size: pcb_size);

        dst.write_u32(cast_length!(pcb_size, "cbSize")?); // cbSize
        Padding::<4>::write(dst); // flags
        dst.write_u32(self.version.0); // version
        dst.write_u32(self.id); // id

        if let Some(v2_payload) = &self.v2_payload {
            // cchPCB
            let utf16_character_count = v2_payload.encode_utf16().count() + 1; // +1 for null terminator
            dst.write_u16(cast_length!(utf16_character_count, "cchPCB")?);

            // wszPCB
            v2_payload.encode_utf16().for_each(|c| dst.write_u16(c));
            dst.write_u16(0); // null terminator
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let fixed_part_size = Self::FIXED_PART_SIZE;

        let variable_part = if let Some(v2_payload) = &self.v2_payload {
            let utf16_character_count = v2_payload.encode_utf16().count() + 1; // +1 for null terminator
            2 + utf16_character_count * 2
        } else {
            0
        };

        fixed_part_size + variable_part
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRECONNECTION_PDU_V1_NULL_SIZE_BUF: [u8; 16] = [
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x00 = 0 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x01, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 1
        0xeb, 0x99, 0xc6, 0xee, // -> RDP_PRECONNECTION_PDU_V1::Id = 0xEEC699EB = 4005992939
    ];
    const PRECONNECTION_PDU_V1_LARGE_SIZE_BUF: [u8; 16] = [
        0xff, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0xff
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x01, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 1
        0xeb, 0x99, 0xc6, 0xee, // -> RDP_PRECONNECTION_PDU_V1::Id = 0xEEC699EB = 4005992939
    ];
    const PRECONNECTION_PDU_V1_BUF: [u8; 16] = [
        0x10, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x10 = 16 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x01, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 1
        0xeb, 0x99, 0xc6, 0xee, // -> RDP_PRECONNECTION_PDU_V1::Id = 0xEEC699EB = 4005992939
    ];
    const PRECONNECTION_PDU_V2_LARGE_PAYLOAD_SIZE_BUF: [u8; 32] = [
        0x20, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x20 = 32 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x02, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 2
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Id = 0
        0xff, 0x00, //       -> RDP_PRECONNECTION_PDU_V2::cchPCB = 0xff
        0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x56, 0x00, 0x4d, 0x00, 0x00,
        0x00, // -> RDP_PRECONNECTION_PDU_V2::wszPCB -> "TestVM\0"
    ];
    const PRECONNECTION_PDU_V2_BUF: [u8; 32] = [
        0x20, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x20 = 32 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x02, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 2
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Id = 0
        0x07, 0x00, //       -> RDP_PRECONNECTION_PDU_V2::cchPCB = 0x7 = 7 characters
        0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x56, 0x00, 0x4d, 0x00, 0x00,
        0x00, // -> RDP_PRECONNECTION_PDU_V2::wszPCB -> "TestVM\0"
    ];

    const PRECONNECTION_PDU_V1: PreconnectionBlob = PreconnectionBlob {
        version: PcbVersion::V1,
        id: 4_005_992_939,
        v2_payload: None,
    };

    lazy_static::lazy_static! {
        static ref PRECONNECTION_PDU_V2: PreconnectionBlob = PreconnectionBlob {
            version: PcbVersion::V2,
            id: 0,
            v2_payload: Some(String::from("TestVM")),
        };
    }

    #[test]
    fn null_size() {
        let e = crate::decode::<PreconnectionBlob>(&PRECONNECTION_PDU_V1_NULL_SIZE_BUF)
            .err()
            .unwrap();

        if let Error::InvalidMessage { field, reason, .. } = e {
            assert_eq!(field, "cbSize");
            assert_eq!(reason, "advertised size too small for Preconnection PDU V1");
        } else {
            panic!("unexpected error: {e}");
        }
    }

    #[test]
    fn truncated() {
        let e = crate::decode::<PreconnectionBlob>(&PRECONNECTION_PDU_V1_LARGE_SIZE_BUF)
            .err()
            .unwrap();

        if let Error::NotEnoughBytes { received, expected, .. } = e {
            assert_eq!(received, 0);
            assert_eq!(expected, 239);
        } else {
            panic!("unexpected error: {e}");
        }
    }

    #[test]
    fn v1_decode() {
        let pcb = crate::decode::<PreconnectionBlob>(&PRECONNECTION_PDU_V1_BUF).unwrap();
        assert_eq!(pcb, PRECONNECTION_PDU_V1);
    }

    #[test]
    fn v1_encode() {
        let mut buf = Vec::new();
        crate::encode_buf(&PRECONNECTION_PDU_V1, &mut buf).unwrap();
        assert_eq!(buf, PRECONNECTION_PDU_V1_BUF);
    }

    #[test]
    fn v1_size() {
        assert_eq!(PRECONNECTION_PDU_V1.size(), PRECONNECTION_PDU_V1_BUF.len());
    }

    #[test]
    fn v2_string_too_big() {
        let e = crate::decode::<PreconnectionBlob>(&PRECONNECTION_PDU_V2_LARGE_PAYLOAD_SIZE_BUF)
            .err()
            .unwrap();

        if let Error::InvalidMessage { field, reason, .. } = e {
            assert_eq!(field, "cchPCB");
            assert_eq!(reason, "PCB string bigger than advertised size");
        } else {
            panic!("unexpected error: {e}");
        }
    }

    #[test]
    fn v2_decode() {
        let pcb = crate::decode::<PreconnectionBlob>(&PRECONNECTION_PDU_V2_BUF).unwrap();
        assert_eq!(pcb, *PRECONNECTION_PDU_V2);
    }

    #[test]
    fn v2_encode() {
        let mut buf = Vec::new();
        crate::encode_buf(&*PRECONNECTION_PDU_V2, &mut buf).unwrap();
        assert_eq!(buf, PRECONNECTION_PDU_V2_BUF);
    }

    #[test]
    fn v2_size() {
        assert_eq!(PRECONNECTION_PDU_V2.size(), PRECONNECTION_PDU_V2_BUF.len());
    }
}
