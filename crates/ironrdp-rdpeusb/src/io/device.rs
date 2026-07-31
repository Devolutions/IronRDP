//! Backend-neutral USB device facts and the RDPEUSB-specific ADD_DEVICE conversion.
//!
//! Backends should fill [`DeviceInfo`] with raw USB topology/descriptor data. This module is
//! responsible for turning those facts into Windows PnP-style strings and RDPEUSB wire wrappers.
//!
//! The split is intentional:
//! - RDPEUSB defines the ADD_DEVICE fields and their wire types, but not every generation detail.
//! - Windows PnP/USB defines the usual hardware ID and compatibility ID formats used for driver
//!   matching.
//! - Device instance ID and container ID are enumerator policy. They must be stable identifiers
//!   with the RDPEUSB-required shape, so this implementation follows FreeRDP's observed strategy.
//!
//! References:
//! - [MS-RDPEUSB ADD_DEVICE]
//! - [MS-RDPEUSB USB_DEVICE_CAPABILITIES]
//! - [Windows device identification strings]
//! - [Standard USB identifiers]
//! - [USB composite device enumeration]
//! - [USB container ID assignment]
//! - [FreeRDP urbdrc_main.c]
//! - [FreeRDP libusb_udevice.c]
//!
//! [MS-RDPEUSB ADD_DEVICE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a26bcb6d-d45d-48a9-b9bd-22e0107d8393
//! [MS-RDPEUSB USB_DEVICE_CAPABILITIES]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/98d4650e-b6d8-47e5-b71b-4d320ab542ee
//! [Windows device identification strings]: https://learn.microsoft.com/en-us/windows-hardware/drivers/install/device-identification-strings
//! [Standard USB identifiers]: https://learn.microsoft.com/en-us/windows-hardware/drivers/install/standard-usb-identifiers
//! [USB composite device enumeration]: https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/enumeration-of-the-composite-parent-device
//! [USB container ID assignment]: https://learn.microsoft.com/en-us/windows-hardware/drivers/install/how-usb-devices-are-assigned-container-ids
//! [FreeRDP urbdrc_main.c]: https://github.com/FreeRDP/FreeRDP/blob/master/channels/urbdrc/client/urbdrc_main.c
//! [FreeRDP libusb_udevice.c]: https://github.com/FreeRDP/FreeRDP/blob/master/channels/urbdrc/client/libusb/libusb_udevice.c

use alloc::{format, string::String, vec, vec::Vec};

use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_str::multi_sz::MultiSzString;
use ironrdp_str::prefixed::Cch32String;

use crate::pdu::header::{InterfaceId, MessageId};
use crate::pdu::sink::{
    AddDevice, DeviceSpeed, NoAckIsochWriteJitterBufSizeInMs, SupportedUsbVer, UsbBusIfaceVer, UsbDeviceCaps, UsbdiVer,
};

const ADD_DEVICE_MESSAGE_ID: MessageId = 0;
const DEFAULT_NO_ACK_ISOCH_WRITE_JITTER_MS: u32 = 0x50;

const USB_CLASS_PER_INTERFACE: u8 = 0x00;
const USB_CLASS_MISCELLANEOUS: u8 = 0xef;
const USB_SUBCLASS_COMMON: u8 = 0x02;
const USB_PROTOCOL_INTERFACE_ASSOCIATION: u8 = 0x01;

/// USB device facts supplied by a client backend.
///
/// [`UrbdrcDeviceBackend::device_info`] returns this backend-neutral description. The RDPEUSB
/// client uses it to construct the Windows Plug and Play identifiers and capabilities carried by
/// `ADD_DEVICE`.
///
/// [`UrbdrcDeviceBackend::device_info`]: crate::client::UrbdrcDeviceBackend::device_info
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Physical/topological location. Used to derive stable Windows PnP instance/container IDs.
    pub location: UsbDeviceLocation,
    /// Raw fields from the USB device descriptor.
    pub descriptor: UsbDeviceDescriptorInfo,
    /// Active configuration, if the backend can read it. Used for composite detection and
    /// first-interface class codes.
    pub active_config: Option<UsbConfigInfo>,
    /// Backend-observed connection speed. RDPEUSB only carries a high-speed boolean.
    pub speed: UsbConnectionSpeed,
}

