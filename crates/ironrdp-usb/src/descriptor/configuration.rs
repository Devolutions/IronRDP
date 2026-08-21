//! Borrowed configuration descriptor views and explicit topology validation.
//!
//! The layouts this module walks are defined by [USB 2.0] 9.6.3
//! "Configuration", 9.6.5 "Interface", and 9.6.6 "Endpoint". The order they
//! must appear in when returned as one `wTotalLength` block is 9.4.3 "Get
//! Descriptor".
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

use super::super::endpoint::{
    EndpointAddress, EndpointAddressError, EndpointAttributes, IsochronousUsageType, MaxPacketSize,
};
use super::{
    CONFIGURATION_DESCRIPTOR_MIN_LENGTH, DescriptorError, DescriptorErrorKind, DescriptorField, DescriptorIter,
    ENDPOINT_DESCRIPTOR_MIN_LENGTH, INTERFACE_DESCRIPTOR_MIN_LENGTH, RawDescriptor, descriptor_type, invalid_field,
    le_u16, require_minimum_length, validate_class_code,
};
use crate::{ClassCode, TransferType, UsbSpeed};

/// Raw configuration `bmAttributes` with non-destructive field accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ConfigurationAttributes(u8);

impl ConfigurationAttributes {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn self_powered(self) -> bool {
        self.0 & 0x40 != 0
    }

    #[must_use]
    pub const fn remote_wakeup(self) -> bool {
        self.0 & 0x20 != 0
    }
}

/// Borrowed view of a standard configuration descriptor header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigurationDescriptor<'a> {
    raw: RawDescriptor<'a>,
}

impl<'a> ConfigurationDescriptor<'a> {
    #[must_use]
    pub const fn raw_descriptor(self) -> RawDescriptor<'a> {
        self.raw
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.raw.as_bytes()
    }

    #[must_use]
    pub const fn length(self) -> u8 {
        self.as_bytes()[0]
    }

    #[must_use]
    pub fn total_length(self) -> u16 {
        le_u16(self.as_bytes(), 2)
    }

    #[must_use]
    pub const fn num_interfaces(self) -> u8 {
        self.as_bytes()[4]
    }

    #[must_use]
    pub const fn configuration_value(self) -> u8 {
        self.as_bytes()[5]
    }

    #[must_use]
    pub const fn configuration_string(self) -> u8 {
        self.as_bytes()[6]
    }

    #[must_use]
    pub const fn attributes(self) -> ConfigurationAttributes {
        ConfigurationAttributes::from_raw(self.as_bytes()[7])
    }

    #[must_use]
    pub const fn max_power_raw(self) -> u8 {
        self.as_bytes()[8]
    }

    /// Maximum bus current in milliamperes, interpreted for negotiated speed.
    #[must_use]
    pub fn max_power_milliamps(self, speed: UsbSpeed) -> u16 {
        let unit = if speed.is_superspeed() { 8 } else { 2 };
        u16::from(self.max_power_raw()) * unit
    }
}

/// Borrowed view of an interface descriptor and its subordinate descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceDescriptor<'a> {
    raw: RawDescriptor<'a>,
    configuration_bytes: &'a [u8],
}

impl<'a> InterfaceDescriptor<'a> {
    #[must_use]
    pub const fn raw_descriptor(self) -> RawDescriptor<'a> {
        self.raw
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.raw.as_bytes()
    }

    #[must_use]
    pub const fn offset(self) -> usize {
        self.raw.offset()
    }

    #[must_use]
    pub const fn length(self) -> u8 {
        self.as_bytes()[0]
    }

    #[must_use]
    pub const fn number(self) -> u8 {
        self.as_bytes()[2]
    }

    #[must_use]
    pub const fn alternate_setting(self) -> u8 {
        self.as_bytes()[3]
    }

    #[must_use]
    pub const fn num_endpoints(self) -> u8 {
        self.as_bytes()[4]
    }

    #[must_use]
    pub const fn class(self) -> ClassCode {
        let bytes = self.as_bytes();
        ClassCode::new(bytes[5], bytes[6], bytes[7])
    }

    #[must_use]
    pub const fn interface_string(self) -> u8 {
        self.as_bytes()[8]
    }

    /// Descriptors belonging to this alternate setting, excluding the
    /// interface descriptor itself.
    #[must_use]
    pub fn descriptors(self) -> DescriptorIter<'a> {
        let start = self.offset() + self.raw.len();
        let mut end = self.configuration_bytes.len();
        for descriptor in DescriptorIter::new_framed(&self.configuration_bytes[start..], start) {
            if matches!(
                descriptor.descriptor_type(),
                descriptor_type::INTERFACE | descriptor_type::INTERFACE_ASSOCIATION
            ) {
                end = descriptor.offset();
                break;
            }
        }
        DescriptorIter::new_framed(&self.configuration_bytes[start..end], start)
    }

    #[must_use]
    pub fn endpoints(self) -> EndpointIter<'a> {
        EndpointIter {
            descriptors: self.descriptors(),
        }
    }

    #[must_use]
    pub fn endpoint(self, address: EndpointAddress) -> Option<EndpointDescriptor<'a>> {
        self.endpoints().find(|endpoint| endpoint.address() == Ok(address))
    }
}

