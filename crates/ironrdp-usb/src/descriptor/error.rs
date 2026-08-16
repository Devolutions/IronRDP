//! Typed errors for USB descriptor framing and explicit validation.

use core::fmt;

use super::super::endpoint::EndpointAddress;
use crate::UsbSpeed;

/// Field associated with an invalid descriptor value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescriptorField {
    UsbVersion,
    DeviceRelease,
    Subclass,
    Protocol,
    TotalLength,
    ConfigurationValue,
    Attributes,
    StringLength,
    EndpointAddress,
    EndpointAttributes,
    MaxPacketSize,
}

impl fmt::Display for DescriptorField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UsbVersion => "bcdUSB",
            Self::DeviceRelease => "bcdDevice",
            Self::Subclass => "subclass",
            Self::Protocol => "protocol",
            Self::TotalLength => "wTotalLength",
            Self::ConfigurationValue => "bConfigurationValue",
            Self::Attributes => "bmAttributes",
            Self::StringLength => "string payload length",
            Self::EndpointAddress => "bEndpointAddress",
            Self::EndpointAttributes => "endpoint bmAttributes",
            Self::MaxPacketSize => "wMaxPacketSize",
        })
    }
}

/// Precise reason why a descriptor stream cannot be parsed or validated.
///
/// Parsing only emits framing and minimum-length errors. Semantic and topology
/// variants are emitted by the explicit `validate` methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescriptorErrorKind {
    BufferTooShort {
        needed: usize,
        available: usize,
    },
    UnexpectedType {
        expected: u8,
        actual: u8,
    },
    InvalidLength {
        descriptor_type: u8,
        declared: usize,
        minimum: usize,
    },
    DescriptorExceedsBoundary {
        descriptor_type: u8,
        declared: usize,
        remaining: usize,
    },
    TrailingBytes {
        consumed: usize,
        available: usize,
    },
    InvalidField {
        descriptor_type: u8,
        field: DescriptorField,
        value: u32,
    },
    UnexpectedDescriptor {
        descriptor_type: u8,
    },
    EndpointBeforeInterface,
    DuplicateInterface {
        interface: u8,
        alternate_setting: u8,
    },
    MissingDefaultAlternate {
        interface: u8,
    },
    InterfaceCountMismatch {
        declared: u8,
        actual: usize,
    },
    EndpointCountMismatch {
        interface: u8,
        alternate_setting: u8,
        declared: u8,
        actual: usize,
    },
    DuplicateEndpoint {
        interface: u8,
        alternate_setting: u8,
        address: EndpointAddress,
    },
    EndpointSharedAcrossInterfaces {
        address: EndpointAddress,
        first: u8,
        second: u8,
    },
}

/// Descriptor error with a byte offset into the supplied buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorError {
    offset: usize,
    kind: DescriptorErrorKind,
}

impl DescriptorError {
    pub(super) const fn new(offset: usize, kind: DescriptorErrorKind) -> Self {
        Self { offset, kind }
    }

    #[must_use]
    pub const fn offset(self) -> usize {
        self.offset
    }

    #[must_use]
    pub const fn kind(self) -> DescriptorErrorKind {
        self.kind
    }
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "USB descriptor error at offset {}: ", self.offset)?;
        match self.kind {
            DescriptorErrorKind::BufferTooShort { needed, available } => {
                write!(f, "need {needed} bytes but only {available} are available")
            }
            DescriptorErrorKind::UnexpectedType { expected, actual } => {
                write!(f, "expected descriptor type {expected:#04x}, got {actual:#04x}")
            }
            DescriptorErrorKind::InvalidLength {
                descriptor_type,
                declared,
                minimum,
            } => write!(
                f,
                "descriptor {descriptor_type:#04x} declares length {declared}, minimum is {minimum}"
            ),
            DescriptorErrorKind::DescriptorExceedsBoundary {
                descriptor_type,
                declared,
                remaining,
            } => write!(
                f,
                "descriptor {descriptor_type:#04x} declares {declared} bytes with only {remaining} remaining"
            ),
            DescriptorErrorKind::TrailingBytes { consumed, available } => {
                write!(f, "descriptor consumes {consumed} bytes but {available} were supplied")
            }
            DescriptorErrorKind::InvalidField {
                descriptor_type,
                field,
                value,
            } => write!(
                f,
                "descriptor {descriptor_type:#04x} has invalid {field} value {value:#x}"
            ),
            DescriptorErrorKind::UnexpectedDescriptor { descriptor_type } => {
                write!(
                    f,
                    "descriptor type {descriptor_type:#04x} is not valid in a configuration set"
                )
            }
            DescriptorErrorKind::EndpointBeforeInterface => f.write_str("endpoint appears before an interface"),
            DescriptorErrorKind::DuplicateInterface {
                interface,
                alternate_setting,
            } => write!(
                f,
                "duplicate interface {} alternate setting {}",
                interface, alternate_setting
            ),
            DescriptorErrorKind::MissingDefaultAlternate { interface } => {
                write!(f, "interface {} has no alternate setting zero", interface)
            }
            DescriptorErrorKind::InterfaceCountMismatch { declared, actual } => {
                write!(f, "configuration declares {declared} interfaces but contains {actual}")
            }
            DescriptorErrorKind::EndpointCountMismatch {
                interface,
                alternate_setting,
                declared,
                actual,
            } => write!(
                f,
                "interface {} alternate {} declares {declared} endpoints but contains {actual}",
                interface, alternate_setting
            ),
            DescriptorErrorKind::DuplicateEndpoint {
                interface,
                alternate_setting,
                address,
            } => write!(
                f,
                "interface {} alternate {} contains duplicate endpoint {:#04x}",
                interface,
                alternate_setting,
                address.raw()
            ),
            DescriptorErrorKind::EndpointSharedAcrossInterfaces { address, first, second } => write!(
                f,
                "endpoint {:#04x} is shared by interfaces {} and {}",
                address.raw(),
                first,
                second
            ),
        }
    }
}

impl core::error::Error for DescriptorError {}

/// Speed-dependent device-descriptor semantic error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDescriptorError {
    speed: UsbSpeed,
    encoded_max_packet_size_0: u8,
}

impl DeviceDescriptorError {
    pub(super) const fn new(speed: UsbSpeed, encoded_max_packet_size_0: u8) -> Self {
        Self {
            speed,
            encoded_max_packet_size_0,
        }
    }

    #[must_use]
    pub const fn speed(self) -> UsbSpeed {
        self.speed
    }

    #[must_use]
    pub const fn encoded_max_packet_size_0(self) -> u8 {
        self.encoded_max_packet_size_0
    }
}

impl fmt::Display for DeviceDescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid bMaxPacketSize0 {} for {:?} speed",
            self.encoded_max_packet_size_0, self.speed
        )
    }
}

impl core::error::Error for DeviceDescriptorError {}
