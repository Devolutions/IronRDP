use alloc::format;

use ironrdp_core::{
    ensure_size, unsupported_value_err, Decode, DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor,
    WriteCursor,
};
use ironrdp_pdu::utils::strict_sum;

use crate::pdu::common::{HResult, SharedMsgHeader};

/// Identifies the interface manipulation capabilties of server/client.
#[repr(u32)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum Capability {
    #[doc(alias = "RIM_CAPABILITY_VERSION_01")]
    RimCapabilityVersion01 = 0x00000001,
}

impl TryFrom<u32> for Capability {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        use Capability::RimCapabilityVersion01 as V1;

        if value == 0x1 {
            Ok(V1)
        } else {
            #[expect(clippy::as_conversions)]
            Err(unsupported_value_err!(
                "Capability",
                format!("is not RIM_CAPABILITY_VERSION_01 ({})", V1 as u32)
            ))
        }
    }
}

#[doc(alias = "RIM_EXCHANGE_CAPABILITY_REQUEST")]
pub struct RimExchangeCapabilityRequest {
    pub header: SharedMsgHeader,
    pub capability: Capability,
}

impl Encode for RimExchangeCapabilityRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;

        #[expect(clippy::as_conversions)]
        dst.write_u32(self.capability as u32);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "RIM_EXCHANGE_CAPABILITY_REQUEST"
    }

    fn size(&self) -> usize {
        strict_sum(&[self.header.size() + size_of::<Capability>()])
    }
}

impl Decode<'_> for RimExchangeCapabilityRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;

        ensure_size!(in: src, size: size_of::<Capability>());
        let caps = src.read_u32();
        let capability = Capability::try_from(caps)?;

        Ok(Self { header, capability })
    }
}

#[doc(alias = "RIM_EXCHANGE_CAPABILITY_RESPONSE")]
pub struct RimExchangeCapabilityResponse {
    pub header: SharedMsgHeader,
    pub capability: Capability,
    pub result: HResult,
}

impl Encode for RimExchangeCapabilityResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

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
        strict_sum(&[self.header.size() + size_of_val(&self.capability) + size_of_val(&self.result)])
    }
}

impl Decode<'_> for RimExchangeCapabilityResponse {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;

        ensure_size!(in: src, size: size_of::<Capability>() + size_of::<HResult>());

        let caps = src.read_u32();
        let capability = Capability::try_from(caps)?;

        let result = src.read_u32();

        Ok(Self {
            header,
            capability,
            result,
        })
    }
}
