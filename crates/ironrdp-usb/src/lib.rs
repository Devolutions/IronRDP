#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![no_std]

pub mod control;
pub mod descriptor;
pub mod endpoint;
pub mod transfer;

/// USB transfer direction, always expressed from the host's point of view.
///
/// Direction is closed because the corresponding USB fields are one bit wide
/// and both possible encodings are defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Host to device.
    Out,
    /// Device to host.
    In,
}

/// USB bus speed used to interpret speed-dependent descriptor fields.
///
/// This is a high-level semantic value, not a directly decoded USB wire field.
/// Transports must explicitly map their own speed representation into it. An
/// enum is intentional here because interpreting an unknown future speed as an
/// existing speed would produce incorrect packet-size and power semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UsbSpeed {
    Low,
    Full,
    High,
    Super,
    SuperPlus,
}

impl UsbSpeed {
    #[must_use]
    pub const fn is_superspeed(self) -> bool {
        matches!(self, Self::Super | Self::SuperPlus)
    }
}

/// Transfer type encoded in endpoint descriptor `bmAttributes` bits 1..0.
///
/// All four possible two-bit encodings are defined by USB, so decoding cannot
/// encounter an unknown transfer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TransferType {
    Control = 0,
    Isochronous = 1,
    Bulk = 2,
    Interrupt = 3,
}

impl TransferType {
    #[must_use]
    pub const fn from_endpoint_attributes(attributes: u8) -> Self {
        match attributes & 0x03 {
            0 => Self::Control,
            1 => Self::Isochronous,
            2 => Self::Bulk,
            3 => Self::Interrupt,
            _ => unreachable!(),
        }
    }
}

/// A USB binary-coded-decimal version, such as `bcdUSB` and `bcdDevice`.
///
/// The raw value is preserved as-is. A device can report an encoding whose
/// nibbles are not decimal digits, and such a descriptor still has to be
/// forwarded byte-for-byte, so rejecting one is [`Self::is_valid`]'s job rather
/// than the constructor's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct BcdVersion(u16);

impl BcdVersion {
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Whether every nibble is a decimal digit.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        (self.0 & 0x000f) <= 9
            && ((self.0 >> 4) & 0x000f) <= 9
            && ((self.0 >> 8) & 0x000f) <= 9
            && ((self.0 >> 12) & 0x000f) <= 9
    }
}

/// One selected alternate setting of an interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterfaceSelection {
    /// `bInterfaceNumber` of the selected interface.
    pub interface: u8,
    /// `bAlternateSetting` of the selected alternate.
    pub alternate_setting: u8,
}

/// USB class, subclass, and protocol values.
///
/// These values deliberately remain open integers: USB-IF and device-class
/// specifications can assign values independently of this crate's release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClassCode {
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

impl ClassCode {
    #[must_use]
    pub const fn new(class: u8, subclass: u8, protocol: u8) -> Self {
        Self {
            class,
            subclass,
            protocol,
        }
    }
}
