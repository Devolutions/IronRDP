//! Common header used by all [MS-RDPEUSB][1] messages.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

use alloc::format;

use ironrdp_core::{
    Decode, DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size,
    unsupported_value_err,
};

#[cfg(doc)]
use crate::pdu::caps::{RimExchangeCapabilityRequest, RimExchangeCapabilityResponse};

/// Unique ID for a "top-level" request-response pair.
pub type MessageId = u32;

/// Indicates in what context is a [`SharedMsgHeader`] being used.
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mask {
    /// Indicates that the [`SharedMsgHeader`] is being used in a response message.
    #[doc(alias = "STREAM_ID_STUB")]
    StreamIdStub = 0x2,

    /// Indicates that the [`SharedMsgHeader`] is not being used in a response message.
    #[doc(alias = "STREAM_ID_PROXY")]
    StreamIdProxy = 0x1,

    /// Indicates that the [`SharedMsgHeader`] is being used in a message for capabilities exchange
    /// ([`RimExchangeCapabilityRequest`], [`RimExchangeCapabilityResponse`]). This value **MUST
    /// NOT** be used for any other messages.
    #[doc(alias = "STREAM_ID_NONE")]
    StreamIdNone = 0x0,
}

impl From<Mask> for u32 {
    #[expect(clippy::as_conversions)]
    fn from(value: Mask) -> Self {
        value as Self
    }
}

impl TryFrom<u8> for Mask {
    type Error = MaskErr;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(Self::StreamIdNone),
            0x1 => Ok(Self::StreamIdProxy),
            0x2 => Ok(Self::StreamIdStub),
            _ => Err(MaskErr(value)),
        }
    }
}

#[derive(Debug)]
pub struct MaskErr(u8);

impl core::fmt::Display for MaskErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "is: {:#X}, should be one of: 0x2 (STREAM_ID_STUB), 0x1 (STREAM_ID_PROXY), 0x0 (STREAM_ID_NONE)",
            self.0
        )
    }
}

impl core::error::Error for MaskErr {}

/// Groups similar kinds of messages together.
///
/// An interface is a "group" of similar kinds of messages. Some interfaces have default ID's
/// (see associated constants), while other interfaces like the **USB Device** and **Request Completion**
/// get allotted interface ID's during the lifecycle a USB redirection channel.
///
/// Goes without saying, server-client should maintain the interface ID's for the **Request
/// Completion** and **USB Devices** interfaces and match them with decoded interface ID's.
///
/// Max value for interface ID's: `0x3F_FF_FF_FF` (30 bits).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceId(pub(in crate::pdu) u32);

impl InterfaceId {
    pub const FIXED_PART_SIZE: usize = size_of::<Self>();

    /// **Exchange Capabilities** interface (ID: `0x0`). Used by both client and server to
    /// exchange capabilities for interface manipulation.
    ///
    /// * [MS-RDPEUSB § 2.2.3 Interface Manipulation Exchange Capabilities Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6aee4e70-9d3b-49d7-a9b9-3c437cb27c8e
    pub const CAPABILITIES: Self = Self(0x0);

    /// **Device Sink** interface (ID: `0x1`). Used by the client to communicate with the server
    /// about new USB devices.
    ///
    /// * [MS-RDPEUSB § 2.2.4 Device Sink Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a9a8add7-4e99-4697-abd0-ad64c80c788d
    pub const DEVICE_SINK: Self = Self(0x1);

    /// **Channel Notification** interface (ID: `0x2`). Used by the server to communicate with the
    /// client.
    ///
    /// * [MS-RDPEUSB § 2.2.5 Channel Notification Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62
    pub const NOTIFY_CLIENT: Self = Self(0x2);

    /// **Channel Notification** interface (ID: `0x3`). Used by the client to communicate with the
    /// server.
    ///
    /// * [MS-RDPEUSB § 2.2.5 Channel Notification Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62
    pub const NOTIFY_SERVER: Self = Self(0x3);
}

impl TryFrom<u32> for InterfaceId {
    type Error = InterfaceIdErr;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value <= 0x3F_FF_FF_FF {
            Ok(InterfaceId(value))
        } else {
            Err(InterfaceIdErr(value))
        }
    }
}

impl From<InterfaceId> for u32 {
    fn from(value: InterfaceId) -> Self {
        value.0
    }
}

impl core::fmt::Display for InterfaceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct InterfaceIdErr(u32);

impl core::fmt::Display for InterfaceIdErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "InterfaceId greater than 30 bits: {}", self.0)
    }
}

impl core::error::Error for InterfaceIdErr {}

