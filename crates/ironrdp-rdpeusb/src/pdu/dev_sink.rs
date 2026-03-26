//! PDU's specific to the [Device Sink][1] interface.
//!
//! Identified by the default interface ID `0x00000001`, this interface is used by the client to
//! communicate with the server about new USB devices.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a9a8add7-4e99-4697-abd0-ad64c80c788d

use alloc::borrow::ToOwned as _;
use alloc::string::ToString as _;

use ironrdp_core::{
    Decode, DecodeError, DecodeOwned as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor,
    ensure_fixed_part_size, ensure_size, unsupported_value_err,
};
use ironrdp_pdu::utils::strict_sum;
use ironrdp_str::multi_sz::MultiSzString;
use ironrdp_str::prefixed::Cch32String;

use crate::pdu::header::{InterfaceId, SharedMsgHeader};

/// Specs: [MS-RDPEUSB § 2.2.1 SHARED_MSG_HEADER][1]
#[doc(alias = "ADD_VIRTUAL_CHANNEL")]
pub struct AddVirtualChannel {
    pub header: SharedMsgHeader,
}

impl AddVirtualChannel {
    pub const FIZED_PART_SIZE: usize = SharedMsgHeader::SIZE_WHEN_NOT_RSP;
}

impl Encode for AddVirtualChannel {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.header.encode(dst)
    }

    fn name(&self) -> &'static str {
        "ADD_VIRTUAL_CHANNEL"
    }

    fn size(&self) -> usize {
        Self::FIZED_PART_SIZE
    }
}

#[doc(alias = "USB_DEVICE_CAPABILITIES")]
pub struct UsbDeviceCaps {
    pub usb_bus_iface_ver: UsbBusIfaceVer,
    pub usbdi_ver: UsbdiVer,
    pub supported_usb_ver: SupportedUsbVer,
    pub device_speed: DeviceSpeed,
    pub no_ack_isoch_write_jitter_buf_size: NoAckIsochWriteJitterBufSizeInMs,
}

impl UsbDeviceCaps {
    pub const CB_SIZE: u32 = 28;

    pub const HCD_CAPS: u32 = 0;

    #[expect(clippy::as_conversions)]
    pub const FIXED_PART_SIZE: usize = Self::CB_SIZE as usize;
}

