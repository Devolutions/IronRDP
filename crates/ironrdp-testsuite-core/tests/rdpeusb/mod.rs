use ironrdp_core::encode_vec;
use ironrdp_rdpeusb::{
    io::device::{
        DeviceInfo, UsbBcdVersion, UsbClassCodes, UsbConfigInfo, UsbConnectionSpeed, UsbDeviceDescriptorInfo,
        UsbDeviceLocation, UsbInterfaceInfo,
    },
    pdu::header::InterfaceId,
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
            device_version: 0x0210,
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

const STREAM_ID_PROXY: u32 = 1;

fn proxy_iface_id(iface: InterfaceId) -> u32 {
    u32::from(iface) | (STREAM_ID_PROXY << 30)
}

fn encode_pdu<T: ironrdp_core::Encode>(pdu: &T) -> Vec<u8> {
    encode_vec(pdu).expect("encode should succeed")
}

mod client;
mod device;
mod io;
mod server;