/// Indicates a task/function to perform.
///
/// Function ID's are defined for all interfaces:
///
/// * Interface Manipulation Exchange Capabilities Interface
///   * [`FunctionId::RIM_EXCHANGE_CAPABILITY_REQUEST`]
/// * Device Sink Interface
///   * [`FunctionId::ADD_VIRTUAL_CHANNEL`]
///   * [`FunctionId::ADD_DEVICE`]
/// * Channel Notification Interface
///   * [`FunctionId::CHANNEL_CREATED`]
/// * USB Device Interface
///   * [`FunctionId::CANCEL_REQUEST`]
///   * [`FunctionId::REGISTER_REQUEST_CALLBACK`]
///   * [`FunctionId::IO_CONTROL`]
///   * [`FunctionId::INTERNAL_IO_CONTROL`]
///   * [`FunctionId::QUERY_DEVICE_TEXT`]
///   * [`FunctionId::TRANSFER_IN_REQUEST`]
///   * [`FunctionId::TRANSFER_OUT_REQUEST`]
///   * [`FunctionId::RETRACT_DEVICE`]
/// * Request Completion Interface
///   * [`FunctionId::IOCONTROL_COMPLETION`]
///   * [`FunctionId::URB_COMPLETION`]
///   * [`FunctionId::URB_COMPLETION_NO_DATA`]
///
/// See [`InterfaceId`] for more info on interfaces.
///
/// * [MS-RDPEUSB § 2.2.1 Shared Message Header (SHARED_MSG_HEADER)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FunctionId(pub(in crate::pdu) u32);

impl FunctionId {
    pub const FIXED_PART_SIZE: usize = size_of::<Self>();
    // // Needed for QI_REQ and QI_RSP
    //
    // /// Release the given interface ID.
    // pub const RIMCALL_RELEASE: Self = Self(0x00000001);
    // pub const RIMCALL_QUERYINTERFACE: Self = Self(0x00000002);

    // -------------------- Exchange Capabilities Interface ---------------------------------------

    /// The server sends the [`RIM_EXCHANGE_CAPABILITY_REQUEST`][1] message.
    ///
    /// [1]: crate::pdu::caps::RimExchangeCapabilityRequest
    pub const RIM_EXCHANGE_CAPABILITY_REQUEST: Self = Self(0x100);

    // -------------------- Request Completion Interface ------------------------------------------

    pub const IOCONTROL_COMPLETION: Self = Self(0x100);
    pub const URB_COMPLETION: Self = Self(0x101);
    pub const URB_COMPLETION_NO_DATA: Self = Self(0x102);

    // -------------------- USB Device Interface --------------------------------------------------

    pub const CANCEL_REQUEST: Self = Self(0x100);
    pub const REGISTER_REQUEST_CALLBACK: Self = Self(0x101);
    pub const IO_CONTROL: Self = Self(0x102);
    pub const INTERNAL_IO_CONTROL: Self = Self(0x103);
    pub const QUERY_DEVICE_TEXT: Self = Self(0x104);
    pub const TRANSFER_IN_REQUEST: Self = Self(0x105);
    pub const TRANSFER_OUT_REQUEST: Self = Self(0x106);
    pub const RETRACT_DEVICE: Self = Self(0x107);

    // -------------------- Device Sink Interface -------------------------------------------------

    /// The client sends the [`ADD_VIRTUAL_CHANNEL`][1] message.
    ///
    /// [1]: super::device_sink::AddVirtualChannel
    pub const ADD_VIRTUAL_CHANNEL: Self = Self(0x100);
    pub const ADD_DEVICE: Self = Self(0x101);

    // -------------------- Channel Notification Interface ----------------------------------------

    pub const CHANNEL_CREATED: Self = Self(0x100);
}

impl TryFrom<u32> for FunctionId {
    type Error = FunctionIdErr;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        // if matches!(value, 0x001 | 0x002 | 0x100..=0x107) {
        if matches!(value, 0x100..=0x107) {
            Ok(Self(value))
        } else {
            Err(FunctionIdErr::NotInRange(value))
        }
    }
}

impl core::fmt::Display for FunctionId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#X}", self.0)
    }
}

#[derive(Debug)]
pub enum FunctionIdErr {
    NotInRange(u32),
    InvalidForInterface(InterfaceId, FunctionId),
    Missing,
    NotAbsent,
}

