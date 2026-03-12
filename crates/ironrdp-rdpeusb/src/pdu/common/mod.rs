//! Common utils needed by PDU's under [MS-RDPEUSB][1].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

#![allow(dead_code)]

use alloc::vec::Vec;
use alloc::{string::ToString as _, vec};
use ironrdp_core::{
    ensure_size, invalid_field_err, unsupported_value_err, Decode, DecodeError, DecodeResult, Encode, EncodeResult,
    ReadCursor, WriteCursor,
};

use ironrdp_pdu::utils::strict_sum;

pub mod ts_urb;

/// An integer value that indicates the result or status of an operation.
///
/// * [MS-ERREF § 2.1 HRESULT][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/0642cb2f-2075-4469-918c-4441e69c548a
pub type HResult = u32;

/// Unique ID for request-response pair.
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
pub type MessageId = u32;

/// Represents a request ID that uniquely identifies an `IO_CONTROL` or `INTERNAL_IO_CONTROL`
/// message.
pub type ReqIdIoctl = u32;

/// Represents a request ID that uniquely identifies a `TRANSFER_IN_REQUEST` or
/// `TRANSFER_OUT_REQUEST` message. 31 bits.
#[derive(Debug, Clone, Copy)]
pub struct RequestIdTsUrb(u32);

impl From<u32> for RequestIdTsUrb {
    /// Construct a request ID for `TRANSFER_IN_REQUEST` or `TRANSFER_OUT_REQUEST`. Discards
    /// highest bit.
    fn from(value: u32) -> Self {
        Self(value & 0x7F_FF_FF_FF)
    }
}

/// Indicates in what context is a [`SHARED_MSG_HEADER`][1] being used.
///
/// Bits 30-31 of the mask occupy the corresponding bits 0-1 of the header (in big endian).
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][2]
///
/// [1]: SharedMsgHeader
/// [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[repr(u32)]
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
    /// exchange ([`RIM_EXCHANGE_CAPABILITY_REQUEST`][2] or [`RIM_EXCHANGE_CAPABILITY_RESPONSE`][3]).
    /// This value MUST NOT be used for any other messages.
    ///
    /// [1]: SharedMsgHeader
    /// [2]: super::exchange_capabilities::RimExchangeCapabilityRequest
    /// [3]: super::exchange_capabilities::RimExchangeCapabilityResponse
    #[doc(alias = "STREAM_ID_NONE")]
    StreamIdNone = 0x0,
}

impl From<Mask> for u32 {
    #[expect(clippy::as_conversions)]
    fn from(value: Mask) -> Self {
        value as Self
    }
}

impl TryFrom<u32> for Mask {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match (value & 0xC0_00_00_00) >> 30 {
            0x0 => Ok(Self::StreamIdNone),
            0x1 => Ok(Self::StreamIdProxy),
            0x2 => Ok(Self::StreamIdStub),
            0x3 | 0x4.. => Err(invalid_field_err!("Mask", "invalid Mask")),
        }
    }
}

/// Groups similar kinds of messages together.
///
/// An interface is a "group" of similar kinds of messages. Some interfaces have default ID's
/// (see unit variants), while other interfaces like the **USB Device** and **Request Completion**
/// get alloted interface ID's during the lifecycle a USB redirection channel.
///
/// If a server:
///
/// * wants to send a message using the **USB Device** interface, it should encode an
///   [`Interface::NonDefault`].
/// * encounters a decoded [`Interface::NonDefault`], then the message was sent on the **Request
///   Completion** interface.
///
/// If a client:
///
/// * wants to send a message using the **Request Completion** interface, it should encode an
///   [`Interface::NonDefault`].
/// * encounters a decoded [`Interface::NonDefault`], then the message was sent on the **USB
///   Device** interface.
///
/// A server/client should additionally check the decoded value inside [`Interface::NonDefault`]
/// with the previously used interface values for **USB Device** and **Request Completion**
/// interfaces for the current USB redirection channel.
///
/// Max value for interface ID's: `0x3F_FF_FF_FF`.
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum Interface {
    /// **Exchange Capabilities** interface (ID: `0x00000000`). Used by both client and server to
    /// exchange capablities for interface manipulation.
    ///
    /// * [MS-RDPEUSB § 2.2.3 Interface Manipulation Exchange Capabilities Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6aee4e70-9d3b-49d7-a9b9-3c437cb27c8e
    ExchangeCapabilites = 0x0,

    /// **Device Sink** interface (ID: `0x00000001`). Used by the client to communicate with the
    /// server about new USB devices.
    ///
    /// * [MS-RDPEUSB § 2.2.4 Device Sink Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a9a8add7-4e99-4697-abd0-ad64c80c788d
    DeviceSink = 0x1,

    /// **Channel Notification** interface (ID: `0x00000002`). Used by the server to communicate with
    /// the client.
    ///
    /// * [MS-RDPEUSB § 2.2.5 Channel Notification Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62
    ServerToClientNotify = 0x2,

    /// **Channel Notification** interface (ID: `0x00000003`). Used by the client to communicate with
    /// the server.
    ///
    /// * [MS-RDPEUSB § 2.2.5 Channel Notification Interface][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a7ea1b33-80bb-4197-a502-ee62
    ClientToServerNotify = 0x3,

    /// Either the **USB Device** interface or the **Request Completion** interface. See type level
    /// docs for more info. Should be in the range `0x00_00_00_04..=0x3F_FF_FF_FF` (2 MSB's are discarded).
    ///
    /// * [MS-RDPEUSB § 2.2.6 USB Device Interface][1]
    /// * [MS-RDPEUSB § 2.2.7 Request Completion Interface][2]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/034257d7-f7a8-4fe1-b8c2-87ac8dc4f50e
    /// [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c0a146fc-20cf-4897-af27-a3c5474151ac
    NonDefault(u32),
}

