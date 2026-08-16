use ironrdp_usb::control::{
    GetDescriptorRequest, GetDescriptorRequestError, Recipient, RequestKind, RequestType, SetupPacket, standard_request,
};
use ironrdp_usb::descriptor::{
    ConfigurationDescriptorSet, DescriptorError, DescriptorErrorKind, DescriptorField, DeviceDescriptor, RawDescriptor,
    StringDescriptor,
};
use ironrdp_usb::endpoint::{EndpointAddress, EndpointAddressErrorKind, EndpointNumber, MaxPacketSize};
use ironrdp_usb::{Direction, TransferType, UsbSpeed};

/// Minimal conforming device descriptor, as defined by USB 2.0 Table 9-8.
const DEVICE_DESCRIPTOR: [u8; 18] = [
    18,   // bLength
    0x01, // bDescriptorType (DEVICE)
    0x00, 0x02, // bcdUSB = 2.00
    0x00, // bDeviceClass
    0x00, // bDeviceSubClass
    0x00, // bDeviceProtocol
    64,   // bMaxPacketSize0
    0x34, 0x12, // idVendor
    0x78, 0x56, // idProduct
    0x00, 0x01, // bcdDevice = 1.00
    0x00, // iManufacturer
    0x00, // iProduct
    0x00, // iSerialNumber
    0x01, // bNumConfigurations
];

/// Wrap `body` in a conforming configuration descriptor header (USB 2.0 Table 9-10).
fn configuration_set(num_interfaces: u8, body: &[u8]) -> Vec<u8> {
    let total_length = u16::try_from(9 + body.len()).unwrap().to_le_bytes();
    let mut bytes = vec![
        9,               // bLength
        0x02,            // bDescriptorType (CONFIGURATION)
        total_length[0], // wTotalLength
        total_length[1], //
        num_interfaces,  // bNumInterfaces
        1,               // bConfigurationValue
        0,               // iConfiguration
        0x80,            // bmAttributes (bus powered)
        50,              // bMaxPower
    ];
    bytes.extend_from_slice(body);
    bytes
}

/// Interface descriptor as defined by USB 2.0 Table 9-12.
fn interface(number: u8, alternate_setting: u8, num_endpoints: u8) -> [u8; 9] {
    [
        9,                 // bLength
        0x04,              // bDescriptorType (INTERFACE)
        number,            // bInterfaceNumber
        alternate_setting, // bAlternateSetting
        num_endpoints,     // bNumEndpoints
        0,                 // bInterfaceClass
        0,                 // bInterfaceSubClass
        0,                 // bInterfaceProtocol
        0,                 // iInterface
    ]
}

/// Bulk endpoint descriptor as defined by USB 2.0 Table 9-13.
fn endpoint(address: u8) -> [u8; 7] {
    [
        7,       // bLength
        0x05,    // bDescriptorType (ENDPOINT)
        address, // bEndpointAddress
        0x02,    // bmAttributes (bulk)
        0x00, 0x02, // wMaxPacketSize = 512
        0,    // bInterval
    ]
}

fn kind(error: DescriptorError) -> DescriptorErrorKind {
    error.kind()
}

#[test]
fn raw_descriptor_framing_boundaries() {
    // A descriptor is framed by `bLength`, so the header must be present and
    // the declared length must be both self-consistent and within the buffer.
    assert!(matches!(
        RawDescriptor::parse_prefix(&[0x02]).map_err(kind),
        Err(DescriptorErrorKind::BufferTooShort {
            needed: 2,
            available: 1
        })
    ));
    assert!(matches!(
        RawDescriptor::parse_prefix(&[1, 0x02]).map_err(kind),
        Err(DescriptorErrorKind::InvalidLength { declared: 1, .. })
    ));
    assert!(matches!(
        RawDescriptor::parse_prefix(&[5, 0x02, 0, 0]).map_err(kind),
        Err(DescriptorErrorKind::DescriptorExceedsBoundary {
            declared: 5,
            remaining: 4,
            ..
        })
    ));

    let descriptor = RawDescriptor::parse_prefix(&[2, 0x02, 0xff]).expect("header-only descriptor is framed");
    assert_eq!(descriptor.len(), 2, "bLength bounds the view, trailing bytes excluded");
}

