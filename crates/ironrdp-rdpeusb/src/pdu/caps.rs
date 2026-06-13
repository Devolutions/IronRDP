//! Messages specific to the [Exchange Capabilities][1] interface.
//!
//! Used to exchange the client's and the server's capabilities for interface manipulation.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6aee4e70-9d3b-49d7-a9b9-3c437cb27c8e

use ironrdp_core::{
    DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

use crate::pdu::header::{FunctionId, InterfaceId, Mask, MessageId, SharedMsgHeader};
use crate::pdu::utils::HResult;

/// Identifies the interface manipulation capabilities of server/client.
#[repr(u32)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Capability {
    #[doc(alias = "RIM_CAPABILITY_VERSION_01")]
    RimCapabilityVersion01 = 0x1,
}

impl Capability {
    pub const FIXED_PART_SIZE: usize = size_of::<u32>();
}

/// [\[MS-RDPEUSB\] 2.2.3.1 Interface Manipulation Exchange Capabilities Request
/// (RIM_EXCHANGE_CAPABILITY_REQUEST)][1] packet.
///
/// Used by the server to request interface manipulation capabilities from the client.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/13494979-ccdf-4c7c-99f0-f56e05cb259e
#[doc(alias = "RIM_EXCHANGE_CAPABILITY_REQUEST")]
#[derive(Debug, PartialEq)]
pub struct RimExchangeCapabilityRequest {
    pub msg_id: MessageId,
    pub capability: Capability,
}

impl RimExchangeCapabilityRequest {
    const PAYLOAD_SIZE: usize = Capability::FIXED_PART_SIZE;

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_REQ;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: InterfaceId::CAPABILITIES.with_mask(Mask::None),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::RIM_EXCHANGE_CAPABILITY_REQUEST),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        if src.read_u32() != 1 {
            return Err(invalid_field_err!(
                "RIM_EXCHANGE_CAPABILITY_REQUEST::CapabilityValue",
                "is not 0x1 (RIM_CAPABILITY_VERSION_01)"
            ));
        }
        Ok(Self {
            msg_id: header.msg_id,
            capability: Capability::RimCapabilityVersion01,
        })
    }
}

impl Encode for RimExchangeCapabilityRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        self.header().encode(dst)?;

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

/// [\[MS-RDPEUSB\] 2.2.3.2 Interface Manipulation Exchange Capabilities Response
/// (RIM_EXCHANGE_CAPABILITY_RESPONSE)][1] packet.
///
/// Sent by the client in response to [`RimExchangeCapabilityRequest`]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/668b8ab2-7a78-4d94-bc78-04645b404cc7
#[doc(alias = "RIM_EXCHANGE_CAPABILITY_RESPONSE")]
#[derive(Debug, PartialEq)]
pub struct RimExchangeCapabilityResponse {
    pub msg_id: MessageId,
    pub capability: Capability,
    pub result: HResult,
}

impl RimExchangeCapabilityResponse {
    const PAYLOAD_SIZE: usize = Capability::FIXED_PART_SIZE + size_of::<HResult>();

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_RSP;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: InterfaceId::CAPABILITIES.with_mask(Mask::None),
            msg_id: self.msg_id,
            function_id: None,
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        if src.read_u32() != 1 {
            return Err(invalid_field_err!(
                "RIM_EXCHANGE_CAPABILITY_RESPONSE::CapabilityValue",
                "is not 0x1 (RIM_CAPABILITY_VERSION_01)"
            ));
        };
        let result = src.read_u32();

        Ok(Self {
            msg_id: header.msg_id,
            capability: Capability::RimCapabilityVersion01,
            result,
        })
    }
}

impl Encode for RimExchangeCapabilityResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        self.header().encode(dst)?;

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

impl DvcEncode for RimExchangeCapabilityRequest {}
impl DvcEncode for RimExchangeCapabilityResponse {}
