use ironrdp_core::{DecodeResult, decode, encode_vec};
use ironrdp_rdpeusb::io::device::{
    DeviceInfo, UsbBcdVersion, UsbClassCodes, UsbConfigInfo, UsbConnectionSpeed, UsbDeviceDescriptorInfo,
    UsbDeviceLocation, UsbInterfaceInfo, add_device_from_info,
};
use ironrdp_rdpeusb::pdu::UrbdrcClientDevicePdu;
use ironrdp_rdpeusb::pdu::completion::ts_urb_result::Raw;
use ironrdp_rdpeusb::pdu::header::InterfaceId;
use rstest::rstest;

use super::simple_device_info;

fn composite_device_info() -> DeviceInfo {
    DeviceInfo {
        active_config: Some(UsbConfigInfo {
            interfaces: vec![
                UsbInterfaceInfo {
                    class_codes: UsbClassCodes {
                        class_code: 0x03,
                        sub_class_code: 0x01,
                        protocol_code: 0x02,
                    },
                },
                UsbInterfaceInfo {
                    class_codes: UsbClassCodes {
                        class_code: 0xff,
                        sub_class_code: 0x00,
                        protocol_code: 0x00,
                    },
                },
            ],
        }),
        ..simple_device_info()
    }
}

fn iad_composite_device_info() -> DeviceInfo {
    let mut info = simple_device_info();
    info.descriptor.class_codes = UsbClassCodes {
        class_code: 0xef,
        sub_class_code: 0x02,
        protocol_code: 0x01,
    };
    info
}

fn no_active_config_device_info() -> DeviceInfo {
    DeviceInfo {
        active_config: None,
        descriptor: UsbDeviceDescriptorInfo {
            class_codes: UsbClassCodes {
                class_code: 0x08,
                sub_class_code: 0x06,
                protocol_code: 0x50,
            },
            ..simple_device_info().descriptor
        },
        ..simple_device_info()
    }
}

fn no_port_numbers_device_info() -> DeviceInfo {
    DeviceInfo {
        location: UsbDeviceLocation {
            bus_number: 7,
            address: 2,
            port_numbers: Vec::new(),
        },
        ..simple_device_info()
    }
}

fn usb_version_device_info(usb_version: UsbBcdVersion) -> DeviceInfo {
    DeviceInfo {
        descriptor: UsbDeviceDescriptorInfo {
            usb_version,
            ..simple_device_info().descriptor
        },
        ..simple_device_info()
    }
}

fn speed_device_info(speed: UsbConnectionSpeed) -> DeviceInfo {
    DeviceInfo {
        speed,
        ..simple_device_info()
    }
}

#[rstest]
#[case::simple(simple_device_info())]
#[case::composite_multiple_interfaces(composite_device_info())]
#[case::composite_iad(iad_composite_device_info())]
#[case::no_active_config(no_active_config_device_info())]
#[case::no_port_numbers(no_port_numbers_device_info())]
#[case::usb10(usb_version_device_info(UsbBcdVersion::from_bcd(0x0100)))]
#[case::usb11(usb_version_device_info(UsbBcdVersion::from_bcd(0x0110)))]
#[case::usb20(usb_version_device_info(UsbBcdVersion::from_bcd(0x0200)))]
#[case::low_speed(speed_device_info(UsbConnectionSpeed::Low))]
#[case::full_speed(speed_device_info(UsbConnectionSpeed::Full))]
#[case::high_speed(speed_device_info(UsbConnectionSpeed::High))]
#[case::super_speed(speed_device_info(UsbConnectionSpeed::Super))]
#[case::unknown_speed(speed_device_info(UsbConnectionSpeed::Unknown))]
fn add_device_from_protocol_agnostic_device_info(#[case] info: DeviceInfo) {
    let udev_iface = InterfaceId::try_from(4).expect("valid device interface id");
    let add_device = add_device_from_info(udev_iface, &info).expect("ADD_DEVICE should be generated");

    assert_eq!(add_device.usb_device, udev_iface);
    encode_vec(&add_device).expect("ADD_DEVICE should encode");
}

// A real Windows client (mstsc) assigns `UsbDevice == 0` to a redirected device.
// `InterfaceId` already permits it (0 <= 0x3FFF_FFFF), so ADD_DEVICE must
// round-trip with that id rather than rejecting it as a "default interface".
#[test]
fn add_device_accepts_usb_device_zero() {
    let udev_iface = InterfaceId::try_from(0).expect("interface id 0 is valid");
    let add_device = add_device_from_info(udev_iface, &simple_device_info()).expect("ADD_DEVICE should be generated");
    assert_eq!(add_device.usb_device, udev_iface);

    let encoded = encode_vec(&add_device).expect("ADD_DEVICE should encode");
    let decoded = decode(&encoded).expect("ADD_DEVICE with UsbDevice == 0 should decode");

    let UrbdrcClientDevicePdu::AddDev(decoded) = decoded else {
        panic!("expected an AddDev PDU");
    };
    assert_eq!(decoded.usb_device, udev_iface);
}

// `1..=3` are the RDPEUSB default Device Sink / Channel Notification interface
// IDs; only `0` is the mstsc compatibility exception, so those must stay rejected.
#[rstest]
#[case(1)]
#[case(2)]
#[case(3)]
fn add_device_rejects_reserved_default_interface(#[case] reserved: u32) {
    let iface = InterfaceId::try_from(reserved).expect("interface id is representable");
    let add_device = add_device_from_info(iface, &simple_device_info()).expect("ADD_DEVICE should be generated");
    let encoded = encode_vec(&add_device).expect("ADD_DEVICE should encode");

    let decoded: DecodeResult<UrbdrcClientDevicePdu<Raw>> = decode(&encoded);
    assert!(decoded.is_err(), "UsbDevice == {reserved} must be rejected");
}