#[test]
fn device_descriptor_length_boundaries() {
    assert!(DeviceDescriptor::parse(&DEVICE_DESCRIPTOR).is_ok());

    let mut short = DEVICE_DESCRIPTOR;
    short[0] = 17;
    assert!(matches!(
        DeviceDescriptor::parse(&short[..17]).map_err(kind),
        Err(DescriptorErrorKind::InvalidLength {
            declared: 17,
            minimum: 18,
            ..
        })
    ));

    let mut trailing = DEVICE_DESCRIPTOR.to_vec();
    trailing.push(0xff);
    assert!(matches!(
        DeviceDescriptor::parse(&trailing).map_err(kind),
        Err(DescriptorErrorKind::TrailingBytes {
            consumed: 18,
            available: 19
        })
    ));

    let mut wrong_type = DEVICE_DESCRIPTOR;
    wrong_type[1] = 0x02;
    assert!(matches!(
        DeviceDescriptor::parse(&wrong_type).map_err(kind),
        Err(DescriptorErrorKind::UnexpectedType {
            expected: 0x01,
            actual: 0x02
        })
    ));
}

#[test]
fn device_descriptor_validate_rejects_non_decimal_bcd_nibbles() {
    assert!(DeviceDescriptor::parse(&DEVICE_DESCRIPTOR).unwrap().validate().is_ok());

    let mut usb_version = DEVICE_DESCRIPTOR;
    usb_version[2] = 0x0a; // The low nibble of bcdUSB is not a decimal digit.
    assert!(matches!(
        DeviceDescriptor::parse(&usb_version).unwrap().validate().map_err(kind),
        Err(DescriptorErrorKind::InvalidField {
            field: DescriptorField::UsbVersion,
            value: 0x020a,
            ..
        })
    ));

    let mut device_release = DEVICE_DESCRIPTOR;
    device_release[13] = 0xf1; // The high nibble of bcdDevice is not a decimal digit.
    assert!(matches!(
        DeviceDescriptor::parse(&device_release)
            .unwrap()
            .validate()
            .map_err(kind),
        Err(DescriptorErrorKind::InvalidField {
            field: DescriptorField::DeviceRelease,
            value: 0xf100,
            ..
        })
    ));
}

#[test]
fn endpoint_zero_max_packet_size_is_speed_dependent() {
    let descriptor = DeviceDescriptor::parse(&DEVICE_DESCRIPTOR).unwrap();
    assert_eq!(descriptor.endpoint_zero_max_packet_size(UsbSpeed::High), Ok(64));
    assert_eq!(descriptor.endpoint_zero_max_packet_size(UsbSpeed::Full), Ok(64));
    assert!(descriptor.endpoint_zero_max_packet_size(UsbSpeed::Low).is_err());
    // SuperSpeed encodes the size as an exponent, so only 9 (meaning 512) is valid.
    assert!(descriptor.endpoint_zero_max_packet_size(UsbSpeed::Super).is_err());
}

#[test]
fn string_descriptor_rejects_an_odd_payload() {
    let even = StringDescriptor::parse(&[6, 0x03, b'a', 0, b'b', 0]).unwrap();
    assert!(even.validate().is_ok());
    assert_eq!(even.code_units().collect::<Vec<_>>(), vec![0x0061, 0x0062]);
    assert_eq!(even.trailing_byte(), None);

    let odd = StringDescriptor::parse(&[5, 0x03, b'a', 0, 0xff]).unwrap();
    assert!(odd.validate().is_err(), "payload is not a whole number of UTF-16 units");
    assert_eq!(odd.trailing_byte(), Some(0xff), "the odd byte stays observable");
}

#[test]
fn configuration_set_is_framed_by_total_length() {
    let mut body = interface(0, 0, 1).to_vec();
    body.extend_from_slice(&endpoint(0x81));
    let bytes = configuration_set(1, &body);
    let total_length = bytes.len();

    // `wTotalLength` only becomes readable at the fourth byte.
    assert_eq!(ConfigurationDescriptorSet::required_length(&bytes[..3]), Ok(None));
    assert_eq!(
        ConfigurationDescriptorSet::required_length(&bytes[..4]),
        Ok(Some(total_length))
    );

    assert_eq!(
        ConfigurationDescriptorSet::parse_if_complete(&bytes[..total_length - 1]),
        Ok(None),
        "one byte short is incomplete, not malformed"
    );
    assert!(ConfigurationDescriptorSet::parse_if_complete(&bytes).unwrap().is_some());
    assert!(matches!(
        ConfigurationDescriptorSet::parse(&bytes[..total_length - 1]).map_err(kind),
        Err(DescriptorErrorKind::BufferTooShort { .. })
    ));

    let mut trailing = bytes;
    trailing.extend_from_slice(&[0xff, 0xff]);
    let parsed = ConfigurationDescriptorSet::parse(&trailing).expect("bytes past wTotalLength are not consumed");
    assert_eq!(parsed.as_bytes().len(), total_length);
}

