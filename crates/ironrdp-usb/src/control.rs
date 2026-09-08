//! USB control-request setup packets.
//!
//! The setup packet layout is [USB 2.0] 9.3 "USB Device Requests", its request
//! codes are Table 9-4 "Standard Request Codes", and the requests built on top
//! of them are 9.4 "Standard Device Requests".
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

use core::fmt;

use super::descriptor::descriptor_type;
use crate::Direction;

/// Type field in `bmRequestType` bits 6..5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct RequestKind(u8);

impl RequestKind {
    pub const STANDARD: Self = Self(0);
    pub const CLASS: Self = Self(1);
    pub const VENDOR: Self = Self(2);
    /// Value reserved by USB 2.0 and USB 3.2.
    pub const RESERVED: Self = Self(3);

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    const fn bits(self) -> u8 {
        self.0 << 5
    }
}

/// Recipient field in `bmRequestType` bits 4..0.
///
/// Values not defined by the common USB 2.0/3.x core set are preserved rather
/// than rejected. In particular, recipient 31 is vendor-specific in USB 3.2
/// but reserved by USB 2.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Recipient(u8);

impl Recipient {
    pub const DEVICE: Self = Self(0);
    pub const INTERFACE: Self = Self(1);
    pub const ENDPOINT: Self = Self(2);
    pub const OTHER: Self = Self(3);
    /// USB 3.x vendor-specific recipient (value 31); reserved by USB 2.0.
    pub const VENDOR_SPECIFIC: Self = Self(31);

    /// Construct a recipient from its five-bit field value.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Option<Self> {
        if raw <= 0x1f { Some(Self(raw)) } else { None }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }
}

/// Complete USB `bmRequestType` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct RequestType(u8);

impl RequestType {
    const DIRECTION_IN: u8 = 0x80;

    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn new(direction: Direction, kind: RequestKind, recipient: Recipient) -> Self {
        let direction = match direction {
            Direction::Out => 0,
            Direction::In => Self::DIRECTION_IN,
        };
        Self(direction | kind.bits() | recipient.raw())
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn direction(self) -> Direction {
        if self.0 & Self::DIRECTION_IN == 0 {
            Direction::Out
        } else {
            Direction::In
        }
    }

    #[must_use]
    pub const fn kind(self) -> RequestKind {
        RequestKind((self.0 >> 5) & 0x03)
    }

    #[must_use]
    pub const fn recipient(self) -> Recipient {
        Recipient(self.0 & 0x1f)
    }
}

/// Standard request codes from the USB device framework.
///
/// These values have this meaning only when [`RequestType::kind`] is
/// [`RequestKind::STANDARD`]. Class and vendor requests own their request-code
/// namespaces independently.
pub mod standard_request {
    pub const GET_STATUS: u8 = 0;
    pub const CLEAR_FEATURE: u8 = 1;
    pub const SET_FEATURE: u8 = 3;
    pub const SET_ADDRESS: u8 = 5;
    pub const GET_DESCRIPTOR: u8 = 6;
    pub const SET_DESCRIPTOR: u8 = 7;
    pub const GET_CONFIGURATION: u8 = 8;
    pub const SET_CONFIGURATION: u8 = 9;
    pub const GET_INTERFACE: u8 = 10;
    pub const SET_INTERFACE: u8 = 11;
    pub const SYNCH_FRAME: u8 = 12;
    /// USB 3.x system-exit-latency parameters.
    pub const SET_SEL: u8 = 0x30;
    /// USB 3.x isochronous delay from host transmission to device receipt.
    pub const SET_ISOCHRONOUS_DELAY: u8 = 0x31;
}

/// Standard feature selectors, carried in `wValue` of `CLEAR_FEATURE` and
/// `SET_FEATURE` ([USB 2.0] Table 9-6).
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
pub mod standard_feature {
    /// Halt condition of the addressed endpoint. The only selector a `CLEAR_FEATURE`
    /// or `SET_FEATURE` with an endpoint recipient may name.
    pub const ENDPOINT_HALT: u16 = 0;
    /// Device-side remote wakeup, addressed to the device.
    pub const DEVICE_REMOTE_WAKEUP: u16 = 1;
    /// High-speed electrical test mode, addressed to the device.
    pub const TEST_MODE: u16 = 2;
}

/// A USB control-transfer setup packet (exactly eight bytes on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupPacket {
    pub request_type: RequestType,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl SetupPacket {
    pub const WIRE_SIZE: usize = 8;

    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::WIRE_SIZE]) -> Self {
        Self {
            request_type: RequestType::from_raw(bytes[0]),
            request: bytes[1],
            value: u16::from_le_bytes([bytes[2], bytes[3]]),
            index: u16::from_le_bytes([bytes[4], bytes[5]]),
            length: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, SetupPacketError> {
        let bytes: [u8; Self::WIRE_SIZE] = bytes.try_into().map_err(|_| SetupPacketError { actual: bytes.len() })?;
        Ok(Self::from_bytes(bytes))
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::WIRE_SIZE] {
        let value = self.value.to_le_bytes();
        let index = self.index.to_le_bytes();
        let length = self.length.to_le_bytes();
        [
            self.request_type.raw(),
            self.request,
            value[0],
            value[1],
            index[0],
            index[1],
            length[0],
            length[1],
        ]
    }

    /// Decode `bRequest` in the standard-request namespace.
    #[must_use]
    pub const fn standard_request(self) -> Option<u8> {
        if self.request_type.kind().raw() == RequestKind::STANDARD.raw() {
            Some(self.request)
        } else {
            None
        }
    }
}

/// A slice whose length is not one USB setup packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupPacketError {
    actual: usize,
}