impl DeviceInfo {
    fn is_composite(&self) -> bool {
        let descriptor_class = self.descriptor.class_codes;
        // Match FreeRDP/libusb composite detection: either a per-interface class device with
        // multiple interfaces, or an Interface Association Descriptor style device class.
        //
        // Refs: [USB composite device enumeration]; [FreeRDP libusb_udevice.c]
        // `interface_create()`.
        let has_single_config_multiple_interfaces = self.descriptor.num_configurations == 1
            && descriptor_class.class_code == USB_CLASS_PER_INTERFACE
            && self
                .active_config
                .as_ref()
                .is_some_and(|config| config.interfaces.len() > 1);

        let has_interface_association_descriptor = descriptor_class.class_code == USB_CLASS_MISCELLANEOUS
            && descriptor_class.sub_class_code == USB_SUBCLASS_COMMON
            && descriptor_class.protocol_code == USB_PROTOCOL_INTERFACE_ASSOCIATION;

        has_single_config_multiple_interfaces || has_interface_association_descriptor
    }

    fn pnp_class_codes(&self) -> UsbClassCodes {
        // FreeRDP uses the first active interface class for compatibility IDs after checking
        // whether the whole device is composite.
        //
        // Ref: [FreeRDP libusb_udevice.c] `interface_create()`.
        self.active_config
            .as_ref()
            .and_then(|config| config.interfaces.first())
            .map(|interface| interface.class_codes)
            .unwrap_or(self.descriptor.class_codes)
    }
}

/// Physical location of a USB device in the backend's USB topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDeviceLocation {
    pub bus_number: u8,
    pub address: u8,
    pub port_numbers: Vec<u8>,
}

impl UsbDeviceLocation {
    fn path(&self) -> String {
        // FreeRDP uses "bus-last_port" as the device path. Keep the full port chain in
        // DeviceInfo for backend fidelity, but only the last port participates in ADD_DEVICE IDs.
        //
        // Ref: [FreeRDP libusb_udevice.c] `udev_get_device_handle()`.
        let last_port_or_address = self.port_numbers.last().copied().unwrap_or(self.address);

        format!("{}-{last_port_or_address}", self.bus_number)
    }
}

/// Fields from a standard USB device descriptor needed for device announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbDeviceDescriptorInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version: u16,
    pub usb_version: UsbBcdVersion,
    pub class_codes: UsbClassCodes,
    pub num_configurations: u8,
}

/// Information from the device's active USB configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbConfigInfo {
    pub interfaces: Vec<UsbInterfaceInfo>,
}

/// Information from a USB interface descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbInterfaceInfo {
    pub class_codes: UsbClassCodes,
}

/// USB class, subclass, and protocol code triplet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbClassCodes {
    pub class_code: u8,
    pub sub_class_code: u8,
    pub protocol_code: u8,
}

impl UsbClassCodes {
    pub const PER_INTERFACE: Self = Self {
        class_code: 0x00,
        sub_class_code: 0x00,
        protocol_code: 0x00,
    };
}

/// Raw `bcdUSB` value from the USB device descriptor.
///
/// The value is preserved without BCD validation. RDPEUSB supports only
/// USB 1.0, 1.1, and 2.0, so newer values are advertised as USB 2.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbBcdVersion(u16);
impl UsbBcdVersion {
    /// Wraps a raw `bcdUSB` value without validating its BCD digits.
    pub const fn from_bcd(value: u16) -> Self {
        Self(value)
    }

    fn to_supported_usb_version(self) -> SupportedUsbVer {
        if self.0 >= 0x0200 {
            SupportedUsbVer::USB_20
        } else if self.0 >= 0x0110 {
            SupportedUsbVer::USB_11
        } else {
            SupportedUsbVer::USB_10
        }
    }

