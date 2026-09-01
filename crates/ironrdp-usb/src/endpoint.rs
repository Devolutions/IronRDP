//! Endpoint identifiers and descriptor bit-field semantics.
//!
//! The `bEndpointAddress`, `bmAttributes`, and `wMaxPacketSize` bit fields
//! interpreted here are defined by [USB 2.0] 9.6.6 "Endpoint".
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

use core::fmt;

use crate::{Direction, TransferType, UsbSpeed};

/// Error returned when an endpoint number is outside the four-bit USB range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointAddressError {
    raw: u8,
    kind: EndpointAddressErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndpointAddressErrorKind {
    NumberOutOfRange,
    ReservedBitsSet,
}

impl EndpointAddressError {
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.raw
    }

    #[must_use]
    pub const fn kind(self) -> EndpointAddressErrorKind {
        self.kind
    }
}

impl fmt::Display for EndpointAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            EndpointAddressErrorKind::NumberOutOfRange => {
                write!(f, "USB endpoint number {} is greater than 15", self.raw)
            }
            EndpointAddressErrorKind::ReservedBitsSet => {
                write!(f, "USB endpoint address {:#04x} has reserved bits set", self.raw)
            }
        }
    }
}

impl core::error::Error for EndpointAddressError {}

/// Endpoint number in the inclusive range 0..=15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EndpointNumber(u8);

impl EndpointNumber {
    pub const DEFAULT_CONTROL: Self = Self(0);

    pub const fn new(number: u8) -> Result<Self, EndpointAddressError> {
        if number <= 15 {
            Ok(Self(number))
        } else {
            Err(EndpointAddressError {
                raw: number,
                kind: EndpointAddressErrorKind::NumberOutOfRange,
            })
        }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }
}

/// Complete endpoint address: endpoint number plus host-relative direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EndpointAddress(u8);

impl EndpointAddress {
    const DIRECTION_IN: u8 = 0x80;
    const RESERVED_MASK: u8 = 0x70;

    pub const fn from_raw(raw: u8) -> Result<Self, EndpointAddressError> {
        if raw & Self::RESERVED_MASK != 0 {
            Err(EndpointAddressError {
                raw,
                kind: EndpointAddressErrorKind::ReservedBitsSet,
            })
        } else {
            Ok(Self(raw))
        }
    }

    #[must_use]
    pub const fn from_parts(number: EndpointNumber, direction: Direction) -> Self {
        let direction = match direction {
            Direction::Out => 0,
            Direction::In => Self::DIRECTION_IN,
        };
        Self(number.raw() | direction)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn number(self) -> EndpointNumber {
        // The mask guarantees the invariant established by EndpointNumber.
        EndpointNumber(self.0 & 0x0f)
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
    pub const fn is_default_control(self) -> bool {
        self.number().raw() == 0
    }
}

impl TryFrom<u8> for EndpointAddress {
    type Error = EndpointAddressError;

    fn try_from(raw: u8) -> Result<Self, Self::Error> {
        Self::from_raw(raw)
    }
}

impl From<EndpointAddress> for u8 {
    fn from(address: EndpointAddress) -> Self {
        address.raw()
    }
}

/// Synchronization field of an isochronous endpoint.
///
/// All four possible two-bit encodings are defined by USB, so decoding cannot
/// encounter an unknown synchronization type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum IsochronousSynchronizationType {
    None = 0,
    Asynchronous = 1,
    Adaptive = 2,
    Synchronous = 3,
}

/// Usage field of an isochronous endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct IsochronousUsageType(u8);

impl IsochronousUsageType {
    pub const DATA: Self = Self(0);
    pub const FEEDBACK: Self = Self(1);
    pub const IMPLICIT_FEEDBACK: Self = Self(2);
    /// Value reserved by USB 2.0.
    pub const RESERVED: Self = Self(3);

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }
}

/// Raw `bmAttributes` from an endpoint descriptor.
///
/// The full byte is retained because bits 5..2 have contextual meanings for
/// isochronous endpoints and USB 3.x interrupt endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct EndpointAttributes(u8);

impl EndpointAttributes {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn transfer_type(self) -> TransferType {
        TransferType::from_endpoint_attributes(self.0)
    }

    #[must_use]
    pub const fn isochronous_synchronization(self) -> Option<IsochronousSynchronizationType> {
        if !matches!(self.transfer_type(), TransferType::Isochronous) {
            return None;
        }

        Some(match (self.0 >> 2) & 0x03 {
            0 => IsochronousSynchronizationType::None,
            1 => IsochronousSynchronizationType::Asynchronous,
            2 => IsochronousSynchronizationType::Adaptive,
            3 => IsochronousSynchronizationType::Synchronous,
            _ => unreachable!(),
        })
    }

    #[must_use]
    pub const fn isochronous_usage(self) -> Option<IsochronousUsageType> {
        if !matches!(self.transfer_type(), TransferType::Isochronous) {
            return None;
        }

        Some(IsochronousUsageType((self.0 >> 4) & 0x03))
    }

    /// Whether a USB 3.x interrupt endpoint uses the notification subtype.
    ///
    /// `None` means the endpoint is not interrupt or uses a reserved encoding.
    #[must_use]
    pub const fn notification_interrupt(self) -> Option<bool> {
        if !matches!(self.transfer_type(), TransferType::Interrupt) || self.0 & 0x0c != 0 {
            return None;
        }

        match (self.0 >> 4) & 0x03 {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

/// Invalid USB 2.0 high-bandwidth transaction encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxPacketSizeError {
    raw: u16,
}

impl MaxPacketSizeError {
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.raw
    }
}

impl fmt::Display for MaxPacketSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "USB wMaxPacketSize {:#06x} uses the reserved high-bandwidth encoding",
            self.raw
        )
    }
}

