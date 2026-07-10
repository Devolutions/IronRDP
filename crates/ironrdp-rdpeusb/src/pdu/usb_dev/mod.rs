//! Messages specific to the [USB Device][1] interface.
//!
//! The USB device interface is used by the server to send IO-related requests to the client.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/034257d7-f7a8-4fe1-b8c2-87ac8dc4f50e

use alloc::format;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode as _, DecodeOwned as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size,
    ensure_size, invalid_field_err, other_err, unsupported_value_err,
};
use ironrdp_dvc::DvcEncode;
use ironrdp_str::prefixed::Cch32String;

use crate::pdu::header::{FunctionId, InterfaceId, Mask, MessageId, SharedMsgHeader};
use crate::pdu::usb_dev::ts_urb::{TsUrbIn, TsUrbInKind, TsUrbOut};
use crate::pdu::utils::{HResult, RequestId, RequestIdIoctl, RequestIdTransferInOut};
#[cfg(doc)]
use crate::pdu::{
    completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData},
    sink::AddDevice,
};

pub mod ts_urb;

/// [\[MS-RDPEUSB\] 2.2.6.1 Cancel Request Message (CANCEL_REQUEST)][1] message.
///
/// Sent from the server to the client to cancel an outstanding IO request.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/93912b05-1fc8-4a43-8abd-78d9aab65d71
#[doc(alias = "CANCEL_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct CancelRequest {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub req_id: RequestId,
}

impl CancelRequest {
    const PAYLOAD_SIZE: usize = 4 /* RequestId */;

    const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_REQ /* Header */ + Self::PAYLOAD_SIZE /* RequestId */;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::CANCEL_REQUEST),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        let req_id = src.read_u32();

        Ok(Self {
            msg_id,
            udev_iface,
            req_id,
        })
    }
}

impl Encode for CancelRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        self.header().encode(dst)?;
        dst.write_u32(self.req_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "CANCEL_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl DvcEncode for CancelRequest {}

/// [\[MS-RDPEUSB\] 2.2.6.2 Register Request Callback Message (REGISTER_REQUEST_CALLBACK)][1] message.
///
/// Sent from the server to the client in order to provide an interface ID for Request Completion
/// to the client. This interface ID is to be used by the subsequent [`IoControlCompletion`],
/// [`UrbCompletion`] and [`UrbCompletionNoData`] messages.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/8693de72-5e87-4b64-a252-101e865311a5
#[doc(alias = "REGISTER_REQUEST_CALLBACK")]
#[derive(Debug, PartialEq, Clone)]
pub struct RegisterRequestCallback {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub request_completion: Option<InterfaceId>,
}

impl RegisterRequestCallback {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::REGISTER_REQUEST_CALLBACK),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* NumRequestCompletion */);
        let request_completion = match src.read_u32() {
            0x0 => None,
            _ => {
                ensure_size!(in: src, size: InterfaceId::FIXED_PART_SIZE);
                match src.read_u32() {
                    0x0..=0x3 => {
                        return Err(invalid_field_err!(
                            "RequestCompletion",
                            "conflict with default interfaces"
                        ));
                    }
                    value => Some(InterfaceId::try_from(value)?),
                }
            }
        };
        Ok(Self {
            msg_id,
            udev_iface,
            request_completion,
        })
    }
}

