//! Common utils needed by PDU's under [MS-RDPEUSB][1].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

#![allow(dead_code)]

use alloc::borrow::ToOwned as _;
use alloc::string::ToString as _;

use ironrdp_core::{
    Decode, DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
    unsupported_value_err,
};

/// Unique ID for request-response pair.
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
pub type MessageId = u32;

/// Indicates in what context is a [`SHARED_MSG_HEADER`][1] being used.
///
/// Bits 30-31 of the mask occupy the corresponding bits 0-1 of the header (in big endian).
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][2]
///
/// [1]: SharedMsgHeader
/// [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mask {
    /// Indicates that the [`SHARED_MSG_HEADER`][1] is being used in a response message.
    ///
    /// [1]: SharedMsgHeader
    #[doc(alias = "STREAM_ID_STUB")]
    StreamIdStub = 0x2,

    /// Indicates that the [`SHARED_MSG_HEADER`][1] is not being used in a response message.
    ///
    /// [1]: SharedMsgHeader
    #[doc(alias = "STREAM_ID_PROXY")]
    StreamIdProxy = 0x1,

    /// Indicates that the [`SHARED_MSG_HEADER`][1] is being used in a message for capabilities
    /// exchange ([`RIM_EXCHANGE_CAPABILITY_REQUEST`][2] / [`RIM_EXCHANGE_CAPABILITY_RESPONSE`][3]).
    /// This value **MUST NOT** be used for any other messages.
    ///
    /// [1]: SharedMsgHeader
    /// [2]: super::exchange_caps::RimExchangeCapabilityRequest
    /// [3]: super::exchange_caps::RimExchangeCapabilityResponse
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
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(Self::StreamIdNone),
            0x1 => Ok(Self::StreamIdProxy),
            0x2 => Ok(Self::StreamIdStub),
            0x3 => Err(unsupported_value_err!("Mask", "is: 0x3".to_owned())),
            0x4.. => Err(invalid_field_err!("Mask", "more than 2 bits")),
        }
    }
}

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
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceId(u32);

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

impl From<u32> for InterfaceId {
    /// Constructs an `InterfaceId` from a value, discarding the highest 2 bits.
    fn from(value: u32) -> Self {
        Self(value & 0x3F_FF_FF_FF)
    }
}

impl From<InterfaceId> for u32 {
    fn from(value: InterfaceId) -> Self {
        value.0
    }
}

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
#[derive(Debug, Clone, Copy)]
pub struct FunctionId(u32);

impl FunctionId {
    pub const FIXED_PART_SIZE: usize = size_of::<Self>();
    // // Needed for QI_REQ and QI_RSP
    //
    // /// Release the given interface ID.
    // pub const RIMCALL_RELEASE: Self = Self(0x00000001);
    // pub const RIMCALL_QUERYINTERFACE: Self = Self(0x00000002);

    // -------------------- Exchange Capabilities Interface ---------------------------------------

    /// The server sends the [`RIM_EXCHANGE_CAPABILITY_REQUEST`][1] message, or the client sends
    /// the [`RIM_EXCHANGE_CAPABILITY_RESPONSE`][2] message in response to the former message.
    ///
    /// [1]: crate::pdu::caps::RimExchangeCapabilityRequest
    /// [2]: crate::pdu::caps::RimExchangeCapabilityResponse
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
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        // if matches!(value, 0x001 | 0x002 | 0x100..=0x107) {
        if matches!(value, 0x100..=0x107) {
            Ok(Self(value))
        } else {
            Err(unsupported_value_err!("FunctionId", value.to_string()))
        }
    }
}

/// Common header `SHARED_MSG_HEADER` for all messages under [MS-RDPEUSB][1].
///
/// ⚠️ Never use the [`size_of`][2] or [`size_of_val`][3] functions with the header,
/// use [`SharedMsgHeader::size`] instead.
///
/// * [MS-RDPEUSB § 2.2.1 Shared Message Header (SHARED_MSG_HEADER)][4]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125
/// [2]: core::mem::size_of
/// [3]: core::mem::size_of_val
/// [4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[doc(alias = "SHARED_MSG_HEADER")]
pub struct SharedMsgHeader {
    pub interface_id: InterfaceId,
    pub mask: Mask,
    pub message_id: MessageId,
    pub function_id: Option<FunctionId>,
}

impl SharedMsgHeader {
    pub const SIZE_WHEN_RSP: usize = InterfaceId::FIXED_PART_SIZE + size_of::<MessageId>();

    pub const SIZE_WHEN_NOT_RSP: usize = Self::SIZE_WHEN_RSP + FunctionId::FIXED_PART_SIZE;
}

impl Encode for SharedMsgHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let first32 = u32::from(self.interface_id) | (u32::from(self.mask) << 30);
        dst.write_u32(first32);
        dst.write_u32(self.message_id);

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
            Self::SIZE_WHEN_NOT_RSP
        } else {
            Self::SIZE_WHEN_RSP
        }
    }
}

impl Decode<'_> for SharedMsgHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: InterfaceId::FIXED_PART_SIZE + size_of::<MessageId>());

        let first32 = src.read_u32();
        let interface_id = InterfaceId::from(first32);
        #[expect(clippy::as_conversions)]
        let mask = Mask::try_from((first32 >> 30) as u8)?;
        let message_id = src.read_u32();

        let function_id = if mask == Mask::StreamIdStub {
            None
        } else {
            ensure_size!(in: src, size: FunctionId::FIXED_PART_SIZE);
            Some(FunctionId::try_from(src.read_u32())?)
        };

        Ok(SharedMsgHeader {
            interface_id,
            mask,
            message_id,
            function_id,
        })
    }
}