impl core::error::Error for MaxPacketSizeError {}

/// Raw endpoint `wMaxPacketSize` with non-destructive field accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MaxPacketSize(u16);

impl MaxPacketSize {
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Maximum payload bytes in one transaction (`wMaxPacketSize` bits 10..0).
    #[must_use]
    pub const fn packet_size(self) -> u16 {
        self.0 & 0x07ff
    }

    const fn additional_transactions_raw(self) -> u16 {
        (self.0 >> 11) & 0x03
    }

    /// USB 2.0 high-speed additional transactions encoded in bits 12..11.
    ///
    /// This field is meaningful only for high-speed isochronous and interrupt
    /// endpoints. The return value is additional transactions (0, 1, or 2),
    /// not the total transaction count.
    pub const fn additional_transactions(self) -> Result<u8, MaxPacketSizeError> {
        match self.additional_transactions_raw() {
            0 => Ok(0),
            1 => Ok(1),
            2 => Ok(2),
            _ => Err(MaxPacketSizeError { raw: self.0 }),
        }
    }

    /// Maximum payload across a USB 2.0 high-speed service interval.
    ///
    /// The caller is responsible for using this only with high-speed periodic
    /// endpoints; other speeds encode bandwidth in different fields.
    pub fn high_speed_payload_per_microframe(self) -> Result<u32, MaxPacketSizeError> {
        let additional = self.additional_transactions()?;
        Ok(u32::from(self.packet_size()) * (u32::from(additional) + 1))
    }

    /// Whether this value satisfies the speed- and transfer-type-dependent
    /// endpoint limits for `speed`.
    ///
    /// At USB 2.0 speeds these are the packet-size limits from [USB 2.0]
    /// Table 5-1 together with the high-bandwidth encoding from Section 9.6.6.
    /// A SuperSpeed endpoint takes its limits from USB 3.2 Section 9.6.6.
    ///
    /// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
    #[must_use]
    pub const fn is_valid_for(self, speed: UsbSpeed, transfer_type: TransferType) -> bool {
        let raw = self.raw();
        let packet_size = self.packet_size();
        let additional_transactions = self.additional_transactions_raw();

        if raw & 0xe000 != 0 {
            return false;
        }

        match (speed, transfer_type) {
            (UsbSpeed::Low, TransferType::Control) => raw == 8,
            (UsbSpeed::Low, TransferType::Interrupt) => additional_transactions == 0 && matches!(packet_size, 1..=8),
            (UsbSpeed::Low, TransferType::Isochronous | TransferType::Bulk) => false,
            (UsbSpeed::Full, TransferType::Control | TransferType::Bulk) => {
                additional_transactions == 0 && matches!(packet_size, 8 | 16 | 32 | 64)
            }
            (UsbSpeed::Full, TransferType::Isochronous) => additional_transactions == 0 && packet_size <= 1023,
            (UsbSpeed::Full, TransferType::Interrupt) => additional_transactions == 0 && matches!(packet_size, 1..=64),
            (UsbSpeed::High, TransferType::Control) => raw == 64,
            (UsbSpeed::High, TransferType::Bulk) => raw == 512,
            (UsbSpeed::High, TransferType::Isochronous) => {
                raw == 0 || (additional_transactions <= 2 && matches!(packet_size, 1..=1024))
            }
            (UsbSpeed::High, TransferType::Interrupt) => {
                additional_transactions <= 2 && matches!(packet_size, 1..=1024)
            }
            (UsbSpeed::Super | UsbSpeed::SuperPlus, TransferType::Control) => raw == 512,
            (UsbSpeed::Super | UsbSpeed::SuperPlus, TransferType::Bulk) => raw == 1024,
            (UsbSpeed::Super | UsbSpeed::SuperPlus, TransferType::Isochronous) => {
                raw == 0 || (additional_transactions == 0 && matches!(packet_size, 1..=1024))
            }
            (UsbSpeed::Super | UsbSpeed::SuperPlus, TransferType::Interrupt) => {
                additional_transactions == 0 && matches!(packet_size, 1..=1024)
            }
        }
    }

    /// Maximum payload bytes moved by one scheduled service of this endpoint.
    ///
    /// A high-speed high-bandwidth isochronous or interrupt endpoint is served
    /// by up to three back-to-back transactions within one microframe
    /// ([USB 2.0] 5.9), so its payload is the packet size multiplied by the
    /// total transaction count encoded in `wMaxPacketSize` bits 12..11
    /// ([USB 2.0] 9.6.6). Every other speed and transfer type moves one packet
    /// per transaction, SuperSpeed included: USB 3.2 carries an endpoint's
    /// burst in its companion descriptor rather than in `wMaxPacketSize`.
    ///
    /// `None` when the encoding is not valid for `speed` and `transfer_type`;
    /// [`Self::is_valid_for`] defines exactly the same precondition.
    ///
    /// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
    #[must_use]
    pub const fn max_payload(self, speed: UsbSpeed, transfer_type: TransferType) -> Option<u16> {
        if !self.is_valid_for(speed, transfer_type) {
            return None;
        }

        let transactions = match (speed, transfer_type) {
            (UsbSpeed::High, TransferType::Isochronous | TransferType::Interrupt) => {
                self.additional_transactions_raw() + 1
            }
            _ => 1,
        };

        // `is_valid_for` bounds the packet size by 1024 and the additional
        // transactions by two, so the product is at most 3072.
        Some(self.packet_size() * transactions)
    }
}