impl Encode for RegisterRequestCallback {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;
        if let Some(request_completion) = self.request_completion {
            dst.write_u32(0x1);
            dst.write_u32(request_completion.into());
        } else {
            dst.write_u32(0x0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "REGISTER_REQUEST_CALLBACK"
    }

    fn size(&self) -> usize {
        let request_completion_size = if self.request_completion.is_some() { 4 } else { 0 };
        SharedMsgHeader::SIZE_REQ + 4 + request_completion_size
    }
}

impl DvcEncode for RegisterRequestCallback {}

/// [\[MS-RDPEUSB\] 2.2.6.3 IO Control Message (IO_CONTROL)][1] message.
///
/// Sent from the server to the client to submit an IO control request to the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/021733cb-8e3b-49ac-b3e3-f7a764b11141
#[doc(alias = "IO_CONTROL")]
#[derive(Debug, PartialEq, Clone)]
pub struct IoControl {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub ioctl_code: IoctlInternalUsb,
    pub input_buffer: Vec<u8>,
    pub output_buffer_size: u32,
    pub req_id: RequestIdIoctl,
}

impl IoControl {
    /// Minimum payload size, assuming `InputBuffer` is empty.
    pub const PAYLOAD_MIN_SIZE: usize = IoctlInternalUsb::FIXED_PART_SIZE // IoControlCode
        + 4 // InputBufferSize
        + 4 // OutputBufferSize
        + 4; // RequestId

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::IO_CONTROL),
        }
    }

    pub fn check_output_buffer_size(&self) -> Result<(), &'static str> {
        match self.ioctl_code {
            IoctlInternalUsb::ResetPort if self.output_buffer_size != 0 => {
                Err("is not: 0; IO_CONTROL::IoControlCode: IOCTL_INTERNAL_USB_RESET_PORT")
            }
            IoctlInternalUsb::GetPortStatus if self.output_buffer_size != 4 => {
                Err("is not: 4; IO_CONTROL::IoControlCode: IOCTL_INTERNAL_USB_GET_PORT_STATUS")
            }
            IoctlInternalUsb::GetHubCount if self.output_buffer_size != 4 => {
                Err("is not: 4; IO_CONTROL::IoControlCode: IOCTL_INTERNAL_USB_GET_HUB_COUNT")
            }
            IoctlInternalUsb::CyclePort if self.output_buffer_size != 0 => {
                Err("is not: 0; IO_CONTROL::IoControlCode: IOCTL_INTERNAL_USB_CYCLE_PORT")
            }
            // USB_BUS_NOTIFICATION will prolly not really be defined in IronRDP since libusb does
            // not really provide APIs to fill all the fields of a USB_BUS_NOTIFICATION structure.
            // Client should return IOCONTROL_COMPLETION with empty output buffer for
            // IOCTL_INTERNAL_USB_GET_BUS_INFO
            // https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ns-usbioctl-_usb_bus_notification
            IoctlInternalUsb::GetBusInfo if self.output_buffer_size != 16 => Err(
                "is not: 16 (size of USB_BUS_NOTIFICATION); IO_CONTROL::IoControlCode: IOCTL_INTERNAL_USB_GET_BUS_INFO",
            ),
            _ => Ok(()),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_MIN_SIZE);
        let ioctl_code = match src.read_u32() {
            0x220_007 => IoctlInternalUsb::ResetPort,
            0x220_013 => IoctlInternalUsb::GetPortStatus,
            0x220_01B => IoctlInternalUsb::GetHubCount,
            0x220_01F => IoctlInternalUsb::CyclePort,
            0x220_020 => IoctlInternalUsb::GetHubName,
            0x220_420 => IoctlInternalUsb::GetBusInfo,
            0x220_424 => IoctlInternalUsb::GetControllerName,
            value => return Err(unsupported_value_err!("IoControlCode", format!("{value}"))),
        };
        let input_buffer_size = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
        ensure_size!(in: src,
            size: input_buffer_size);
        // TODO: size limit
        let input_buffer = src.read_slice(input_buffer_size).to_vec();
        ensure_size!(in: src, size: 4 /*output buffer size */ + 4 /* request id */);
        let output_buffer_size = src.read_u32();
        let req_id = src.read_u32();
        let io_control = Self {
            msg_id,
            udev_iface,
            ioctl_code,
            input_buffer,
            output_buffer_size,
            req_id,
        };

        io_control
            .check_output_buffer_size()
            .map(|()| io_control)
            .map_err(|reason| invalid_field_err!("IO_CONTROL::OutputBufferSize", reason))
    }
}

impl Encode for IoControl {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.check_output_buffer_size()
            .map_err(|reason| invalid_field_err!("IO_CONTROL::OutputBufferSize", reason))?;
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;

