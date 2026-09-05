//! USB standard descriptors, lossless framing, typed views, and validation.
//!
//! Parsing and validation are intentionally separate:
//!
//! - parsing checks wire framing and the minimum lengths needed for safe typed
//!   access;
//! - validation checks context-independent field and topology rules from
//!   [USB 2.0] 9.6 "Standard USB Descriptor Definitions".
//!
//! This distinction matters for bridges: a device can successfully return a
//! quirky descriptor which still has to be forwarded byte-for-byte even when a
//! consumer declines to cache or interpret it. The distinction is reflected in
//! the return type of [`ConfigurationDescriptorSet::validate`], so an API that
//! acts on the described topology can require the checked set by signature.
//!
//! Unknown, class-specific, and not-yet-typed standard descriptors remain
//! available in wire order through [`ConfigurationDescriptorSet::descriptors`].
//! Descriptor views borrow the caller's bytes and do not allocate.
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

mod configuration;
mod device;
mod error;
mod raw;

pub use configuration::{
    ConfigurationAttributes, ConfigurationDescriptor, ConfigurationDescriptorSet, EndpointDescriptor, EndpointIter,
    InterfaceDescriptor, InterfaceIter, ValidConfigurationDescriptorSet,
};
pub use device::{DeviceDescriptor, StringDescriptor};
pub use error::{DescriptorError, DescriptorErrorKind, DescriptorField, DeviceDescriptorError};
pub use raw::{DescriptorIter, RawDescriptor, descriptor_type};

use crate::ClassCode;

/// `bLength` of a standard device descriptor ([USB 2.0] 9.6.1).
///
/// The descriptor is fixed size, so this is also how many bytes a
/// `GET_DESCRIPTOR` must ask for to read one whole.
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
pub const DEVICE_DESCRIPTOR_MIN_LENGTH: usize = 18;
/// `bLength` of a configuration descriptor header ([USB 2.0] 9.6.3).
///
/// Only the header is fixed size: it declares `wTotalLength`, the size of the
/// whole descriptor set that follows it.
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
pub const CONFIGURATION_DESCRIPTOR_MIN_LENGTH: usize = 9;
/// `bLength` of an interface descriptor ([USB 2.0] 9.6.5).
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
pub const INTERFACE_DESCRIPTOR_MIN_LENGTH: usize = 9;
/// `bLength` of an endpoint descriptor ([USB 2.0] 9.6.6).
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
pub const ENDPOINT_DESCRIPTOR_MIN_LENGTH: usize = 7;

fn require_minimum_length(
    offset: usize,
    descriptor_type: u8,
    declared: usize,
    minimum: usize,
) -> Result<(), DescriptorError> {
    if declared < minimum {
        Err(DescriptorError::new(
            offset,
            DescriptorErrorKind::InvalidLength {
                descriptor_type,
                declared,
                minimum,
            },
        ))
    } else {
        Ok(())
    }
}

fn invalid_field(offset: usize, descriptor_type: u8, field: DescriptorField, value: u32) -> DescriptorError {
    DescriptorError::new(
        offset,
        DescriptorErrorKind::InvalidField {
            descriptor_type,
            field,
            value,
        },
    )
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// Enforce the subclass rule shared by the device and interface descriptors.
///
/// [USB 2.0] Table 9-8 and Table 9-12 both state that a subclass code must be
/// reset to zero when its class code is, so the same check applies to
/// `bDeviceSubClass` and `bInterfaceSubClass` alike.
///
/// The protocol code is deliberately not checked: neither table requires it to
/// be zero when the class code is. Both only describe what a zero protocol code
/// means, so rejecting a nonzero one would invent a rule USB does not state.
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
fn validate_class_code(offset: usize, descriptor_type: u8, class: ClassCode) -> Result<(), DescriptorError> {
    if class.class == 0 && class.subclass != 0 {
        return Err(invalid_field(
            offset,
            descriptor_type,
            DescriptorField::Subclass,
            u32::from(class.subclass),
        ));
    }
    Ok(())
}
