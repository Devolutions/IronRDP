use ironrdp_rdpeusb::client::{
    DeviceInfo, UsbBcdVersion, UsbClassCodes, UsbConfigInfo, UsbConnectionSpeed, UsbDeviceDescriptorInfo,
    UsbDeviceLocation, UsbInterfaceInfo,
};

fn simple_device_info() -> DeviceInfo {
    DeviceInfo {
        location: UsbDeviceLocation {
            bus_number: 7,
            address: 2,
            port_numbers: vec![1, 4],
        },
        descriptor: UsbDeviceDescriptorInfo {
            vendor_id: 0x1234,
            product_id: 0xabcd,
            device_version: UsbBcdVersion::from_bcd(0x0210),
            usb_version: UsbBcdVersion::from_bcd(0x0200),
            class_codes: UsbClassCodes::PER_INTERFACE,
            num_configurations: 1,
        },
        active_config: Some(UsbConfigInfo {
            interfaces: vec![UsbInterfaceInfo {
                class_codes: UsbClassCodes {
                    class_code: 0x03,
                    sub_class_code: 0x01,
                    protocol_code: 0x02,
                },
            }],
        }),
        speed: UsbConnectionSpeed::Unknown,
    }
}

mod client;
mod device;