        #[expect(clippy::as_conversions)]
        dst.write_u32(self.ioctl_code as u32);

        dst.write_u32(self.input_buffer.len().try_into().map_err(|e| other_err!(source: e))?); // InputBufferSize
        dst.write_slice(&self.input_buffer);

        dst.write_u32(self.output_buffer_size);
        dst.write_u32(self.req_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "IO_CONTROL"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ + Self::PAYLOAD_MIN_SIZE + self.input_buffer.len()
    }
}

impl DvcEncode for IoControl {}

/// [\[MS-RDPEUSB\] 2.2.12 USB IO Control Code][1]s.
///
/// IO Control Codes are sent as part of an [`IoControl`] request, and these codes specify what
/// operation is requested in the I/O request.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/4f4574f0-9368-4708-8f98-06aa2f44e198
#[repr(u32)]
#[non_exhaustive]
#[doc(alias = "IOCTL_INTERNAL_USB")]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum IoctlInternalUsb {
    /// [\[MS-RDPEUSB\] 2.2.12.1 IOCTL_INTERNAL_USB_RESET_PORT][1].
    ///
    /// Used by a driver to reset the upstream port of the device it manages. For using this IOCTL
    /// with an [`IoControl`] message, `input_buffer` should be empty, and `output_buffer_size`
    /// should be set to `0`. See [WDK: IOCTL_INTERNAL_USB_RESET_PORT][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/8f13c014-2ece-481d-a843-9ae9b03d45fe
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ni-usbioctl-ioctl_internal_usb_reset_port
    #[doc(alias = "IOCTL_INTERNAL_USB_RESET_PORT")]
    ResetPort = 0x220_007,

    /// [\[MS-RDPEUSB\] 2.2.12.2 IOCTL_INTERNAL_USB_GET_PORT_STATUS][1].
    ///
    /// Used to query the status of the device. For using this IOCTL with an [`IoControl`] message,
    /// `input_buffer` should be empty, and `output_buffer_size` should be set to `4`. See [WDK:
    /// IOCTL_INTERNAL_USB_GET_PORT_STATUS][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/598c5366-576d-4fe4-b928-0e1990f88098
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ni-usbioctl-ioctl_internal_usb_get_port_status
    #[doc(alias = "IOCTL_INTERNAL_USB_GET_PORT_STATUS")]
    GetPortStatus = 0x220_013,

    /// [\[MS-RDPEUSB\] 2.2.12.3 IOCTL_INTERNAL_USB_GET_HUB_COUNT][1].
    ///
    /// For using this IOCTL with an [`IoControl`] message, `input_buffer` should be empty, and
    /// `output_buffer_size` should be set to `4`. See [WDK: IOCTL_INTERNAL_USB_GET_HUB_COUNT][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/9ce32995-3886-4c35-8f19-67e6a86a33ca
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ni-usbioctl-ioctl_internal_usb_get_hub_count
    #[doc(alias = "IOCTL_INTERNAL_USB_GET_HUB_COUNT")]
    GetHubCount = 0x220_01B,

    /// [\[MS-RDPEUSB\] 2.2.12.4 IOCTL_INTERNAL_USB_CYCLE_PORT][1].
    ///
    /// Used to simulate a device unplug and replug of the USB device. For using this IOCTL with an
    /// [`IoControl`] message, `input_buffer` should be empty, and `output_buffer_size` should be
    /// set to `0`. See [WDK: IOCTL_INTERNAL_USB_CYCLE_PORT][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5909123b-8a5c-4302-9eab-0bd43419573d
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ni-usbioctl-ioctl_internal_usb_cycle_port
    #[doc(alias = "IOCTL_INTERNAL_USB_CYCLE_PORT")]
    CyclePort = 0x220_01F,

    /// [\[MS-RDPEUSB\] 2.2.12.5 IOCTL_INTERNAL_USB_GET_HUB_NAME][1].
    ///
    /// Used to retrieve the unicode symbolic name for the USB device if the USB device is a hub.
    /// For using this IOCTL with an [`IoControl`] message, `input_buffer` should be empty. See
    /// [WDK: IOCTL_INTERNAL_USB_GET_HUB_NAME][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/09ba1399-d642-4bdb-b9ec-41a4a34c4e98
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ni-usbioctl-ioctl_internal_usb_get_hub_name
    #[doc(alias = "IOCTL_INTERNAL_USB_GET_HUB_NAME")]
    GetHubName = 0x220_020,

    /// [\[MS-RDPEUSB\] 2.2.12.6 IOCTL_INTERNAL_USB_GET_BUS_INFO][1].
    ///
    /// Used to query for certain bus information (the fields of [`USB_BUS_NOTIFICATION`][2]). For
    /// using this IOCTL with an [`IoControl`] message, `input_buffer` should be empty, and
    /// `output_buffer_size` should be set to `16`. See [WDK: IOCTL_INTERNAL_USB_GET_BUS_INFO][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/632e208e-1aea-480d-b600-dfe9a25e05a2
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ns-usbioctl-_usb_bus_notification
    #[doc(alias = "IOCTL_INTERNAL_USB_GET_BUS_INFO")]
    GetBusInfo = 0x220_420,

    /// [\[MS-RDPEUSB\] 2.2.12.7 IOCTL_INTERNAL_USB_GET_CONTROLLER_NAME][1].
    ///
    /// Used to query the device name of the USB host controller. For using this IOCTL with an
    /// [`IoControl`] message, `input_buffer` should be empty. See [WDK:
    /// IOCTL_INTERNAL_USB_GET_CONTROLLER_NAME][2].
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/f6fbc0ba-7736-49c2-a52e-bf538a8d6c15
    /// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbioctl/ni-usbioctl-ioctl_internal_usb_get_controller_name
    #[doc(alias = "IOCTL_INTERNAL_USB_GET_CONTROLLER_NAME")]
    GetControllerName = 0x220_424,
}