/// Iterator over standard endpoint descriptors in one interface alternate.
#[derive(Debug, Clone)]
pub struct EndpointIter<'a> {
    descriptors: DescriptorIter<'a>,
}

impl<'a> Iterator for EndpointIter<'a> {
    type Item = EndpointDescriptor<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.descriptors
            .find_map(|raw| (raw.descriptor_type() == descriptor_type::ENDPOINT).then_some(EndpointDescriptor { raw }))
    }
}

/// Borrowed view of a standard endpoint descriptor prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointDescriptor<'a> {
    raw: RawDescriptor<'a>,
}

impl<'a> EndpointDescriptor<'a> {
    #[must_use]
    pub const fn raw_descriptor(self) -> RawDescriptor<'a> {
        self.raw
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.raw.as_bytes()
    }

    #[must_use]
    pub const fn offset(self) -> usize {
        self.raw.offset()
    }

    #[must_use]
    pub const fn length(self) -> u8 {
        self.as_bytes()[0]
    }

    #[must_use]
    pub const fn address_raw(self) -> u8 {
        self.as_bytes()[2]
    }

    pub const fn address(self) -> Result<EndpointAddress, EndpointAddressError> {
        EndpointAddress::from_raw(self.address_raw())
    }

    #[must_use]
    pub const fn attributes(self) -> EndpointAttributes {
        EndpointAttributes::from_raw(self.as_bytes()[3])
    }

    #[must_use]
    pub const fn transfer_type(self) -> TransferType {
        self.attributes().transfer_type()
    }

    #[must_use]
    pub fn max_packet_size(self) -> MaxPacketSize {
        MaxPacketSize::from_raw(le_u16(self.as_bytes(), 4))
    }

    #[must_use]
    pub const fn interval(self) -> u8 {
        self.as_bytes()[6]
    }
}

/// Iterator over every interface descriptor in a configuration set.
#[derive(Debug, Clone)]
pub struct InterfaceIter<'a> {
    descriptors: DescriptorIter<'a>,
    configuration_bytes: &'a [u8],
}

impl<'a> Iterator for InterfaceIter<'a> {
    type Item = InterfaceDescriptor<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.descriptors.find_map(|raw| {
            (raw.descriptor_type() == descriptor_type::INTERFACE).then_some(InterfaceDescriptor {
                raw,
                configuration_bytes: self.configuration_bytes,
            })
        })
    }
}

/// Borrowed complete configuration descriptor set through `wTotalLength`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigurationDescriptorSet<'a> {
    bytes: &'a [u8],
    configuration: ConfigurationDescriptor<'a>,
}

impl<'a> ConfigurationDescriptorSet<'a> {
    /// Read `wTotalLength` once enough of a configuration prefix is present.
    ///
    /// `Ok(None)` means more prefix bytes are required. An invalid type or an
    /// already-observable invalid length is reported immediately.
    pub fn required_length(prefix: &[u8]) -> Result<Option<usize>, DescriptorError> {
        if prefix.is_empty() {
            return Ok(None);
        }
        let declared_length = usize::from(prefix[0]);
        if declared_length < CONFIGURATION_DESCRIPTOR_MIN_LENGTH {
            return Err(DescriptorError::new(
                0,
                DescriptorErrorKind::InvalidLength {
                    descriptor_type: descriptor_type::CONFIGURATION,
                    declared: declared_length,
                    minimum: CONFIGURATION_DESCRIPTOR_MIN_LENGTH,
                },
            ));
        }
        if prefix.len() < 2 {
            return Ok(None);
        }
        let actual = prefix[1];
        if actual != descriptor_type::CONFIGURATION {
            return Err(DescriptorError::new(
                0,
                DescriptorErrorKind::UnexpectedType {
                    expected: descriptor_type::CONFIGURATION,
                    actual,
                },
            ));
        }
        if prefix.len() < 4 {
            return Ok(None);
        }

        let raw_total_length = le_u16(prefix, 2);
        let total_length = usize::from(raw_total_length);
        if total_length < declared_length {
            return Err(invalid_field(
                0,
                descriptor_type::CONFIGURATION,
                DescriptorField::TotalLength,
                u32::from(raw_total_length),
            ));
        }
        Ok(Some(total_length))
    }