impl SetupPacketError {
    #[must_use]
    pub const fn actual_length(self) -> usize {
        self.actual
    }
}

impl fmt::Display for SetupPacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "USB setup packet is {} bytes; expected {}",
            self.actual,
            SetupPacket::WIRE_SIZE
        )
    }
}

impl core::error::Error for SetupPacketError {}

/// Typed view of the fields shared by standard GET_DESCRIPTOR requests.
///
/// This view intentionally preserves the recipient and `wIndex`. Class
/// specifications can issue GET_DESCRIPTOR to an interface, and only string
/// descriptors universally interpret `wIndex` as a LANGID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GetDescriptorRequest {
    /// Recipient encoded in `bmRequestType`.
    pub recipient: Recipient,
    /// Descriptor-type byte from the high byte of `wValue`.
    pub descriptor_type: u8,
    /// Descriptor index from the low byte of `wValue`.
    pub descriptor_index: u8,
    /// Raw `wIndex`; this is a LANGID for string descriptors.
    pub index: u16,
    /// Maximum response length from `wLength`.
    pub requested_length: u16,
}

impl GetDescriptorRequest {
    #[must_use]
    pub const fn string_language_id(self) -> Option<u16> {
        if self.descriptor_type == descriptor_type::STRING {
            Some(self.index)
        } else {
            None
        }
    }
}

/// Why a setup packet cannot be viewed as a standard GET_DESCRIPTOR request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GetDescriptorRequestError {
    NotStandard,
    WrongRequest(u8),
    WrongDirection,
}

impl fmt::Display for GetDescriptorRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStandard => f.write_str("GET_DESCRIPTOR request type is not standard"),
            Self::WrongRequest(request) => {
                write!(f, "USB request code {request:#04x} is not GET_DESCRIPTOR")
            }
            Self::WrongDirection => f.write_str("GET_DESCRIPTOR with a data stage is not device-to-host"),
        }
    }
}

impl core::error::Error for GetDescriptorRequestError {}

impl TryFrom<SetupPacket> for GetDescriptorRequest {
    type Error = GetDescriptorRequestError;

    fn try_from(setup: SetupPacket) -> Result<Self, Self::Error> {
        if setup.request_type.kind() != RequestKind::STANDARD {
            return Err(GetDescriptorRequestError::NotStandard);
        }
        if setup.request != standard_request::GET_DESCRIPTOR {
            return Err(GetDescriptorRequestError::WrongRequest(setup.request));
        }
        // USB ignores the direction bit when wLength is zero.
        if setup.length != 0 && !matches!(setup.request_type.direction(), Direction::In) {
            return Err(GetDescriptorRequestError::WrongDirection);
        }

        let [descriptor_index, descriptor_type] = setup.value.to_le_bytes();
        Ok(Self {
            recipient: setup.request_type.recipient(),
            descriptor_type,
            descriptor_index,
            index: setup.index,
            requested_length: setup.length,
        })
    }
}