impl IoctlInternalUsb {
    pub const FIXED_PART_SIZE: usize = 4 /* IoControlCode */;
}

/// [\[MS-RDPEUSB\] 2.2.13 USB Internal IO Control Code][1].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55d1cd44-eda3-4cba-931c-c3cb8b3c3c92
#[derive(Debug, PartialEq, Clone, Copy)]
#[doc(alias = "IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME")]
pub struct UsbInternalIoctlCode(pub u32);

impl UsbInternalIoctlCode {
    /// [\[MS-RDPEUSB\] 2.2.13.1 IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME][1].
    ///
    /// Sent when the server receives a request its system to query the device's current frame
    /// number (as specified in *USB 2.0 Specification, section 10.2.3 Frame and Microframe
    /// Generation*). To use with an [`InternalIoControl`] message, `input_buffer` should be empty,
    /// and `output_buffer_size` should be set to `4`. This IOCTL is defined only in the context of
    /// \[MS-RDPEUSB\] and not WDK.
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/68506bc9-fedc-4fc1-b826-3cdbb1988774
    #[doc(alias = "IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME")]
    pub const QUERY_BUS_TIME: Self = Self(0x00224000);
}

/// [\[MS-RDPEUSB\] 2.2.6.4 Internal IO Control Message (INTERNAL_IO_CONTROL)][1] message.
///
/// Sent from the server to the client to submit an internal IO control request to the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c3f3e320-336d-4d1b-84c9-51e0ed330ffe
#[doc(alias = "INTERNAL_IO_CONTROL")]
#[derive(Debug, PartialEq, Clone)]
pub struct InternalIoControl {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub ioctl_code: UsbInternalIoctlCode,
    pub input_buffer: Vec<u8>,
    pub output_buffer_size: u32,
    pub req_id: RequestIdIoctl,
}