#[test]
fn configuration_set_validates_a_conforming_topology() {
    let mut body = interface(0, 0, 1).to_vec();
    body.extend_from_slice(&endpoint(0x81));
    body.extend_from_slice(&interface(1, 0, 0));
    let bytes = configuration_set(2, &body);

    let parsed = ConfigurationDescriptorSet::parse(&bytes).unwrap();
    let valid = parsed.validate().expect("a conforming topology validates");
    assert_eq!(valid.as_set().interfaces().count(), 2);
    assert!(
        valid
            .as_set()
            .interface(0, 0)
            .unwrap()
            .endpoint(EndpointAddress::from_raw(0x81).unwrap())
            .is_some()
    );
}

#[test]
fn configuration_set_validate_rejects_topology_violations() {
    // Parsing accepts every case below; only `validate` is meant to reject them.
    let cases: [(&str, u8, Vec<u8>); 6] = [
        ("endpoint before any interface", 1, endpoint(0x81).to_vec()),
        ("declared interface count mismatch", 2, interface(0, 0, 0).to_vec()),
        ("declared endpoint count mismatch", 1, interface(0, 0, 1).to_vec()),
        (
            "duplicate interface alternate",
            1,
            [interface(0, 0, 0), interface(0, 0, 0)].concat(),
        ),
        ("missing alternate setting zero", 1, interface(0, 1, 0).to_vec()),
        (
            "reserved high-bandwidth encoding",
            1,
            [
                interface(0, 0, 1).as_slice(),
                // Interrupt endpoint whose wMaxPacketSize bits 12..11 are 0b11.
                &[7, 0x05, 0x81, 0x03, 0x40, 0x18, 1],
            ]
            .concat(),
        ),
    ];

    for (case, num_interfaces, body) in cases {
        let bytes = configuration_set(num_interfaces, &body);
        let parsed = ConfigurationDescriptorSet::parse(&bytes).unwrap_or_else(|_| panic!("{case} should parse"));
        assert!(parsed.validate().is_err(), "{case} should not validate");
    }
}

#[test]
fn setup_packet_requires_exactly_eight_bytes() {
    let bytes = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00];
    assert!(SetupPacket::parse(&bytes).is_ok());

    for length in [0, 7, 9] {
        let buffer = vec![0; length];
        let error = SetupPacket::parse(&buffer).expect_err("only eight bytes are a setup packet");
        assert_eq!(error.actual_length(), length);
    }
}

#[test]
fn setup_packet_round_trips_through_its_wire_form() {
    for bytes in [[0x00; 8], [0xff; 8], [0x80, 0x06, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06]] {
        assert_eq!(SetupPacket::from_bytes(bytes).to_bytes(), bytes);
    }
}

#[test]
fn request_type_fields_partition_the_byte() {
    for raw in 0..=u8::MAX {
        let request_type = RequestType::from_raw(raw);
        let rebuilt = RequestType::new(request_type.direction(), request_type.kind(), request_type.recipient());
        assert_eq!(
            rebuilt.raw(),
            raw,
            "direction, kind and recipient must cover bmRequestType"
        );
    }

    assert!(Recipient::from_raw(0x1f).is_some());
    assert!(Recipient::from_raw(0x20).is_none(), "recipient is a five-bit field");
}