impl core::fmt::Display for FunctionIdErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FunctionIdErr::NotInRange(value) => {
                write!(
                    f,
                    "is: {value:#X}, should be one of: [0x100, 0x101, 0x102, 0x103, 0x104, 0x105, 0x106, 0x107]"
                )
            }
            FunctionIdErr::InvalidForInterface(i, value) => {
                write!(f, "FunctionId {:#X} is invalid for the interface {i}", value.0)
            }
            FunctionIdErr::Missing => write!(f, "FunctionId is absent when it should be present"),
            FunctionIdErr::NotAbsent => write!(f, "FunctionId is present when it should be absent"),
        }
    }
}

impl core::error::Error for FunctionIdErr {}

/// [\[MS-RDPEUSB\] 2.2.1 Shared Message Header (SHARED_MSG_HEADER)][1].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[doc(alias = "SHARED_MSG_HEADER")]
#[derive(Debug, PartialEq, Clone)]
pub struct SharedMsgHeader {
    pub interface_id: InterfaceId,
    pub mask: Mask,
    pub msg_id: MessageId,
    pub function_id: Option<FunctionId>,
}

impl SharedMsgHeader {
    pub const SIZE_RSP: usize = size_of::<u32>(/* InterfaceId, Mask */) + size_of::<MessageId>();

    pub const SIZE_REQ: usize = Self::SIZE_RSP + FunctionId::FIXED_PART_SIZE;
}

impl Encode for SharedMsgHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let first32 = u32::from(self.interface_id) | (u32::from(self.mask) << 30);
        dst.write_u32(first32);
        dst.write_u32(self.msg_id);

        if let Some(id) = self.function_id {
            dst.write_u32(id.0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "SHARED_MSG_HEADER"
    }

    fn size(&self) -> usize {
        if self.function_id.is_some() {
            Self::SIZE_REQ
        } else {
            Self::SIZE_RSP
        }
    }
}

impl Decode<'_> for SharedMsgHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: const { size_of::<u32>(/* InterfaceId, Mask */) + size_of::<MessageId>()} );

        let first32 = src.read_u32();
        let interface_id = InterfaceId::try_from(first32 & 0x3F_FF_FF_FF).expect("value clamped");
        #[expect(clippy::as_conversions)]
        let mask = Mask::try_from((first32 >> 30) as u8)
            .map_err(|source| unsupported_value_err!("Mask", format!("{}", source.0)))?;

        let msg_id = src.read_u32();

        let function_id = match mask {
            Mask::StreamIdStub => None,
            Mask::StreamIdProxy => {
                ensure_size!(in: src, size: FunctionId::FIXED_PART_SIZE);
                let id = FunctionId::try_from(src.read_u32()).map_err(|source| {
                    let value = match &source {
                        FunctionIdErr::NotInRange(value) => value,
                        _ => unreachable!("FunctionId::try_from only returns NotInRange error"),
                    };
                    let e: DecodeError = unsupported_value_err!("FunctionId", format!("{value}"));
                    e.with_source(source)
                })?;
                Some(id)
            }
            Mask::StreamIdNone => {
                ensure_size!(in: src, size: FunctionId::FIXED_PART_SIZE);
                // either 0x100 (FunctionId) or 0x001 (CapabilityValue)
                (src.peek_u32() == FunctionId::RIM_EXCHANGE_CAPABILITY_REQUEST.0).then(|| FunctionId(src.read_u32()))
            }
        };

        Ok(SharedMsgHeader {
            interface_id,
            mask,
            msg_id,
            function_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn req() {
        let mut wire = Vec::from([0; SharedMsgHeader::SIZE_REQ]);
        let mut dst = WriteCursor::new(&mut wire);
        let header_en = SharedMsgHeader {
            interface_id: InterfaceId(234),
            mask: Mask::StreamIdProxy,
            msg_id: 6767,
            function_id: Some(FunctionId::RIM_EXCHANGE_CAPABILITY_REQUEST),
        };
        header_en.encode(&mut dst).unwrap();
        let mut src = ReadCursor::new(&wire);
        let header_de = SharedMsgHeader::decode(&mut src).unwrap();
        assert_eq!(header_en, header_de);
    }

    #[test]
    fn rsp() {
        let mut wire = Vec::from([0; SharedMsgHeader::SIZE_RSP]);
        let mut dst = WriteCursor::new(&mut wire);
        let header_en = SharedMsgHeader {
            interface_id: InterfaceId(234),
            mask: Mask::StreamIdStub,
            msg_id: 6767,
            function_id: None,
        };
        header_en.encode(&mut dst).unwrap();
        let mut src = ReadCursor::new(&wire);
        let header_de = SharedMsgHeader::decode(&mut src).unwrap();
        assert_eq!(header_en, header_de);
    }
}