impl InternalIoControl {
    pub const PAYLOAD_MIN_SIZE: usize = 4 // IoControlCode
        + 4 // InputBufferSize
        + 4 // OutputBufferSize
        + 4; // RequestId

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::INTERNAL_IO_CONTROL),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_MIN_SIZE);

        let code = src.read_u32();

        let size = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
        ensure_size!(in: src, size: size);
        let input_buffer = src.read_slice(size).to_vec();

        ensure_size!(in: src, size: 4 /*output buffer size */ + 4 /* request id */);
        let output_buffer_size = src.read_u32(/* OutputBufferSize */);
        let req_id = src.read_u32();

        Ok(Self {
            msg_id,
            udev_iface,
            ioctl_code: UsbInternalIoctlCode(code),
            input_buffer,
            output_buffer_size,
            req_id,
        })
    }
}

impl Encode for InternalIoControl {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;
        dst.write_u32(self.ioctl_code.0); // IoControlCode
        dst.write_u32(self.input_buffer.len().try_into().map_err(|e| other_err!(source: e))?); // InputBufferSize
        dst.write_slice(&self.input_buffer); // InputBuffer
        dst.write_u32(self.output_buffer_size); // OutputBufferSize
        dst.write_u32(self.req_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "INTERNAL_IO_CONTROL"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ + Self::PAYLOAD_MIN_SIZE + self.input_buffer.len()
    }
}

impl DvcEncode for InternalIoControl {}

/// [\[MS-RDPEUSB\] 2.2.6.5 Query Device Text Message (QUERY_DEVICE_TEXT)][1] message.
///
/// Sent from the server to the client in order to query the USB's device text (like description or
/// location information) when it receives a query device text request
/// ([`IRP_MN_QUERY_DEVICE_TEXT`][2]) from its system.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/d03a7696-2d56-4f20-b7a9-a5e72a045956
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/irp-mn-query-device-text
#[doc(alias = "QUERY_DEVICE_TEXT")]
#[derive(Debug, PartialEq, Clone)]
pub struct QueryDeviceText {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub text_type: u32,
    // TODO: Find out if MS-LCID and USB language ID's are same
    pub locale_id: u32,
}

impl QueryDeviceText {
    pub const PAYLOAD_SIZE: usize = 4 /* TextType */ + 4 /* LocaleId */;

    pub const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_REQ /* Header */ + Self::PAYLOAD_SIZE;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::QUERY_DEVICE_TEXT),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let text_type = src.read_u32();
        let locale_id = src.read_u32();

        Ok(Self {
            msg_id,
            udev_iface,
            text_type,
            locale_id,
        })
    }
}

impl Encode for QueryDeviceText {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header().encode(dst)?;
        dst.write_u32(self.text_type);
        dst.write_u32(self.locale_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "QUERY_DEVICE_TEXT"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl DvcEncode for QueryDeviceText {}

/// [\[MS-RDPEUSB\] 2.2.6.6 Query Device Text Response Message (QUERY_DEVICE_TEXT_RSP)][1] message.
///
/// Sent from the client in response to a [`QueryDeviceText`] message sent by the server.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/acffdcfa-c792-40a4-a8ee-c545ea5b0a38
#[doc(alias = "QUERY_DEVICE_TEXT_RSP")]
#[derive(Debug, PartialEq, Clone)]
pub struct QueryDeviceTextRsp {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub device_description: Cch32String,
    pub hresult: HResult,
}

impl QueryDeviceTextRsp {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Stub),
            msg_id: self.msg_id,
            function_id: None,
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        let device_description = Cch32String::decode_owned(src)?;

        ensure_size!(in: src, size: 4 /* HResult */);
        let hresult = src.read_u32();

        Ok(Self {
            msg_id,
            udev_iface,
            device_description,
            hresult,
        })
    }
}

impl Encode for QueryDeviceTextRsp {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header().encode(dst)?;
        self.device_description.encode(dst)?;

        dst.write_u32(self.hresult);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "QUERY_DEVICE_TEXT_RSP"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_RSP /* Header */
            + self.device_description.size() // cchDeviceDescription + DeviceDescription
            + 4 /* HResult */
    }
}

impl DvcEncode for QueryDeviceTextRsp {}