#[test]
fn get_descriptor_view_boundaries() {
    let get_descriptor = SetupPacket {
        request_type: RequestType::new(Direction::In, RequestKind::STANDARD, Recipient::DEVICE),
        request: standard_request::GET_DESCRIPTOR,
        value: 0x0100,
        index: 0,
        length: 18,
    };
    let view = GetDescriptorRequest::try_from(get_descriptor).unwrap();
    assert_eq!(view.descriptor_type, 0x01);
    assert_eq!(view.descriptor_index, 0x00);
    assert_eq!(view.string_language_id(), None, "wIndex is a LANGID only for strings");

    let class_request = SetupPacket {
        request_type: RequestType::new(Direction::In, RequestKind::CLASS, Recipient::DEVICE),
        ..get_descriptor
    };
    assert_eq!(
        GetDescriptorRequest::try_from(class_request),
        Err(GetDescriptorRequestError::NotStandard)
    );

    let wrong_request = SetupPacket {
        request: standard_request::GET_CONFIGURATION,
        ..get_descriptor
    };
    assert_eq!(
        GetDescriptorRequest::try_from(wrong_request),
        Err(GetDescriptorRequestError::WrongRequest(
            standard_request::GET_CONFIGURATION
        ))
    );

    let host_to_device = SetupPacket {
        request_type: RequestType::new(Direction::Out, RequestKind::STANDARD, Recipient::DEVICE),
        ..get_descriptor
    };
    assert_eq!(
        GetDescriptorRequest::try_from(host_to_device),
        Err(GetDescriptorRequestError::WrongDirection)
    );
    // USB ignores the direction bit when there is no data stage.
    assert!(
        GetDescriptorRequest::try_from(SetupPacket {
            length: 0,
            ..host_to_device
        })
        .is_ok()
    );
}

#[test]
fn endpoint_address_boundaries() {
    assert!(EndpointNumber::new(15).is_ok());
    assert_eq!(
        EndpointNumber::new(16).unwrap_err().kind(),
        EndpointAddressErrorKind::NumberOutOfRange
    );

    assert_eq!(
        EndpointAddress::from_raw(0x10).unwrap_err().kind(),
        EndpointAddressErrorKind::ReservedBitsSet
    );

    let address = EndpointAddress::from_raw(0x8f).unwrap();
    assert_eq!(address.number().raw(), 15);
    assert_eq!(address.direction(), Direction::In);
    assert_eq!(EndpointAddress::from_parts(address.number(), Direction::In), address);
    assert!(EndpointAddress::from_raw(0x00).unwrap().is_default_control());
}

#[test]
fn max_packet_size_rejects_the_reserved_high_bandwidth_encoding() {
    for (raw, expected) in [(0x0000, 0), (0x0800, 1), (0x1000, 2)] {
        assert_eq!(MaxPacketSize::from_raw(raw).additional_transactions(), Ok(expected));
    }
    assert!(MaxPacketSize::from_raw(0x1800).additional_transactions().is_err());

    // Three transactions of the maximum high-speed packet size.
    assert_eq!(
        MaxPacketSize::from_raw(0x1400).high_speed_payload_per_microframe(),
        Ok(3072)
    );
}

#[test]
fn validates_low_speed_packet_sizes() {
    assert!(MaxPacketSize::from_raw(8).is_valid_for_usb2(UsbSpeed::Low, TransferType::Control));
    assert!(!MaxPacketSize::from_raw(16).is_valid_for_usb2(UsbSpeed::Low, TransferType::Control));

    assert!(MaxPacketSize::from_raw(1).is_valid_for_usb2(UsbSpeed::Low, TransferType::Interrupt));
    assert!(MaxPacketSize::from_raw(8).is_valid_for_usb2(UsbSpeed::Low, TransferType::Interrupt));
    assert!(!MaxPacketSize::from_raw(0).is_valid_for_usb2(UsbSpeed::Low, TransferType::Interrupt));
    assert!(!MaxPacketSize::from_raw(9).is_valid_for_usb2(UsbSpeed::Low, TransferType::Interrupt));
    assert!(!MaxPacketSize::from_raw(0x0808).is_valid_for_usb2(UsbSpeed::Low, TransferType::Interrupt));

    assert!(!MaxPacketSize::from_raw(8).is_valid_for_usb2(UsbSpeed::Low, TransferType::Bulk));
    assert!(!MaxPacketSize::from_raw(8).is_valid_for_usb2(UsbSpeed::Low, TransferType::Isochronous));
}