    fn is_at_least_usb20(self) -> bool {
        self.0 >= 0x0200
    }
}

/// Connection speed reported by the USB backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbConnectionSpeed {
    Unknown,
    Low,
    Full,
    High,
    Super,
    SuperPlus,
}

/// Convert backend USB facts into the RDPEUSB ADD_DEVICE PDU.
///
/// `usb_device` is deliberately passed separately: it is the per-device USB interface ID allocated
/// by the DVC processor, not a property of the USB backend device.
///
/// The output strings are Windows PnP identifiers. They are opaque to this crate once generated;
/// the important part is using the standard USB forms and keeping instance/container values stable.
pub fn add_device_from_info(usb_device: InterfaceId, info: &DeviceInfo) -> PduResult<AddDevice> {
    let device_version = info.descriptor.device_version;
    let location_path = info.location.path();

    Ok(AddDevice {
        msg_id: ADD_DEVICE_MESSAGE_ID,
        usb_device,
        // Cch32String and MultiSzString are wire-format concerns. Keep DeviceInfo plain and build
        // these counted UTF-16 wrappers only at the RDPEUSB boundary.
        //
        // Ref: [MS-RDPEUSB ADD_DEVICE] field definitions for cchDeviceInstanceId, cchHwIds,
        // cchCompatIds, and cchContainerId.
        device_instance_id: Cch32String::new(device_instance_id(&location_path)),
        hw_ids: Some(
            MultiSzString::new(hardware_ids(
                info.descriptor.vendor_id,
                info.descriptor.product_id,
                device_version,
            ))
            .map_err(|e| pdu_other_err!("generated ADD_DEVICE hardware IDs contain an embedded nul", source: e))?,
        ),
        compat_ids: Some(MultiSzString::new(compatibility_ids(info)).map_err(
            |e| pdu_other_err!("generated ADD_DEVICE compatibility IDs contain an embedded nul", source: e),
        )?),
        container_id: Cch32String::new(container_id(
            info.descriptor.vendor_id,
            info.descriptor.product_id,
            &location_path,
        )),
        usb_device_caps: usb_device_caps(info)?,
    })
}

fn hardware_ids(vendor_id: u16, product_id: u16, device_version: u16) -> Vec<String> {
    // Windows PnP hardware IDs, ordered from most specific to less specific.
    //
    // Refs: [Standard USB identifiers]; [FreeRDP urbdrc_main.c]
    // `urdbrc_send_usb_device_add()`.
    vec![
        format!("USB\\VID_{vendor_id:04X}&PID_{product_id:04X}&REV_{device_version:04X}"),
        format!("USB\\VID_{vendor_id:04X}&PID_{product_id:04X}"),
    ]
}

fn compatibility_ids(info: &DeviceInfo) -> Vec<String> {
    if info.is_composite() {
        // Composite devices advertise DevClass_00 plus USB\COMPOSITE, matching FreeRDP.
        //
        // Refs: [USB composite device enumeration]; [FreeRDP urbdrc_main.c]
        // `urdbrc_send_usb_device_add()`.
        vec![
            String::from("USB\\DevClass_00&SubClass_00&Prot_00"),
            String::from("USB\\DevClass_00&SubClass_00"),
            String::from("USB\\DevClass_00"),
            String::from("USB\\COMPOSITE"),
        ]
    } else {
        let codes = info.pnp_class_codes();

        // Non-composite devices advertise class/subclass/protocol in decreasing specificity.
        //
        // Refs: [Standard USB identifiers]; [FreeRDP urbdrc_main.c]
        // `urdbrc_send_usb_device_add()`.
        vec![
            format!(
                "USB\\Class_{:02X}&SubClass_{:02X}&Prot_{:02X}",
                codes.class_code, codes.sub_class_code, codes.protocol_code
            ),
            format!(
                "USB\\Class_{:02X}&SubClass_{:02X}",
                codes.class_code, codes.sub_class_code
            ),
            format!("USB\\Class_{:02X}", codes.class_code),
        ]
    }
}