    /// Parse when a GET_DESCRIPTOR response contains all of `wTotalLength`.
    pub fn parse_if_complete(bytes: &'a [u8]) -> Result<Option<Self>, DescriptorError> {
        let Some(required) = Self::required_length(bytes)? else {
            return Ok(None);
        };
        if bytes.len() < required {
            return Ok(None);
        }
        Self::parse(bytes).map(Some)
    }

    /// Frame one complete configuration descriptor set.
    ///
    /// Bytes after `wTotalLength` are not consumed. Parsing checks boundaries
    /// and the minimum sizes required by the typed views, but deliberately does
    /// not reject semantic quirks such as a zero configuration value, endpoint
    /// topology mismatch, or reserved field values. Use [`Self::validate`] when
    /// conformance is required: it returns a
    /// [`ValidConfigurationDescriptorSet`], which conformance-dependent APIs
    /// can require by signature.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DescriptorError> {
        let Some(total_length) = Self::required_length(bytes)? else {
            return Err(DescriptorError::new(
                0,
                DescriptorErrorKind::BufferTooShort {
                    needed: 4,
                    available: bytes.len(),
                },
            ));
        };
        if bytes.len() < total_length {
            return Err(DescriptorError::new(
                0,
                DescriptorErrorKind::BufferTooShort {
                    needed: total_length,
                    available: bytes.len(),
                },
            ));
        }

        let bytes = &bytes[..total_length];
        let mut descriptors = DescriptorIter::new(bytes)?;
        let configuration_raw = descriptors.next().ok_or_else(|| {
            DescriptorError::new(
                0,
                DescriptorErrorKind::BufferTooShort {
                    needed: CONFIGURATION_DESCRIPTOR_MIN_LENGTH,
                    available: 0,
                },
            )
        })?;

        for descriptor in descriptors {
            let minimum = match descriptor.descriptor_type() {
                descriptor_type::INTERFACE => Some(INTERFACE_DESCRIPTOR_MIN_LENGTH),
                descriptor_type::ENDPOINT => Some(ENDPOINT_DESCRIPTOR_MIN_LENGTH),
                _ => None,
            };
            if let Some(minimum) = minimum {
                require_minimum_length(
                    descriptor.offset(),
                    descriptor.descriptor_type(),
                    descriptor.len(),
                    minimum,
                )?;
            }
        }