impl Encode for UsbDeviceCaps {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        #[expect(clippy::as_conversions)]
        {
            dst.write_u32(Self::CB_SIZE);
            dst.write_u32(self.usb_bus_iface_ver as u32);
            dst.write_u32(self.usbdi_ver as u32);
            dst.write_u32(self.supported_usb_ver as u32);
            dst.write_u32(Self::HCD_CAPS);
            dst.write_u32(self.device_speed as u32);
        }
        dst.write_u32(self.no_ack_isoch_write_jitter_buf_size.0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "USB_DEVICE_CAPABILITIES"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for UsbDeviceCaps {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        if src.read_u32() != Self::CB_SIZE {
            return Err(unsupported_value_err!("CbSize", "is not: 28".to_owned()));
        }
        let usb_bus_iface_ver = match src.read_u32() {
            0x0 => UsbBusIfaceVer::V0,
            0x1 => UsbBusIfaceVer::V1,
            0x2 => UsbBusIfaceVer::V2,
            _ => {
                return Err(unsupported_value_err!(
                    "UsbBusInterfaceVersion",
                    "is not one of: 0x0, 0x1, 0x2".to_owned()
                ));
            }
        };
        let usbdi_ver = match src.read_u32() {
            0x500 => UsbdiVer::V0x500,
            0x600 => UsbdiVer::V0x600,
            _ => {
                return Err(unsupported_value_err!(
                    "USBDI_Version",
                    "is not one of: 0x500, 0x600".to_owned()
                ));
            }
        };
        let supported_usb_ver = match src.read_u32() {
            0x100 => SupportedUsbVer::Usb10,
            0x110 => SupportedUsbVer::Usb11,
            0x200 => SupportedUsbVer::Usb20,
            _ => {
                return Err(unsupported_value_err!(
                    "SupportedUsbVersion",
                    "is not one of: 0x100, 0x110, 0x200".to_owned()
                ));
            }
        };
        if src.read_u32() != Self::HCD_CAPS {
            return Err(unsupported_value_err!("HcdCapabilities", "is not: 0x0".to_owned()));
        }
        let device_speed = match src.read_u32() {
            0x0 => DeviceSpeed::FullSpeed,
            0x1 => DeviceSpeed::HighSpeed,
            _ => {
                return Err(unsupported_value_err!(
                    "DeviceIsHighSpeed",
                    "is not one of: 0x0, 0x1".to_owned()
                ));
            }
        };
        let no_ack_isoch_write_jitter_buf_size = match src.read_u32() {
            0 => NoAckIsochWriteJitterBufSizeInMs::TS_URB_ISOCH_TRANSER_NOT_SUPPORTED,
            value @ 10..=512 => NoAckIsochWriteJitterBufSizeInMs(value),
            _ => {
                return Err(unsupported_value_err!(
                    "NoAckIsochWriteJitterBufferSizeInMs",
                    "is not: 0, or in the range 10..=512".to_owned()
                ));
            }
        };

        Ok(Self {
            usb_bus_iface_ver,
            usbdi_ver,
            supported_usb_ver,
            device_speed,
            no_ack_isoch_write_jitter_buf_size,
        })
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum UsbBusIfaceVer {
    V0 = 0x0,
    V1 = 0x1,
    V2 = 0x2,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum UsbdiVer {
    V0x500 = 0x500,
    V0x600 = 0x600,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum SupportedUsbVer {
    Usb10 = 0x100,
    Usb11 = 0x110,
    Usb20 = 0x200,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum DeviceSpeed {
    FullSpeed = 0x0,
    HighSpeed = 0x1,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct NoAckIsochWriteJitterBufSizeInMs(u32);

impl NoAckIsochWriteJitterBufSizeInMs {
    const TS_URB_ISOCH_TRANSER_NOT_SUPPORTED: Self = Self(0);

    pub fn outstanding_isoch_data(&self) -> Option<u32> {
        (self.0 != 0).then_some(self.0)
    }
}

impl TryFrom<u32> for NoAckIsochWriteJitterBufSizeInMs {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::TS_URB_ISOCH_TRANSER_NOT_SUPPORTED),
            10..=512 => Ok(Self(value)),
            value => Err(unsupported_value_err!(
                "NoAckIsochWriteJitterBufferSizeInMs",
                value.to_string()
            )),
        }
    }
}

#[doc(alias = "ADD_DEVICE")]
pub struct AddDevice {
    pub header: SharedMsgHeader,
    /// The (unique) interface ID to be used by request messages in the [USB Devices][1] interface.
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/034257d7-f7a8-4fe1-b8c2-87ac8dc4f50e
    pub usb_device: InterfaceId,
    pub device_instance_id: Cch32String,
    pub hw_ids: Option<MultiSzString>,
    pub compat_ids: Option<MultiSzString>,
    pub container_id: Cch32String,
    pub usb_device_caps: UsbDeviceCaps,
}

impl AddDevice {
    pub const NUM_USB_DEVICE: u32 = 0x1;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4); // NumUsbDevice
        if src.read_u32() != 0x1 {
            return Err(unsupported_value_err!("NumUsbDevice", "is not: 0x1".to_owned()));
        }

        ensure_size!(in: src, size: InterfaceId::FIXED_PART_SIZE);
        let usb_device = match src.read_u32() {
            0x0..0x4 => {
                return Err(unsupported_value_err!(
                    "UsbDevice",
                    "is one of: 0x0, 0x1, 0x2, 0x3 (default interface ID's)".to_owned()
                ));
            }
            value @ 0x4..=0x3F_FF_FF_FF => InterfaceId::from(value),
            0x40_00_00_00.. => {
                return Err(unsupported_value_err!(
                    "UsbDevice",
                    "is greater than: 0x3F_FF_FF_FF (more than 30 bits)".to_owned()
                ));
            }
        };
        let device_instance_id = Cch32String::decode_owned(src)?;
        ensure_size!(in: src, size: 4); // cchHwIds
        let hw_ids = if src.peek_u32() != 0 {
            Some(MultiSzString::decode_owned(src)?)
        } else {
            let _ = src.read_u32(); // skip cchHwIds
            None
        };
        ensure_size!(in: src, size: 4); // cchCompatIds
        let compat_ids = if src.peek_u32() != 0 {
            Some(MultiSzString::decode_owned(src)?)
        } else {
            let _ = src.read_u32(); // skip cchCompatIds
            None
        };
        let container_id = Cch32String::decode_owned(src)?;
        let usb_device_caps = UsbDeviceCaps::decode(src)?;

        Ok(Self {
            header,
            usb_device,
            device_instance_id,
            hw_ids,
            compat_ids,
            container_id,
            usb_device_caps,
        })
    }
}

impl Encode for AddDevice {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header.encode(dst)?;
        dst.write_u32(Self::NUM_USB_DEVICE);
        dst.write_u32(self.usb_device.into());
        self.device_instance_id.encode(dst)?;
        match &self.hw_ids {
            Some(ids) => ids.encode(dst)?,
            None => dst.write_u32(0x0),
        };
        match &self.compat_ids {
            Some(ids) => ids.encode(dst)?,
            None => dst.write_u32(0x0),
        };
        self.container_id.encode(dst)?;
        self.usb_device_caps.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "ADD_DEVICE"
    }

    fn size(&self) -> usize {
        let device_instance_id = self.device_instance_id.size();
        let hw_ids = match &self.hw_ids {
            Some(hardware_ids) => hardware_ids.size(),
            None => const { size_of::<u32>() }, // cchHwIds
        };
        let compat_ids = match &self.compat_ids {
            Some(compatibility_ids) => compatibility_ids.size(),
            None => const { size_of::<u32>() }, // cchCompatIds
        };
        let container_id = self.container_id.size();

        strict_sum(&[SharedMsgHeader::SIZE_WHEN_NOT_RSP
            + 4 // NumUsbDevice
            + InterfaceId::FIXED_PART_SIZE // UsbDevice
            + device_instance_id
            + hw_ids
            + compat_ids
            + container_id
            + UsbDeviceCaps::FIXED_PART_SIZE])
    }
}