/// [\[MS-RDPEUSB\] 2.2.6.7 Transfer In Request (TRANSFER_IN_REQUEST)][1] message.
///
/// Sent from the server to the client in order to request data from the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e40f7738-bdd3-480f-a8bb-e1557a83a151
#[doc(alias = "TRANSFER_IN_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TransferInRequest {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub ts_urb: TsUrbIn,
    pub output_buffer_size: u32,
}

impl TransferInRequest {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::TRANSFER_IN_REQUEST),
        }
    }

    pub fn request_id(&self) -> RequestIdTransferInOut {
        self.ts_urb.header.req_id
    }

    pub fn check_output_buffer_size(&self) -> Result<(), &'static str> {
        use TsUrbInKind::*;

        match &self.ts_urb.kind {
            SelectConfig(_) if self.output_buffer_size != 0 => {
                Err("is not: 0; TRANSFER_IN_REQUEST::TsUrb: TS_URB_SELECT_CONFIGURATION")
            }
            SelectIface(_) if self.output_buffer_size != 0 => {
                Err("is not: 0; TRANSFER_IN_REQUEST::TsUrb: TS_URB_SELECT_INTERFACE")
            }
            PipeReq(_) if self.output_buffer_size != 0 => {
                Err("is not: 0; TRANSFER_IN_REQUEST::TsUrb: TS_URB_PIPE_REQUEST")
            }
            GetCurFrameNum(_) if self.output_buffer_size != 0 => {
                Err("is not: 0; TRANSFER_IN_REQUEST::TsUrb: TS_URB_GET_CURRENT_FRAME_NUMBER")
            }
            CtlFeatReq(_) if self.output_buffer_size != 0 => {
                Err("is not: 0; TRANSFER_IN_REQUEST::TsUrb: TS_URB_CONTROL_FEATURE_REQUEST")
            }
            CtlGetStatus(_) if self.output_buffer_size != 2 => {
                Err("is not: 2; TRANSFER_IN_REQUEST::TsUrb: TS_URB_CONTROL_GET_STATUS_REQUEST")
            }
            CtlGetConfig(_) if self.output_buffer_size != 1 => {
                Err("is not: 1; TRANSFER_IN_REQUEST::TsUrb: TS_URB_CONTROL_GET_CONFIGURATION_REQUEST")
            }
            CtlGetIface(_) if self.output_buffer_size != 1 => {
                Err("is not: 1; TRANSFER_IN_REQUEST::TsUrb: TS_URB_CONTROL_GET_INTERFACE_REQUEST")
            }
            // At the time of writing, MS OS Feature Descriptor size can be max 4 * 1024 bytes.
            // But still, can't really enforce any bounds on OutputBufferSize.
            OsFeatDescReq(_) => Ok(()),
            // No bounds whatsoever for all the other TS_URB's
            _ => Ok(()),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* CbTsUrb */);
        let cb_ts_urb = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;

        ensure_size!(in: src, size: cb_ts_urb);
        let ts_urb = TsUrbIn::decode(&mut ReadCursor::new(src.read_slice(cb_ts_urb)))?;

        ensure_size!(in: src, size: 4 /* OutputBufferSize */);
        let output_buffer_size = src.read_u32();

        let transfer_in_req = Self {
            msg_id,
            udev_iface,
            ts_urb,
            output_buffer_size,
        };

        transfer_in_req
            .check_output_buffer_size()
            .map_err(|reason| invalid_field_err!("TRANSFER_IN_REQUEST::OutputBufferSize", reason))?;

        Ok(transfer_in_req)
    }
}

impl Encode for TransferInRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.check_output_buffer_size()
            .map_err(|reason| invalid_field_err!("TRANSFER_IN_REQUEST::OutputBufferSize", reason))?;
        ensure_size!(in: dst, size: self.size());

        self.header().encode(dst)?;
        dst.write_u32(self.ts_urb.size().try_into().map_err(|e| other_err!(source: e))?);
        self.ts_urb.encode(dst)?;
        dst.write_u32(self.output_buffer_size);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TRANSFER_IN_REQUEST"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ /* Header */
            + 4 /* CbTsUrb */
            + self.ts_urb.size() /* TsUrb */
            + 4 /* OutputBufferSize */
    }
}

