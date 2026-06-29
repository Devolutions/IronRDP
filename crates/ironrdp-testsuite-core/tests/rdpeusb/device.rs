use ironrdp_core::encode_vec;
use ironrdp_rdpeusb::client::{
    DeviceInfo, UsbBcdVersion, UsbClassCodes, UsbConfigInfo, UsbConnectionSpeed, UsbDeviceDescriptorInfo,
    UsbDeviceLocation, UsbInterfaceInfo, add_device_from_info,
};
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
