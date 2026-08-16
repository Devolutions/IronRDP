//! Borrowed device and string descriptor views.
//!
//! The two layouts are defined by [USB 2.0] 9.6.1 "Device" and 9.6.7 "String".
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

use super::{
    DEVICE_DESCRIPTOR_MIN_LENGTH, DescriptorError, DescriptorErrorKind, DescriptorField, DeviceDescriptorError,
    RawDescriptor, descriptor_type, invalid_field, le_u16, require_minimum_length, validate_class_code,
};
use crate::{BcdVersion, ClassCode, UsbSpeed};

/// Standard USB device descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDescriptor<'a> {
    raw: RawDescriptor<'a>,
}

impl<'a> DeviceDescriptor<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DescriptorError> {
        let available = bytes.len();
        let (descriptor, consumed) = Self::parse_prefix(bytes)?;
        if consumed != available {
            return Err(DescriptorError::new(
                consumed,
                DescriptorErrorKind::TrailingBytes { consumed, available },
            ));
        }
        Ok(descriptor)
    }

    /// Parse the first device descriptor in a larger buffer and report its
    /// declared `bLength`.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, usize), DescriptorError> {
        let raw = parse_standalone_descriptor(bytes, descriptor_type::DEVICE, DEVICE_DESCRIPTOR_MIN_LENGTH)?;
        let consumed = raw.len();
        Ok((Self { raw }, consumed))
    }

    /// Validate context-independent device descriptor fields.
    ///
    /// Endpoint-zero packet size depends on the negotiated speed and is
    /// therefore validated separately by [`Self::endpoint_zero_max_packet_size`].
    pub fn validate(self) -> Result<(), DescriptorError> {
        let usb_version = self.usb_version();
        if !usb_version.is_valid() {
            return Err(invalid_field(
                0,
                descriptor_type::DEVICE,
                DescriptorField::UsbVersion,
                u32::from(usb_version.raw()),
            ));
        }
        let device_release = self.device_release();
        if !device_release.is_valid() {
            return Err(invalid_field(
                0,
                descriptor_type::DEVICE,
                DescriptorField::DeviceRelease,
                u32::from(device_release.raw()),
            ));
        }
        validate_class_code(0, descriptor_type::DEVICE, self.class())
    }

    #[must_use]
    pub const fn raw_descriptor(self) -> RawDescriptor<'a> {
        self.raw
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.raw.as_bytes()
    }

    #[must_use]
    pub fn usb_version(self) -> BcdVersion {
        BcdVersion::from_raw(le_u16(self.as_bytes(), 2))
    }

    #[must_use]
    pub fn class(self) -> ClassCode {
        let bytes = self.as_bytes();
        ClassCode::new(bytes[4], bytes[5], bytes[6])
    }

    /// Raw `bMaxPacketSize0`; use [`Self::endpoint_zero_max_packet_size`] to
    /// interpret it for a negotiated speed.
    #[must_use]
    pub fn max_packet_size_0_raw(self) -> u8 {
        self.as_bytes()[7]
    }

    #[must_use]
    pub fn vendor_id(self) -> u16 {
        le_u16(self.as_bytes(), 8)
    }

    #[must_use]
    pub fn product_id(self) -> u16 {
        le_u16(self.as_bytes(), 10)
    }

    #[must_use]
    pub fn device_release(self) -> BcdVersion {
        BcdVersion::from_raw(le_u16(self.as_bytes(), 12))
    }

    #[must_use]
    pub fn manufacturer_string(self) -> u8 {
        self.as_bytes()[14]
    }

    #[must_use]
    pub fn product_string(self) -> u8 {
        self.as_bytes()[15]
    }

    #[must_use]
    pub fn serial_number_string(self) -> u8 {
        self.as_bytes()[16]
    }

    #[must_use]
    pub fn num_configurations(self) -> u8 {
        self.as_bytes()[17]
    }

    /// Interpret `bMaxPacketSize0` using negotiated bus speed.
    ///
    /// `bcdUSB` is deliberately not used as a substitute for speed: a USB 3.x
    /// capable device can enumerate using USB 2.0 endpoint-zero semantics.
    pub fn endpoint_zero_max_packet_size(self, speed: UsbSpeed) -> Result<u16, DeviceDescriptorError> {
        let encoded = self.max_packet_size_0_raw();
        let valid = match speed {
            UsbSpeed::Low => encoded == 8,
            UsbSpeed::Full => matches!(encoded, 8 | 16 | 32 | 64),
            UsbSpeed::High => encoded == 64,
            UsbSpeed::Super | UsbSpeed::SuperPlus => encoded == 9,
        };
        if !valid {
            return Err(DeviceDescriptorError::new(speed, encoded));
        }

        Ok(if speed.is_superspeed() {
            1u16 << encoded
        } else {
            u16::from(encoded)
        })
    }
}

/// Standard USB string descriptor payload.
///
/// Parsing only frames the descriptor. Call [`Self::validate`] before treating
/// every payload byte as UTF-16LE; a non-conforming final byte remains visible
/// through [`Self::trailing_byte`]. String index zero uses the same 16-bit units
/// for LANGIDs rather than Unicode text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringDescriptor<'a> {
    raw: RawDescriptor<'a>,
}

impl<'a> StringDescriptor<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DescriptorError> {
        let available = bytes.len();
        let (descriptor, consumed) = Self::parse_prefix(bytes)?;
        if consumed != available {
            return Err(DescriptorError::new(
                consumed,
                DescriptorErrorKind::TrailingBytes { consumed, available },
            ));
        }
        Ok(descriptor)
    }

    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, usize), DescriptorError> {
        let raw = parse_standalone_descriptor(bytes, descriptor_type::STRING, 2)?;
        let consumed = raw.len();
        Ok((Self { raw }, consumed))
    }

    pub fn validate(self) -> Result<(), DescriptorError> {
        // INVARIANT: `bLength` is a single byte, so a framed descriptor is at most 255 bytes long.
        let payload_len = self.as_bytes().len() - 2;
        if !payload_len.is_multiple_of(2) {
            return Err(invalid_field(
                0,
                descriptor_type::STRING,
                DescriptorField::StringLength,
                u32::try_from(payload_len).unwrap_or(u32::MAX),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn raw_descriptor(self) -> RawDescriptor<'a> {
        self.raw
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.raw.as_bytes()
    }

    pub fn code_units(self) -> impl ExactSizeIterator<Item = u16> + 'a {
        self.as_bytes()[2..]
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    #[must_use]
    pub fn trailing_byte(self) -> Option<u8> {
        let payload = &self.as_bytes()[2..];
        (!payload.len().is_multiple_of(2)).then(|| payload[payload.len() - 1])
    }
}

fn parse_standalone_descriptor<'a>(
    bytes: &'a [u8],
    expected: u8,
    minimum: usize,
) -> Result<RawDescriptor<'a>, DescriptorError> {
    let raw = RawDescriptor::parse_prefix(bytes)?;
    let actual = raw.descriptor_type();
    if actual != expected {
        return Err(DescriptorError::new(
            0,
            DescriptorErrorKind::UnexpectedType { expected, actual },
        ));
    }
    require_minimum_length(0, actual, raw.len(), minimum)?;
    Ok(raw)
}
