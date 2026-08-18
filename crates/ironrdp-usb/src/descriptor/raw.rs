//! Raw descriptor identities and lossless traversal.
//!
//! The common `bLength` and `bDescriptorType` header every descriptor starts
//! with is defined by [USB 2.0] 9.5 "Descriptors", and the standard values of
//! `bDescriptorType` are Table 9-5 "Descriptor Types".
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

use super::{DescriptorError, DescriptorErrorKind};

/// Standard values in the open USB `bDescriptorType` namespace.
///
/// Descriptor types themselves remain plain `u8` so class-specific, vendor,
/// and future values can be preserved without wrapping every byte.
pub mod descriptor_type {
    pub const DEVICE: u8 = 0x01;
    pub const CONFIGURATION: u8 = 0x02;
    pub const STRING: u8 = 0x03;
    pub const INTERFACE: u8 = 0x04;
    pub const ENDPOINT: u8 = 0x05;
    pub const DEVICE_QUALIFIER: u8 = 0x06;
    pub const OTHER_SPEED_CONFIGURATION: u8 = 0x07;
    pub const INTERFACE_POWER: u8 = 0x08;
    pub const OTG: u8 = 0x09;
    pub const DEBUG: u8 = 0x0a;
    pub const INTERFACE_ASSOCIATION: u8 = 0x0b;
    pub const BOS: u8 = 0x0f;
    pub const DEVICE_CAPABILITY: u8 = 0x10;
    pub const SUPER_SPEED_ENDPOINT_COMPANION: u8 = 0x30;
    pub const SUPER_SPEED_PLUS_ISOCHRONOUS_ENDPOINT_COMPANION: u8 = 0x31;
}

/// Borrowed descriptor preserving its original bytes and wire offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDescriptor<'a> {
    offset: usize,
    bytes: &'a [u8],
}

impl<'a> RawDescriptor<'a> {
    /// Frame the first descriptor in `bytes`.
    ///
    /// Only the descriptor header and declared `bLength` are interpreted. The
    /// descriptor type and payload are deliberately left open.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<Self, DescriptorError> {
        Self::parse_at(bytes, 0, 0)
    }

    pub(super) fn parse_at(
        bytes: &'a [u8],
        relative_offset: usize,
        base_offset: usize,
    ) -> Result<Self, DescriptorError> {
        let offset = base_offset + relative_offset;
        let remaining = bytes.len().saturating_sub(relative_offset);
        if remaining < 2 {
            return Err(DescriptorError::new(
                offset,
                DescriptorErrorKind::BufferTooShort {
                    needed: 2,
                    available: remaining,
                },
            ));
        }

        let length = usize::from(bytes[relative_offset]);
        let descriptor_type = bytes[relative_offset + 1];
        if length < 2 {
            return Err(DescriptorError::new(
                offset,
                DescriptorErrorKind::InvalidLength {
                    descriptor_type,
                    declared: length,
                    minimum: 2,
                },
            ));
        }
        if length > remaining {
            return Err(DescriptorError::new(
                offset,
                DescriptorErrorKind::DescriptorExceedsBoundary {
                    descriptor_type,
                    declared: length,
                    remaining,
                },
            ));
        }

        Ok(Self {
            offset,
            bytes: &bytes[relative_offset..relative_offset + length],
        })
    }

    #[must_use]
    pub const fn offset(self) -> usize {
        self.offset
    }

    #[must_use]
    pub const fn descriptor_type(self) -> u8 {
        self.bytes[1]
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len()
    }

    /// Always `false`: framing guarantees at least the two header bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        false
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// Bytes following `bLength` and `bDescriptorType`.
    #[must_use]
    pub fn payload(self) -> &'a [u8] {
        &self.bytes[2..]
    }
}

/// Iterator over a structurally framed descriptor stream.
///
/// Construction scans the stream once. Iteration is then infallible and
/// preserves unknown and class-specific descriptors in wire order.
#[derive(Debug, Clone)]
pub struct DescriptorIter<'a> {
    bytes: &'a [u8],
    base_offset: usize,
    relative_offset: usize,
}

impl<'a> DescriptorIter<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, DescriptorError> {
        Self::with_offset(bytes, 0)
    }

    pub(super) fn with_offset(bytes: &'a [u8], base_offset: usize) -> Result<Self, DescriptorError> {
        let mut relative_offset = 0;
        while relative_offset < bytes.len() {
            let descriptor = RawDescriptor::parse_at(bytes, relative_offset, base_offset)?;
            relative_offset += descriptor.len();
        }
        Ok(Self::new_framed(bytes, base_offset))
    }

    pub(super) const fn new_framed(bytes: &'a [u8], base_offset: usize) -> Self {
        Self {
            bytes,
            base_offset,
            relative_offset: 0,
        }
    }
}

impl<'a> Iterator for DescriptorIter<'a> {
    type Item = RawDescriptor<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.relative_offset >= self.bytes.len() {
            return None;
        }

        // `DescriptorIter` can only be constructed after the complete stream
        // has been framed, so this cannot fail unless its invariant is broken.
        let descriptor = RawDescriptor::parse_at(self.bytes, self.relative_offset, self.base_offset).ok()?;
        self.relative_offset += descriptor.len();
        Some(descriptor)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.bytes.len().saturating_sub(self.relative_offset) / 2))
    }
}