#[test]
fn validates_full_speed_packet_sizes() {
    for packet_size in [8, 16, 32, 64] {
        assert!(MaxPacketSize::from_raw(packet_size).is_valid_for_usb2(UsbSpeed::Full, TransferType::Control));
        assert!(MaxPacketSize::from_raw(packet_size).is_valid_for_usb2(UsbSpeed::Full, TransferType::Bulk));
    }
    for packet_size in [0, 1, 7, 9, 63, 65, 512] {
        assert!(!MaxPacketSize::from_raw(packet_size).is_valid_for_usb2(UsbSpeed::Full, TransferType::Control));
        assert!(!MaxPacketSize::from_raw(packet_size).is_valid_for_usb2(UsbSpeed::Full, TransferType::Bulk));
    }

    assert!(MaxPacketSize::from_raw(0).is_valid_for_usb2(UsbSpeed::Full, TransferType::Isochronous));
    assert!(MaxPacketSize::from_raw(1023).is_valid_for_usb2(UsbSpeed::Full, TransferType::Isochronous));
    assert!(!MaxPacketSize::from_raw(1024).is_valid_for_usb2(UsbSpeed::Full, TransferType::Isochronous));

    assert!(MaxPacketSize::from_raw(1).is_valid_for_usb2(UsbSpeed::Full, TransferType::Interrupt));
    assert!(MaxPacketSize::from_raw(64).is_valid_for_usb2(UsbSpeed::Full, TransferType::Interrupt));
    assert!(!MaxPacketSize::from_raw(0).is_valid_for_usb2(UsbSpeed::Full, TransferType::Interrupt));
    assert!(!MaxPacketSize::from_raw(65).is_valid_for_usb2(UsbSpeed::Full, TransferType::Interrupt));
    assert!(!MaxPacketSize::from_raw(0x0840).is_valid_for_usb2(UsbSpeed::Full, TransferType::Interrupt));
}

#[test]
fn validates_high_speed_packet_sizes() {
    assert!(MaxPacketSize::from_raw(64).is_valid_for_usb2(UsbSpeed::High, TransferType::Control));
    assert!(!MaxPacketSize::from_raw(32).is_valid_for_usb2(UsbSpeed::High, TransferType::Control));

    assert!(MaxPacketSize::from_raw(512).is_valid_for_usb2(UsbSpeed::High, TransferType::Bulk));
    assert!(!MaxPacketSize::from_raw(64).is_valid_for_usb2(UsbSpeed::High, TransferType::Bulk));
    assert!(!MaxPacketSize::from_raw(1024).is_valid_for_usb2(UsbSpeed::High, TransferType::Bulk));

    for transfer_type in [TransferType::Isochronous, TransferType::Interrupt] {
        assert!(MaxPacketSize::from_raw(1).is_valid_for_usb2(UsbSpeed::High, transfer_type));
        assert!(MaxPacketSize::from_raw(1024).is_valid_for_usb2(UsbSpeed::High, transfer_type));
        assert!(MaxPacketSize::from_raw(0x0c00).is_valid_for_usb2(UsbSpeed::High, transfer_type));
        assert!(MaxPacketSize::from_raw(0x1400).is_valid_for_usb2(UsbSpeed::High, transfer_type));
        assert!(!MaxPacketSize::from_raw(1025).is_valid_for_usb2(UsbSpeed::High, transfer_type));
        assert!(!MaxPacketSize::from_raw(0x1c00).is_valid_for_usb2(UsbSpeed::High, transfer_type));
        assert!(!MaxPacketSize::from_raw(0x2400).is_valid_for_usb2(UsbSpeed::High, transfer_type));
    }
    assert!(MaxPacketSize::from_raw(0).is_valid_for_usb2(UsbSpeed::High, TransferType::Isochronous));
    assert!(!MaxPacketSize::from_raw(0x0800).is_valid_for_usb2(UsbSpeed::High, TransferType::Isochronous));
    assert!(!MaxPacketSize::from_raw(0).is_valid_for_usb2(UsbSpeed::High, TransferType::Interrupt));
}

#[test]
fn usb2_validation_rejects_superspeed_semantics() {
    for speed in [UsbSpeed::Super, UsbSpeed::SuperPlus] {
        assert!(!MaxPacketSize::from_raw(512).is_valid_for_usb2(speed, TransferType::Control));
        assert!(!MaxPacketSize::from_raw(1024).is_valid_for_usb2(speed, TransferType::Isochronous));
        assert!(!MaxPacketSize::from_raw(1024).is_valid_for_usb2(speed, TransferType::Bulk));
        assert!(!MaxPacketSize::from_raw(1024).is_valid_for_usb2(speed, TransferType::Interrupt));
    }
}