        Ok(Self {
            bytes,
            configuration: ConfigurationDescriptor { raw: configuration_raw },
        })
    }

    /// Validate context-independent Chapter 9 fields and USB endpoint topology.
    ///
    /// Speed-dependent packet and interval rules, class-specific descriptors,
    /// and the detailed SuperSpeed companion rules require additional context
    /// and are intentionally not asserted here.
    ///
    /// Success yields a [`ValidConfigurationDescriptorSet`], so an operation
    /// defined only for a conforming set can require that proof in its
    /// signature instead of restating the rules. `Self` is [`Copy`]: the
    /// unvalidated set stays usable, so quirky bytes remain forwardable
    /// byte-for-byte, and the returned value revokes nothing.
    pub fn validate(self) -> Result<ValidConfigurationDescriptorSet<'a>, DescriptorError> {
        self.validate_configuration_header()?;

        let mut has_current_interface = false;
        for descriptor in self.descriptors().skip(1) {
            let descriptor_type = descriptor.descriptor_type();
            match descriptor_type {
                descriptor_type::INTERFACE => {
                    let interface = InterfaceDescriptor {
                        raw: descriptor,
                        configuration_bytes: self.bytes,
                    };
                    validate_class_code(interface.offset(), descriptor_type, interface.class())?;
                    has_current_interface = true;
                }
                descriptor_type::ENDPOINT => {
                    if !has_current_interface {
                        return Err(DescriptorError::new(
                            descriptor.offset(),
                            DescriptorErrorKind::EndpointBeforeInterface,
                        ));
                    }
                    validate_endpoint(EndpointDescriptor { raw: descriptor })?;
                }
                descriptor_type::INTERFACE_ASSOCIATION => has_current_interface = false,
                descriptor_type::DEVICE | descriptor_type::CONFIGURATION | descriptor_type::STRING => {
                    return Err(DescriptorError::new(
                        descriptor.offset(),
                        DescriptorErrorKind::UnexpectedDescriptor { descriptor_type },
                    ));
                }
                _ => {}
            }
        }

        self.validate_interface_topology()?;

        Ok(ValidConfigurationDescriptorSet(self))
    }

    fn validate_configuration_header(self) -> Result<(), DescriptorError> {
        let configuration = self.configuration();
        if configuration.configuration_value() == 0 {
            return Err(invalid_field(
                0,
                descriptor_type::CONFIGURATION,
                DescriptorField::ConfigurationValue,
                0,
            ));
        }
        let attributes = configuration.attributes().raw();
        if attributes & 0x80 == 0 || attributes & 0x1f != 0 {
            return Err(invalid_field(
                0,
                descriptor_type::CONFIGURATION,
                DescriptorField::Attributes,
                u32::from(attributes),
            ));
        }
        Ok(())
    }

    fn validate_interface_topology(self) -> Result<(), DescriptorError> {
        // The nested re-iteration below is quadratic. This crate cannot
        // allocate lookup tables, and the input is bounded by the 16-bit
        // wTotalLength, which keeps the worst case acceptable for a
        // hostile-input path.
        let mut distinct_interfaces = 0usize;
        for interface in self.interfaces() {
            if self.interfaces().any(|previous| {
                previous.offset() < interface.offset()
                    && previous.number() == interface.number()
                    && previous.alternate_setting() == interface.alternate_setting()
            }) {
                return Err(DescriptorError::new(
                    interface.offset(),
                    DescriptorErrorKind::DuplicateInterface {
                        interface: interface.number(),
                        alternate_setting: interface.alternate_setting(),
                    },
                ));
            }

            if !self
                .interfaces()
                .any(|previous| previous.offset() < interface.offset() && previous.number() == interface.number())
            {
                distinct_interfaces += 1;
                if !self
                    .interfaces()
                    .any(|candidate| candidate.number() == interface.number() && candidate.alternate_setting() == 0)
                {
                    return Err(DescriptorError::new(
                        interface.offset(),
                        DescriptorErrorKind::MissingDefaultAlternate {
                            interface: interface.number(),
                        },
                    ));
                }
            }

            let actual_endpoints = interface.endpoints().count();
            if actual_endpoints != usize::from(interface.num_endpoints()) {
                return Err(DescriptorError::new(
                    interface.offset(),
                    DescriptorErrorKind::EndpointCountMismatch {
                        interface: interface.number(),
                        alternate_setting: interface.alternate_setting(),
                        declared: interface.num_endpoints(),
                        actual: actual_endpoints,
                    },
                ));
            }

            for endpoint in interface.endpoints() {
                let Ok(address) = endpoint.address() else {
                    // The field-level pass above reports the precise error.
                    continue;
                };
                if interface
                    .endpoints()
                    .any(|previous| previous.offset() < endpoint.offset() && previous.address() == Ok(address))
                {
                    return Err(DescriptorError::new(
                        endpoint.offset(),
                        DescriptorErrorKind::DuplicateEndpoint {
                            interface: interface.number(),
                            alternate_setting: interface.alternate_setting(),
                            address,
                        },
                    ));
                }

                for other in self
                    .interfaces()
                    .filter(|other| other.offset() < interface.offset() && other.number() != interface.number())
                {
                    if other
                        .endpoints()
                        .any(|other_endpoint| other_endpoint.address() == Ok(address))
                    {
                        return Err(DescriptorError::new(
                            endpoint.offset(),
                            DescriptorErrorKind::EndpointSharedAcrossInterfaces {
                                address,
                                first: other.number(),
                                second: interface.number(),
                            },
                        ));
                    }
                }
            }
        }

        if distinct_interfaces != usize::from(self.configuration().num_interfaces()) {
            return Err(DescriptorError::new(
                0,
                DescriptorErrorKind::InterfaceCountMismatch {
                    declared: self.configuration().num_interfaces(),
                    actual: distinct_interfaces,
                },
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub const fn configuration(self) -> ConfigurationDescriptor<'a> {
        self.configuration
    }

    #[must_use]
    pub fn interfaces(self) -> InterfaceIter<'a> {
        InterfaceIter {
            descriptors: self.descriptors(),
            configuration_bytes: self.bytes,
        }
    }

    #[must_use]
    pub fn interface(self, number: u8, alternate_setting: u8) -> Option<InterfaceDescriptor<'a>> {
        self.interfaces()
            .find(|interface| interface.number() == number && interface.alternate_setting() == alternate_setting)
    }

    pub fn default_interfaces(self) -> impl Iterator<Item = InterfaceDescriptor<'a>> {
        self.interfaces().filter(|interface| interface.alternate_setting() == 0)
    }

    #[must_use]
    pub const fn descriptors(self) -> DescriptorIter<'a> {
        DescriptorIter::new_framed(self.bytes, 0)
    }
}