impl DvcEncode for TransferInRequest {}

/// [\[MS-RDPEUSB\] 2.2.6.8 Transfer Out Request (TRANSFER_OUT_REQUEST)][1] message.
///
/// Sent from the server to the client in order to submit data to the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6d6c85b2-47bb-4674-975a-dc7d8ed684cd
#[doc(alias = "TRANSFER_OUT_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TransferOutRequest {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub ts_urb: TsUrbOut,
    pub output_buffer: Vec<u8>,
}

impl TransferOutRequest {
    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::TRANSFER_OUT_REQUEST),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        let ts_urb = {
            ensure_size!(in: src, size: 4 /* CbTsUrb */);
            let cb_ts_urb = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
            ensure_size!(in: src, size: cb_ts_urb);
            let mut src = ReadCursor::new(src.read_slice(cb_ts_urb));
            TsUrbOut::decode(&mut src)?
        };

        ensure_size!(in: src, size: 4 /* OutputBufferSize */);
        let output_buffer_size = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
        // TODO: limit size
        ensure_size!(in: src, size: output_buffer_size);
        let output_buffer = src.read_slice(output_buffer_size).to_vec();

        Ok(Self {
            msg_id,
            udev_iface,
            ts_urb,
            output_buffer,
        })
    }
}

impl Encode for TransferOutRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header().encode(dst)?;

        dst.write_u32(self.ts_urb.size().try_into().map_err(|e| other_err!(source: e))?);

        self.ts_urb.encode(dst)?;

        dst.write_u32(self.output_buffer.len().try_into().map_err(|e| other_err!(source: e))?);

        dst.write_slice(&self.output_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TRANSFER_OUT_REQUEST"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ /* Header */
            + 4 /* CbTsUrb */
            + self.ts_urb.size() /* TsUrb */
            + 4 /* OutputBufferSize */
            + self.output_buffer.len() /* OutputBuffer */
    }
}

impl DvcEncode for TransferOutRequest {}

/// [\[MS-RDPEUSB\] 2.2.6.9 Retract Device (RETRACT_DEVICE)][1] message.
///
/// Sent from the server to the client in order to stop redirecting the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/92eeb057-9314-48ab-bc37-199d892ebc9f
#[doc(alias = "RETRACT_DEVICE")]
#[derive(Debug, PartialEq, Clone)]
pub struct RetractDevice {
    pub msg_id: MessageId,
    pub udev_iface: InterfaceId,
    pub reason: UsbRetractReason,
}

impl RetractDevice {
    pub const PAYLOAD_SIZE: usize = 4 /* Reason */;

    pub const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_REQ /* Header */ + Self::PAYLOAD_SIZE;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.udev_iface.with_mask(Mask::Proxy),
            msg_id: self.msg_id,
            function_id: Some(FunctionId::RETRACT_DEVICE),
        }
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>, msg_id: MessageId, udev_iface: InterfaceId) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let reason = src.read_u32();
        #[expect(clippy::as_conversions)]
        if reason != UsbRetractReason::BlockedByPolicy as u32 {
            return Err(unsupported_value_err!("RETRACT_DEVICE::Reason", format!("{reason}")));
        }

        Ok(Self {
            msg_id,
            udev_iface,
            reason: UsbRetractReason::BlockedByPolicy,
        })
    }
}

impl Encode for RetractDevice {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header().encode(dst)?;
        #[expect(clippy::as_conversions)]
        dst.write_u32(self.reason as u32);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RETRACT_DEVICE"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.8 USB_RETRACT_REASON Constants][1].
///
/// The reason why the server requests the client to stop redirecting a USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/f3a2ce5e-7c9a-4b0d-b98a-d0241f538b10
#[repr(u32)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum UsbRetractReason {
    /// The USB device is to be stopped from being redirected because the device is blocked by the
    /// server's (group) policy.
    BlockedByPolicy = 0x1,
}

impl DvcEncode for RetractDevice {}
