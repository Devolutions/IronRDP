use ironrdp_core::{
    DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size,
    unsupported_value_err,
};

use crate::ensure_payload_size;
use crate::pdu::header::SharedMsgHeader;
use crate::pdu::utils::HResult;

/// Identifies the interface manipulation capabilties of server/client.
#[repr(u32)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum Capability {
    #[doc(alias = "RIM_CAPABILITY_VERSION_01")]
    RimCapabilityVersion01 = 0x1,
}

impl Capability {
    pub const FIXED_PART_SIZE: usize = size_of::<Self>();
}

impl TryFrom<u32> for Capability {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value == 0x1 {
            Ok(Self::RimCapabilityVersion01)
        } else {
            Err(unsupported_value_err!(
                "CapabilityValue",
                "is not: `RIM_CAPABILITY_VERSION_01 = 0x1`".into()
            ))
        }
    }
}

#[doc(alias = "RIM_EXCHANGE_CAPABILITY_REQUEST")]
pub struct RimExchangeCapabilityRequest {
    pub header: SharedMsgHeader,
    pub capability: Capability,
}

impl RimExchangeCapabilityRequest {
    pub const PAYLOAD_SIZE: usize = Capability::FIXED_PART_SIZE;

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_NOT_RSP;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);
        let capability = Capability::try_from(src.read_u32())?;

        Ok(Self { header, capability })
    }
}

impl Encode for RimExchangeCapabilityRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        self.header.encode(dst)?;

        #[expect(clippy::as_conversions)]
        dst.write_u32(self.capability as u32);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "RIM_EXCHANGE_CAPABILITY_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

#[doc(alias = "RIM_EXCHANGE_CAPABILITY_RESPONSE")]
pub struct RimExchangeCapabilityResponse {
    pub header: SharedMsgHeader,
    pub capability: Capability,
    pub result: HResult,
}

impl RimExchangeCapabilityResponse {
    pub const PAYLOAD_SIZE: usize = Capability::FIXED_PART_SIZE + size_of::<HResult>();

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_RSP;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);
        let capability = Capability::try_from(src.read_u32())?;

        let result = src.read_u32();

        Ok(Self {
            header,
            capability,
            result,
        })
    }
}

impl Encode for RimExchangeCapabilityResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header.encode(dst)?;

        #[expect(clippy::as_conversions)]
        dst.write_u32(self.capability as u32);

        dst.write_u32(self.result);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "RIM_EXCHANGE_CAPABILITY_RESPONSE"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}