impl From<Interface> for u32 {
    fn from(value: Interface) -> Self {
        match value {
            Interface::ExchangeCapabilites => 0x0,
            Interface::DeviceSink => 0x1,
            Interface::ServerToClientNotify => 0x2,
            Interface::ClientToServerNotify => 0x3,
            Interface::NonDefault(id) => id,
        }
    }
}

impl From<u32> for Interface {
    /// Constructs an [`Interface`] in the range `..=0x3F_FF_FF_FF`, with 2 MSB's discarded.
    fn from(value: u32) -> Self {
        match value & 0x3F_FF_FF_FF {
            0x0 => Self::ExchangeCapabilites,
            0x1 => Self::DeviceSink,
            0x2 => Self::ServerToClientNotify,
            0x3 => Self::ClientToServerNotify,
            id => Self::NonDefault(id),
        }
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
/// See [`Interface`] for more info on interfaces.
///
/// * [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71cfb32c-ba15-4f95-9241-70f9df273909
#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct FunctionId(u32);

impl FunctionId {
    // // Needed for QI_REQ and QI_RSP
    //
    // /// Release the given interface ID.
    // pub const RIMCALL_RELEASE: Self = Self(0x00000001);
    // pub const RIMCALL_QUERYINTERFACE: Self = Self(0x00000002);

    // -------------------- Exchange Capabilities Interface ---------------------------------------

    /// The server sends the [`RIM_EXCHANGE_CAPABILITY_REQUEST`][1] message, or the client sends
    /// the [`RIM_EXCHANGE_CAPABILITY_RESPONSE`][2] message in response to the former message.
    ///
    /// [1]: super::exchange_capabilities::RimExchangeCapabilityRequest
    /// [2]: super::exchange_capabilities::RimExchangeCapabilityResponse
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
        if matches!(value, 0x001 | 0x002 | 0x100..=0x107) {
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
    /// Only bits 2-31 of interface occupy bits 2-31 of the heaeder (in BE).
    pub interface: Interface,
    /// Bits 6-7 of a mask occupy bits 0-1 of the header (in BE).
    pub mask: Mask,
    pub message_id: MessageId,
    pub function_id: Option<FunctionId>,
}

impl Encode for SharedMsgHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let first32 = u32::from(self.interface) | (u32::from(self.mask) << 30);
        dst.write_u32(first32);
        dst.write_u32(self.message_id);

        if let Some(fn_id) = self.function_id {
            dst.write_u32(fn_id.0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "SHARED_MSG_HEADER"
    }

    fn size(&self) -> usize {
        const INTERFACE: usize = const { size_of::<Interface>() }; // Along with mask
        const MESSAGE_ID: usize = const { size_of::<MessageId>() };

        let fn_id = if self.function_id.is_some() {
            size_of_val(&self.function_id)
        } else {
            0
        };

        strict_sum(&[INTERFACE + MESSAGE_ID + fn_id])
    }
}

impl Decode<'_> for SharedMsgHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<Interface>() + size_of::<MessageId>());

        let first32 = src.read_u32();
        let interface = Interface::from(first32);
        let mask = Mask::try_from(first32)?;
        let msg_id = src.read_u32();

        let function_id = if mask == Mask::StreamIdStub {
            None
        } else {
            ensure_size!(in: src, size: size_of::<FunctionId>());
            let fn_id = src.read_u32();
            Some(FunctionId::try_from(fn_id)?)
        };

        Ok(SharedMsgHeader {
            interface,
            mask,
            message_id: msg_id,
            function_id,
        })
    }
}

/// Null-terminated UTF-16LE byte array without Byte Order Mark.
pub struct Utf16Le {
    /// No. of Unicode characters in `inner`
    cch: u32,
    /// Byte-pair array containing Unicode string in UTF-16LE (Null-terminated).
    inner: Vec<u16>,
}

impl Utf16Le {
    /// No. of Unicode characters in the byte array.
    pub fn cch(&self) -> u32 {
        self.cch
    }

