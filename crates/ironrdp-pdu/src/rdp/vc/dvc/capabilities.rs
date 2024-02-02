#[cfg(test)]
mod tests;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{Header, PduType, HEADER_SIZE, UNUSED_U8};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

const DVC_CAPABILITIES_PAD_SIZE: usize = 1;
const DVC_CAPABILITIES_VERSION_SIZE: usize = 2;
const DVC_CAPABILITIES_CHARGE_SIZE: usize = 2;
const DVC_CAPABILITIES_CHARGE_COUNT: usize = 4;

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum CapsVersion {
    V1 = 0x0001,
    V2 = 0x0002,
    V3 = 0x0003,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitiesRequestPdu {
    V1,
    V2 {
        charges: [u16; DVC_CAPABILITIES_CHARGE_COUNT],
    },
    V3 {
        charges: [u16; DVC_CAPABILITIES_CHARGE_COUNT],
    },
}

impl CapabilitiesRequestPdu {
    const NAME: &'static str = "CapabilitiesRequestPdu";

    const FIXED_PART_SIZE: usize = HEADER_SIZE + DVC_CAPABILITIES_PAD_SIZE + DVC_CAPABILITIES_VERSION_SIZE;
}

impl PduEncode for CapabilitiesRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let dvc_header = Header {
            channel_id_type: UNUSED_U8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Capabilities,
        };
        dvc_header.encode(dst)?;
        dst.write_u8(UNUSED_U8);

        match self {
            CapabilitiesRequestPdu::V1 => dst.write_u16(CapsVersion::V1.to_u16().unwrap()),
            CapabilitiesRequestPdu::V2 { charges } => {
                dst.write_u16(CapsVersion::V2.to_u16().unwrap());
                for charge in charges.iter() {
                    dst.write_u16(*charge);
                }
            }
            CapabilitiesRequestPdu::V3 { charges } => {
                dst.write_u16(CapsVersion::V3.to_u16().unwrap());
                for charge in charges.iter() {
                    dst.write_u16(*charge);
                }
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let charges_length = match self {
            CapabilitiesRequestPdu::V1 => 0,
            CapabilitiesRequestPdu::V2 { charges } | CapabilitiesRequestPdu::V3 { charges } => {
                charges.len() * DVC_CAPABILITIES_CHARGE_SIZE
            }
        };

        Self::FIXED_PART_SIZE + charges_length
    }
}

impl<'de> PduDecode<'de> for CapabilitiesRequestPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE - HEADER_SIZE);

        let _pad = src.read_u8();
        let version = CapsVersion::from_u16(src.read_u16())
            .ok_or_else(|| invalid_message_err!("DvcCapabilities", "invalid version"))?;

        match version {
            CapsVersion::V1 => Ok(Self::V1),
            CapsVersion::V2 => {
                let mut charges = [0; DVC_CAPABILITIES_CHARGE_COUNT];
                for c in charges.iter_mut() {
                    *c = src.read_u16();
                }
                Ok(Self::V2 { charges })
            }
            CapsVersion::V3 => {
                let mut charges = [0; DVC_CAPABILITIES_CHARGE_COUNT];
                for c in charges.iter_mut() {
                    *c = src.read_u16();
                }
                Ok(Self::V3 { charges })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesResponsePdu {
    pub version: CapsVersion,
}

impl CapabilitiesResponsePdu {
    const NAME: &'static str = "CapabilitiesResponsePdu";

    const FIXED_PART_SIZE: usize = HEADER_SIZE + DVC_CAPABILITIES_PAD_SIZE + DVC_CAPABILITIES_VERSION_SIZE;
}

impl PduEncode for CapabilitiesResponsePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let dvc_header = Header {
            channel_id_type: UNUSED_U8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Capabilities,
        };
        dvc_header.encode(dst)?;
        dst.write_u8(UNUSED_U8);
        dst.write_u16(self.version.to_u16().unwrap());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for CapabilitiesResponsePdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE - HEADER_SIZE);

        let _pad = src.read_u8();
        let version = CapsVersion::from_u16(src.read_u16())
            .ok_or_else(|| invalid_message_err!("DvcCapabilities", "invalid version"))?;

        Ok(Self { version })
    }
}