/// A [`ConfigurationDescriptorSet`] that passed [`ConfigurationDescriptorSet::validate`].
///
/// This is a witness, not a wrapper: it carries no data of its own and exposes
/// no accessors beyond [`Self::as_set`]. It lets an API require, in its
/// signature, a descriptor set whose [USB 2.0] 9.6 field values and
/// interface/endpoint topology were already checked. In particular
/// `bNumInterfaces` matches the number of distinct interface numbers present
/// (9.6.3), each alternate setting's `bNumEndpoints` matches the endpoint
/// descriptors that follow it and every interface number has an alternate
/// setting zero (9.6.5), and every endpoint address is well formed, is not the
/// default control endpoint, is unique within its interface, and is not shared
/// with another interface number (9.6.6).
///
/// [`ConfigurationDescriptorSet::validate`] is the only constructor. There is
/// deliberately no unchecked one: the guarantee would otherwise be forgeable,
/// and every accessor stays available on the unvalidated set for callers that
/// only need to read bytes.
///
/// [USB 2.0]: https://www.usb.org/document-library/usb-20-specification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidConfigurationDescriptorSet<'a>(ConfigurationDescriptorSet<'a>);

impl<'a> ValidConfigurationDescriptorSet<'a> {
    /// The validated descriptor set.
    ///
    /// Dropping the evidence is always sound; there is no conversion in the
    /// other direction.
    #[must_use]
    pub const fn as_set(self) -> ConfigurationDescriptorSet<'a> {
        self.0
    }
}

fn validate_endpoint(endpoint: EndpointDescriptor<'_>) -> Result<(), DescriptorError> {
    let address = endpoint.address().map_err(|_| {
        invalid_field(
            endpoint.offset(),
            descriptor_type::ENDPOINT,
            DescriptorField::EndpointAddress,
            u32::from(endpoint.address_raw()),
        )
    })?;
    if address.is_default_control() {
        return Err(invalid_field(
            endpoint.offset(),
            descriptor_type::ENDPOINT,
            DescriptorField::EndpointAddress,
            u32::from(address.raw()),
        ));
    }

    validate_endpoint_attributes(endpoint.offset(), endpoint.attributes())?;
    validate_max_packet_size(endpoint.offset(), endpoint.transfer_type(), endpoint.max_packet_size())
}

fn validate_endpoint_attributes(offset: usize, attributes: EndpointAttributes) -> Result<(), DescriptorError> {
    let raw = attributes.raw();
    let invalid = if raw & 0xc0 != 0 {
        true
    } else {
        match attributes.transfer_type() {
            TransferType::Control | TransferType::Bulk => raw & 0x3c != 0,
            TransferType::Isochronous => matches!(attributes.isochronous_usage(), Some(IsochronousUsageType::RESERVED)),
            // The notification subtype is defined by USB 3.x. Accept it
            // without speed context, but reject the two encodings reserved by
            // every currently modeled USB revision.
            TransferType::Interrupt => attributes.notification_interrupt().is_none(),
        }
    };
    if invalid {
        Err(invalid_field(
            offset,
            descriptor_type::ENDPOINT,
            DescriptorField::EndpointAttributes,
            u32::from(raw),
        ))
    } else {
        Ok(())
    }
}

fn validate_max_packet_size(
    offset: usize,
    transfer_type: TransferType,
    max_packet_size: MaxPacketSize,
) -> Result<(), DescriptorError> {
    let raw = max_packet_size.raw();
    let invalid = raw & 0xe000 != 0
        || max_packet_size.additional_transactions().is_err()
        || (matches!(transfer_type, TransferType::Control | TransferType::Bulk) && raw & 0x1800 != 0);
    if invalid {
        Err(invalid_field(
            offset,
            descriptor_type::ENDPOINT,
            DescriptorField::MaxPacketSize,
            u32::from(raw),
        ))
    } else {
        Ok(())
    }
}