    /// Get the byte slice containing null-terminated Unicode string.
    pub fn inner(&self) -> &[u16] {
        &self.inner
    }

    /// Size in bytes needed to encode this a [`Utf16LeByteArray`].
    pub fn size(&self) -> usize {
        const CCH: usize = const { size_of::<u32>() };
        CCH + (self.inner.len() << 1)
    }

    pub fn encode_no_cch(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.inner.len() << 1);
        for ch in &self.inner {
            dst.write_u16(*ch);
        }
        Ok(())
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.cch);
        self.encode_no_cch(dst)
    }

    fn decode_upto_null(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let mut cch = 0;
        let mut inner = Vec::new();

        loop {
            ensure_size!(in: src, size: size_of::<u16>());
            let ch = src.read_u16();
            inner.push(ch);
            cch += 1;

            if ch == 0x0 {
                break;
            } else if (0xD800..=0xDC00).contains(&ch) {
                ensure_size!(in: src, size: size_of::<u16>());
                let low_surrogate = src.read_u16();
                inner.push(low_surrogate);
            }
        }

        Ok(Self { cch, inner })
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>());

        let cch: u32 = src.read_u32();
        let upto_null = Self::decode_upto_null(src)?;

        if upto_null.cch != cch {
            Err::<Self, DecodeError>(invalid_field_err!(
                "Null-terminated Unicode string",
                "has characters unequal to preceeding `cch` field"
            ))
        } else {
            Ok(upto_null)
        }
    }
}

impl<A> FromIterator<A> for Utf16Le
where
    u16: From<A>,
{
    /// Construct a [`Utf16LeByteArray`] from an iterator of `u16`. Validity of `u16`s for UTF-16
    /// is not checked. If supplied iterator doesnt end with null, then null is appended.
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        let mut inner = Vec::new();
        let mut cch = 0;
        let mut iter = iter.into_iter().map(Into::<u16>::into);

        while let Some(ch) = iter.next() {
            inner.push(ch);
            if (0xD800..=0xDC00).contains(&ch) {
                let low_surrogate = iter.next().expect("no low surogate after encountering high surrogate.");
                inner.push(low_surrogate);
            }
            cch += 1;
        }

        // push null if not in input
        if let Some(1..) = inner.last() {
            inner.push(0x0);
            cch += 1;
        }

        Self { cch, inner }
    }
}

pub struct MultiSZ {
    cch: u32,
    inner: Vec<Utf16Le>,
}

impl MultiSZ {
    pub fn cch(&self) -> u32 {
        self.cch
    }

    pub fn inner(&self) -> &[Utf16Le] {
        &self.inner
    }

    pub fn size(&self) -> usize {
        const CCH: usize = const { size_of::<u32>() };
        let mut len = 0;
        for utf16le in &self.inner {
            len += utf16le.inner.len() << 1;
        }
        CCH + len
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.cch);

        for utf16le in &self.inner {
            utf16le.encode_no_cch(dst)?;
        }

        Ok(())
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>());

        let mut inner = Vec::new();
        let cch = src.read_u32();
        let mut count = 0;

        while count < cch {
            let utf16le = Utf16Le::decode_upto_null(src)?;
            count += utf16le.cch;
            inner.push(utf16le);
        }

        if count != cch {
            Err::<Self, DecodeError>(invalid_field_err!(
                "Null-terminated MultiSZ string",
                "has characters unequal to preceeding `cch` field"
            ))
        } else {
            Ok(Self { cch, inner })
        }
    }
}

impl<A> FromIterator<A> for MultiSZ
where
    Utf16Le: From<A>,
{
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        let mut inner = Vec::new();
        let mut cch = 0;
        let iter = iter.into_iter().map(Into::<Utf16Le>::into);

        for utf16le_ba in iter {
            cch += utf16le_ba.cch;
            inner.push(utf16le_ba);
        }

        // push null if not in input
        if let Some(last) = inner.last() {
            if last.cch != 1 || last.inner == vec![0] {
                inner.push(Utf16Le { cch: 1, inner: vec![0] });
                cch += 1;
            }
        }

        Self { cch, inner }
    }
}