fn usb_device_caps(info: &DeviceInfo) -> PduResult<UsbDeviceCaps> {
    // These constants mirror FreeRDP's ADD_DEVICE capabilities. The current PDU enum only models
    // USB 1.0/1.1/2.0, so USB 3.x backend versions are reported as Usb20 for this field.
    //
    // Refs: [MS-RDPEUSB USB_DEVICE_CAPABILITIES]; [FreeRDP urbdrc_main.c]
    // `urbdrc_send_add_device()`.
    Ok(UsbDeviceCaps {
        usb_bus_iface_ver: UsbBusIfaceVer::V2,
        usbdi_ver: UsbdiVer::V0X600,
        supported_usb_ver: info.descriptor.usb_version.to_supported_usb_version(),
        device_speed: device_speed(info)?,
        no_ack_isoch_write_jitter_buf_size: NoAckIsochWriteJitterBufSizeInMs::try_from(
            DEFAULT_NO_ACK_ISOCH_WRITE_JITTER_MS,
        )
        .map_err(|_| pdu_other_err!("default isochronous jitter buffer size is invalid"))?,
    })
}

fn device_speed(info: &DeviceInfo) -> PduResult<DeviceSpeed> {
    match info.speed {
        UsbConnectionSpeed::Low | UsbConnectionSpeed::Full => Ok(DeviceSpeed::FULL_SPEED),
        UsbConnectionSpeed::High | UsbConnectionSpeed::Super | UsbConnectionSpeed::SuperPlus => {
            Ok(DeviceSpeed::HIGH_SPEED)
        }
        UsbConnectionSpeed::Unknown => {
            if info.descriptor.usb_version.is_at_least_usb20() {
                Ok(DeviceSpeed::HIGH_SPEED)
            } else {
                Ok(DeviceSpeed::FULL_SPEED)
            }
        }
    }
}

fn device_instance_id(location_path: &str) -> String {
    // FreeRDP formats a zero-padded 16-byte ASCII seed as a GUID-looking instance ID.
    //
    // RDPEUSB only requires a null-terminated Unicode string identifying the USB device instance.
    // Windows device identification strings are opaque string-comparison keys, so this is an
    // enumerator policy choice rather than a USB descriptor field.
    //
    // Refs: [MS-RDPEUSB ADD_DEVICE] DeviceInstanceId; [Windows device identification strings];
    // [FreeRDP urbdrc_main.c] `func_instance_id_generate()`.
    let raw = format!("\\{location_path}");

    guid_from_bytes(bytes16_from_ascii(raw.as_bytes()), false)
}

fn container_id(vendor_id: u16, product_id: u16, location_path: &str) -> String {
    // Container ID uses VID/PID plus the last 8 bytes of the location path, with braces.
    //
    // RDPEUSB requires a non-zero GUID string. Windows uses container IDs to group devnodes that
    // represent the same physical device; without the full Windows USB/ACPI/container descriptor
    // heuristic available on the client side, follow FreeRDP's stable VID/PID/path-derived value.
    //
    // Refs: [MS-RDPEUSB ADD_DEVICE] ContainerId; [USB container ID assignment];
    // [FreeRDP urbdrc_main.c] `func_container_id_generate()`.
    let path_suffix = location_path
        .get(location_path.len().saturating_sub(8)..)
        .expect("location path is ASCII");
    let raw = format!("{vendor_id:04X}{product_id:04X}{path_suffix}");
    guid_from_bytes(bytes16_from_ascii(raw.as_bytes()), true)
}

fn bytes16_from_ascii(value: &[u8]) -> [u8; 16] {
    let mut bytes = [0; 16];
    let copy_len = value.len().min(bytes.len());

    bytes[..copy_len].copy_from_slice(&value[..copy_len]);

    bytes
}

fn guid_from_bytes(bytes: [u8; 16], braces: bool) -> String {
    let guid = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );

    if braces { format!("{{{guid}}}") } else { guid }
}
