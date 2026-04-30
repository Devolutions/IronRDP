//! Messages specific to the [USB Device][1] interface.
//!
//! The USB device interface is used by the server to send IO-related requests to the client.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/034257d7-f7a8-4fe1-b8c2-87ac8dc4f50e

use alloc::format;
use alloc::vec::Vec;

use ironrdp_core::{
    DecodeError, DecodeOwned as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size,
    ensure_size, invalid_field_err, other_err, unsupported_value_err,
};
use ironrdp_pdu::utils::strict_sum;
use ironrdp_str::prefixed::Cch32String;

use crate::pdu::header::{InterfaceId, SharedMsgHeader};
use crate::pdu::usb_dev::ts_urb::{TransferDirection, TsUrb};
use crate::pdu::utils::{HResult, RequestId, RequestIdIoctl};
#[cfg(doc)]
use crate::pdu::{
    completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData},
    header::{FunctionId, Mask},
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
    pub header: SharedMsgHeader,
    pub req_id: RequestId,
}

impl CancelRequest {
    const PAYLOAD_SIZE: usize = size_of::<RequestId>();

    const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_REQ;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        let req_id = src.read_u32();

        Ok(Self { header, req_id })
    }
}

impl Encode for CancelRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        self.header.encode(dst)?;
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
    pub header: SharedMsgHeader,
    pub request_completion: Option<InterfaceId>,
}

impl RegisterRequestCallback {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>());
        let request_completion = match src.read_u32() {
            0x0 => None,
            0x1 => {
                ensure_size!(in: src, size: InterfaceId::FIXED_PART_SIZE);
                let interface = InterfaceId::try_from(src.read_u32()).map_err(|source| {
                    let e: DecodeError =
                        invalid_field_err!("REGISTER_REQUEST_CALLBACK::RequestCompletion", "more than 30 bits");
                    e.with_source(source)
                })?;
                Some(interface)
            }
            _ => {
                return Err(invalid_field_err!(
                    "REGISTER_REQUEST_CALLBACK::NumRequestCompletion",
                    "is not 0x0 or 0x1"
                ));
            }
        };
        Ok(Self {
            header,
            request_completion,
        })
    }
}

impl Encode for RegisterRequestCallback {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
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
        const NUM_REQUEST_COMPLETION: usize = size_of::<u32>();
        let request_completion = match self.request_completion {
            Some(_) => InterfaceId::FIXED_PART_SIZE,
            None => 0,
        };

        strict_sum(&[SharedMsgHeader::SIZE_REQ + NUM_REQUEST_COMPLETION + request_completion])
    }
}

/// [\[MS-RDPEUSB\] 2.2.6.3 IO Control Message (IO_CONTROL)][1] message.
///
/// Sent from the server to the client to submit an IO control request to the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/021733cb-8e3b-49ac-b3e3-f7a764b11141
#[doc(alias = "IO_CONTROL")]
#[derive(Debug, PartialEq, Clone)]
pub struct IoControl {
    pub header: SharedMsgHeader,
    pub ioctl_code: IoctlInternalUsb,
    /// Should be empty. As of v20240423, all USB IO Control Code's ([MS-RDPEUSB] 2.2.12 USB IO
    /// Control Code) used in the protocol require sending an empty input buffer.
    ///
    /// https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/4f4574f0-9368-4708-8f98-06aa2f44e198
    pub input_buffer: Vec<u8>,
    pub output_buffer_size: u32,
    pub req_id: RequestIdIoctl,
}

impl IoControl {
    #[expect(clippy::identity_op)]
    pub const PAYLOAD_SIZE: usize = IoctlInternalUsb::FIZED_PART_SIZE
        + size_of::<u32>(/* InputBufferSize */)
        + 0 /* InputBuffer */
        + size_of::<u32>(/* OutputBufferSize */)
        + size_of::<u32>(/* RequestId */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_REQ;

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

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
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
        if let size @ 1.. = src.read_u32(/* InputBufferSize */) {
            return Err(unsupported_value_err!(
                "IO_CONTROL::InputBufferSize",
                format!("{size:#X}")
            ));
        }
        let output_buffer_size = src.read_u32();
        let req_id = src.read_u32();
        let io_control = Self {
            header,
            ioctl_code,
            input_buffer: Vec::new(),
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
        ensure_fixed_part_size!(in: dst);
        self.header.encode(dst)?;

        #[expect(clippy::as_conversions)]
        dst.write_u32(self.ioctl_code as u32);

        if !self.input_buffer.is_empty() {
            return Err(invalid_field_err!("IO_CONTROL::InputBuffer", "is not empty"));
        }
        // dst.write_u32(0); // InputBufferSize
        dst.write_u32(self.input_buffer.len().try_into().map_err(|e| other_err!(source: e))?); // InputBufferSize
        dst.write_slice(&self.input_buffer);

        // let output_buffer_size = match self.ioctl_code {
        //     IoctlInternalUsb::ResetPort | IoctlInternalUsb::CyclePort => 0,
        //     IoctlInternalUsb::GetPortStatus | IoctlInternalUsb::GetHubCount => 4,
        //     IoctlInternalUsb::GetHubName | IoctlInternalUsb::GetControllerName => self.output_buffer_size,
        //     IoctlInternalUsb::GetBusInfo => 16,
        // };
        dst.write_u32(self.output_buffer_size);
        dst.write_u32(self.req_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "IO_CONTROL"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

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
    pub const FIZED_PART_SIZE: usize = size_of::<Self>();
}

/// [\[MS-RDPEUSB\] 2.2.13 USB Internal IO Control Code][1].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55d1cd44-eda3-4cba-931c-c3cb8b3c3c92
#[repr(u32)]
#[non_exhaustive]
#[derive(Debug, PartialEq, Clone)]
#[doc(alias = "IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME")]
pub enum UsbInternalIoctlCode {
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
    IoctlTsusbgdIoctlUsbdiQueryBusTime = 0x00224000,
}

const IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME: u32 = 0x00224000;

/// [\[MS-RDPEUSB\] 2.2.6.4 Internal IO Control Message (INTERNAL_IO_CONTROL)][1] message.
///
/// Sent from the server to the client to submit an internal IO control request to the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c3f3e320-336d-4d1b-84c9-51e0ed330ffe
#[doc(alias = "INTERNAL_IO_CONTROL")]
#[derive(Debug, PartialEq, Clone)]
pub struct InternalIoControl {
    pub header: SharedMsgHeader,
    // Should make adding new codes easier.
    pub ioctl_code: UsbInternalIoctlCode,
    /// As of **v20240423**, all codes used for this message require sending an empty input buffer.
    ///
    /// * [MS-RDPEUSB 2.2.13 USB Internal IO Control Code][1]
    ///
    /// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55d1cd44-eda3-4cba-931c-c3cb8b3c3c92
    pub input_buffer: Vec<u8>,
    pub output_buffer_size: u32,
    pub req_id: RequestIdIoctl,
}

impl InternalIoControl {
    #[expect(clippy::identity_op, reason = "for developer documentation purposes?")]
    pub const PAYLOAD_SIZE: usize = size_of::<u32>() // IoControlCode
        + size_of::<u32>(/* InputBufferSize */)
        + 0 // InputBuffer
        + size_of::<u32>(/* OutputBufferSize */)
        + size_of::<RequestIdIoctl>(/* RequestId */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_REQ;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        {
            let code = src.read_u32();
            if code != IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME {
                return Err(unsupported_value_err!(
                    "INTERNAL_IO_CONTROL::IoControlCode",
                    format!("{code:#X}")
                ));
            }
        }
        {
            let size = src.read_u32(/* InputBufferSize */);
            if size != 0 {
                return Err(unsupported_value_err!(
                    "INTERNAL_IO_CONTROL::InputBufferSize",
                    format!("{size:#X}")
                ));
            }
        }
        let output_buffer_size = src.read_u32(/* OutputBufferSize */);
        if output_buffer_size != 0x4 {
            return Err(unsupported_value_err!(
                "INTERNAL_IO_CONTROL::OutputBufferSize",
                format!("{output_buffer_size:#X}")
            ));
        }
        let req_id = src.read_u32();

        Ok(Self {
            header,
            ioctl_code: UsbInternalIoctlCode::IoctlTsusbgdIoctlUsbdiQueryBusTime,
            input_buffer: Vec::new(),
            output_buffer_size,
            req_id,
        })
    }
}

impl Encode for InternalIoControl {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header.encode(dst)?;
        dst.write_u32(IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME); // IoControlCode
        dst.write_u32(0x0); // InputBufferSize
        dst.write_u32(0x4); // OutputBufferSize
        dst.write_u32(self.req_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "INTERNAL_IO_CONTROL"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

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
    pub header: SharedMsgHeader,
    pub text_type: DeviceTextType,
    // TODO: Find out if MS-LCID and USB language ID's are same
    pub locale_id: u32,
}

impl QueryDeviceText {
    pub const PAYLOAD_SIZE: usize = size_of::<DeviceTextType>() + size_of::<u32>(/* LocaleId */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_REQ;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let text_type = match src.read_u32() {
            0 => DeviceTextType::Description,
            1 => DeviceTextType::LocationInformation,
            value => {
                return Err(unsupported_value_err!(
                    "QUERY_DEVICE_TEXT::TextType",
                    format!("{value}")
                ));
            }
        };
        let locale_id = src.read_u32();

        Ok(Self {
            header,
            text_type,
            locale_id,
        })
    }
}

impl Encode for QueryDeviceText {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header.encode(dst)?;
        #[expect(clippy::as_conversions)]
        dst.write_u32(self.text_type as u32);
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

/// Indicates what kind of text/information is to be requested.
#[repr(u32)]
#[doc(alias = "DEVICE_TEXT_TYPE")]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DeviceTextType {
    /// Basic description like manufacturer or product name.
    #[doc(alias = "DeviceTextDescription")]
    Description = 0x0,

    /// Information such as where/what is the device connected to (bus or device number).
    #[doc(alias = "DeviceTextLocationInformation")]
    LocationInformation = 0x1,
}

/// [\[MS-RDPEUSB\] 2.2.6.6 Query Device Text Response Message (QUERY_DEVICE_TEXT_RSP)][1] message.
///
/// Sent from the client in response to a [`QueryDeviceText`] message sent by the server.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/acffdcfa-c792-40a4-a8ee-c545ea5b0a38
#[doc(alias = "QUERY_DEVICE_TEXT_RSP")]
#[derive(Debug, PartialEq, Clone)]
pub struct QueryDeviceTextRsp {
    pub header: SharedMsgHeader,
    pub device_description: Cch32String,
    pub hresult: HResult,
}

impl QueryDeviceTextRsp {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        let device_description = Cch32String::decode_owned(src)?;

        ensure_size!(in: src, size: 4); // HResult
        let hresult = src.read_u32();

        Ok(Self {
            header,
            device_description,
            hresult,
        })
    }
}

impl Encode for QueryDeviceTextRsp {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header.encode(dst)?;
        self.device_description.encode(dst)?;

        dst.write_u32(self.hresult);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "QUERY_DEVICE_TEXT_RSP"
    }

    fn size(&self) -> usize {
        strict_sum(&[SharedMsgHeader::SIZE_RSP + self.device_description.size() + const { size_of::<HResult>() }])
    }
}

// macro_rules! check_output_buffer_size {
//     ($ts_urb:expr, $output_buffer_size:expr) => {{
//     }};
// }

// #[derive(Debug)]
// pub struct TransferInRequestOutputBufferSizeErr {
//     is: u32,
//     expected: u32,
//     ts_urb: &'static str,
// }
//
// impl core::fmt::Display for TransferInRequestOutputBufferSizeErr {
//     fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
//         write!(f, "")
//     }
// }

/// [\[MS-RDPEUSB\] 2.2.6.7 Transfer In Request (TRANSFER_IN_REQUEST)][1] message.
///
/// Sent from the server to the client in order to request data from the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e40f7738-bdd3-480f-a8bb-e1557a83a151
#[doc(alias = "TRANSFER_IN_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TransferInRequest {
    pub header: SharedMsgHeader,
    pub ts_urb: TsUrb,
    pub output_buffer_size: u32,
}

impl TransferInRequest {
    pub fn check_output_buffer_size(&self) -> Result<(), &'static str> {
        use TsUrb::*;

        match self.ts_urb {
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

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4); // CbTsUrb
        let cb_ts_urb = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;

        let ts_urb = TsUrb::decode(&mut ReadCursor::new(src.read_slice(cb_ts_urb)), TransferDirection::In)?;

        ensure_size!(in: src, size: 4);
        let output_buffer_size = src.read_u32();

        let transfer_in_req = Self {
            header,
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

        self.header.encode(dst)?;
        dst.write_u32(self.ts_urb.size().try_into().map_err(|e| other_err!(source: e))?);
        self.ts_urb.encode(dst, TransferDirection::In)?;
        dst.write_u32(self.output_buffer_size);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TRANSFER_IN_REQUEST"
    }

    fn size(&self) -> usize {
        const CB_TS_URB: usize = size_of::<u32>();
        const OUTPUT_BUFFER_SIZE: usize = size_of::<u32>();
        SharedMsgHeader::SIZE_REQ + CB_TS_URB + self.ts_urb.size() + OUTPUT_BUFFER_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.6.8 Transfer Out Request (TRANSFER_OUT_REQUEST)][1] message.
///
/// Sent from the server to the client in order to submit data to the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6d6c85b2-47bb-4674-975a-dc7d8ed684cd
#[doc(alias = "TRANSFER_OUT_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TransferOutRequest {
    pub header: SharedMsgHeader,
    pub ts_urb: TsUrb,
    pub output_buffer: Vec<u8>,
}

impl TransferOutRequest {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        let ts_urb = {
            ensure_size!(in: src, size: 4); // CbTsUrb
            let cb_ts_urb = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
            let mut src = ReadCursor::new(src.read_slice(cb_ts_urb));
            TsUrb::decode(&mut src, TransferDirection::Out)?
        };

        ensure_size!(in: src, size: 4); // OutputBufferSize
        let output_buffer_size = src.read_u32().try_into().map_err(|e| other_err!(source: e))?;
        let output_buffer = src.read_slice(output_buffer_size).to_vec();

        Ok(Self {
            header,
            ts_urb,
            output_buffer,
        })
    }
}

impl Encode for TransferOutRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header.encode(dst)?;

        dst.write_u32(self.ts_urb.size().try_into().map_err(|e| other_err!(source: e))?);

        self.ts_urb.encode(dst, TransferDirection::Out)?;

        dst.write_u32(self.output_buffer.len().try_into().map_err(|e| other_err!(source: e))?);

        dst.write_slice(&self.output_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TRANSFER_OUT_REQUEST"
    }

    fn size(&self) -> usize {
        SharedMsgHeader::SIZE_REQ
            + const {
                size_of::<u32>(/* CbTsUrb */)
            }
            + self.ts_urb.size()
            + const {
                size_of::<u32>(/* OutputBufferSize */)
            }
            + self.output_buffer.len()
    }
}

/// [\[MS-RDPEUSB\] 2.2.6.9 Retract Device (RETRACT_DEVICE)][1] message.
///
/// Sent from the server to the client in order to stop redirecting the USB device.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/92eeb057-9314-48ab-bc37-199d892ebc9f
#[doc(alias = "RETRACT_DEVICE")]
#[derive(Debug, PartialEq, Clone)]
pub struct RetractDevice {
    pub header: SharedMsgHeader,
    pub reason: UsbRetractReason,
}

impl RetractDevice {
    pub const PAYLOAD_SIZE: usize = size_of::<UsbRetractReason>();

    pub const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_REQ + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let reason = src.read_u32();
        #[expect(clippy::as_conversions)]
        if reason != UsbRetractReason::BlockedByPolicy as u32 {
            return Err(unsupported_value_err!("RETRACT_DEVICE::Reason", format!("{reason}")));
        }

        Ok(Self {
            header,
            reason: UsbRetractReason::BlockedByPolicy,
        })
    }
}

impl Encode for RetractDevice {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use ironrdp_core::Decode as _;

    use super::*;
    use crate::pdu::header::FunctionId;
    use crate::pdu::usb_dev::ts_urb::utils::{
        SetupPacket, TsUrbHeader, TsUsbdInterfaceInfo, TsUsbdPipeInfo, UrbFunction, UsbConfigDesc,
    };
    use crate::pdu::usb_dev::ts_urb::{
        TsUrbBulkOrInterruptTransfer, TsUrbControlDescRequest, TsUrbControlFeatRequest, TsUrbControlGetConfigRequest,
        TsUrbControlGetInterfaceRequest, TsUrbControlGetStatusRequest, TsUrbControlTransfer, TsUrbControlTransferEx,
        TsUrbControlVendorClassRequest, TsUrbGetCurrFrameNum, TsUrbIsochTransfer, TsUrbOsFeatDescRequest,
        TsUrbPipeRequest, TsUrbSelectConfig, TsUrbSelectInterface,
    };
    use crate::pdu::utils::{
        RequestIdTransferInOut, USBD_START_ISO_TRANSFER_ASAP, USBD_TRANSFER_DIRECTION_IN, USBD_TRANSFER_DIRECTION_OUT,
        round_trip,
    };

    #[test]
    fn cancel_req() {
        let en = CancelRequest {
            header: SharedMsgHeader {
                interface_id: InterfaceId(123),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 345,
                function_id: Some(FunctionId::CANCEL_REQUEST),
            },
            req_id: 678,
        };
        let de = round_trip!(en, CancelRequest);
        assert_eq!(en, de);
    }

    #[test]
    fn reg_req_cb() {
        let en = RegisterRequestCallback {
            header: SharedMsgHeader {
                interface_id: InterfaceId(234),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 123,
                function_id: Some(FunctionId::REGISTER_REQUEST_CALLBACK),
            },
            request_completion: Some(InterfaceId(765)),
        };
        let de = round_trip!(en, RegisterRequestCallback);
        assert_eq!(en, de);
    }

    #[test]
    fn io_control() {
        let mut en = IoControl {
            header: SharedMsgHeader {
                interface_id: InterfaceId(623),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 675,
                function_id: Some(FunctionId::IO_CONTROL),
            },
            ioctl_code: IoctlInternalUsb::ResetPort,
            input_buffer: vec![],
            output_buffer_size: 0,
            req_id: 78,
        };
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);

        (en.ioctl_code, en.output_buffer_size) = (IoctlInternalUsb::GetPortStatus, 4);
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);

        (en.ioctl_code, en.output_buffer_size) = (IoctlInternalUsb::GetHubCount, 4);
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);

        (en.ioctl_code, en.output_buffer_size) = (IoctlInternalUsb::CyclePort, 0);
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);

        (en.ioctl_code, en.output_buffer_size) = (IoctlInternalUsb::GetHubName, 123123);
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);

        (en.ioctl_code, en.output_buffer_size) = (IoctlInternalUsb::GetBusInfo, 16);
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);

        en.ioctl_code = IoctlInternalUsb::GetControllerName;
        (en.ioctl_code, en.output_buffer_size) = (IoctlInternalUsb::GetControllerName, 53456);
        let de = round_trip!(en, IoControl);
        assert_eq!(en, de);
    }

    #[test]
    fn internal_io_control() {
        let mut en = InternalIoControl {
            header: SharedMsgHeader {
                interface_id: InterfaceId(6754),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 34234,
                function_id: Some(FunctionId::INTERNAL_IO_CONTROL),
            },
            ioctl_code: UsbInternalIoctlCode::IoctlTsusbgdIoctlUsbdiQueryBusTime,
            input_buffer: vec![1, 2, 3],
            output_buffer_size: 1234,
            req_id: 7865,
        };

        let de = round_trip!(en, InternalIoControl);
        (en.input_buffer, en.output_buffer_size) = (vec![], 4);
        assert_eq!(en, de);
    }

    #[test]
    fn query_device_text() {
        let mut en = QueryDeviceText {
            header: SharedMsgHeader {
                interface_id: InterfaceId(234),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 1231,
                function_id: Some(FunctionId::QUERY_DEVICE_TEXT),
            },
            text_type: DeviceTextType::Description,
            locale_id: 8734,
        };
        let de = round_trip!(en, QueryDeviceText);
        assert_eq!(en, de);

        en.text_type = DeviceTextType::LocationInformation;
        let de = round_trip!(en, QueryDeviceText);
        assert_eq!(en, de);
    }

    #[test]
    fn query_device_text_rsp() {
        let en = QueryDeviceTextRsp {
            header: SharedMsgHeader {
                interface_id: InterfaceId(234),
                mask: crate::pdu::header::Mask::StreamIdStub,
                msg_id: 21341,
                function_id: None,
            },
            device_description: Cch32String::new("adasdasd"),
            hresult: 13123,
        };
        let de = round_trip!(en, QueryDeviceTextRsp);
        assert_eq!(en, de);
    }

    #[test]
    fn transfer_in_req() {
        let mut en = TransferInRequest {
            header: SharedMsgHeader {
                interface_id: InterfaceId(234),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 3123,
                function_id: Some(FunctionId::TRANSFER_IN_REQUEST),
            },
            ts_urb: TsUrb::SelectConfig(TsUrbSelectConfig {
                header: TsUrbHeader {
                    func: UrbFunction::SelectConfiguration,
                    req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                    no_ack: false,
                },
                usbd_ifaces: vec![
                    TsUsbdInterfaceInfo {
                        interface_number: 1,
                        alternate_setting: 1,
                        ts_usbd_pipe_info: vec![
                            TsUsbdPipeInfo {
                                max_packet_size: 12,
                                max_transfer_size: 34,
                                pipe_flags: 0,
                            },
                            TsUsbdPipeInfo {
                                max_packet_size: 56,
                                max_transfer_size: 78,
                                pipe_flags: 1,
                            },
                        ],
                    },
                    TsUsbdInterfaceInfo {
                        interface_number: 1,
                        alternate_setting: 2,
                        ts_usbd_pipe_info: vec![
                            TsUsbdPipeInfo {
                                max_packet_size: 13,
                                max_transfer_size: 35,
                                pipe_flags: 0,
                            },
                            TsUsbdPipeInfo {
                                max_packet_size: 57,
                                max_transfer_size: 79,
                                pipe_flags: 1,
                            },
                        ],
                    },
                ],
                desc: Some(UsbConfigDesc {
                    length: 1,
                    descriptor_type: 2,
                    total_length: 3,
                    num_interfaces: 4,
                    configuration_value: 5,
                    configuration: 6,
                    attributes: 7,
                    max_power: 8,
                }),
            }),
            output_buffer_size: 0,
        };
        let de = round_trip!(en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::SelectIface(TsUrbSelectInterface {
            header: TsUrbHeader {
                func: UrbFunction::SelectInterface,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            config_handle: 4,
            usbd_iface: TsUsbdInterfaceInfo {
                interface_number: 1,
                alternate_setting: 2,
                ts_usbd_pipe_info: vec![
                    TsUsbdPipeInfo {
                        max_packet_size: 13,
                        max_transfer_size: 35,
                        pipe_flags: 0,
                    },
                    TsUsbdPipeInfo {
                        max_packet_size: 57,
                        max_transfer_size: 79,
                        pipe_flags: 1,
                    },
                ],
            },
        });
        let de = round_trip!(en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::PipeReq(TsUrbPipeRequest {
            header: TsUrbHeader {
                func: UrbFunction::AbortPipe,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 213,
        });
        let de = round_trip!(en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::GetCurFrameNum(TsUrbGetCurrFrameNum {
            header: TsUrbHeader {
                func: UrbFunction::GetCurrentFrameNumber,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
        });
        let de = round_trip!(en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlTransfer(TsUrbControlTransfer {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
            setup_packet: SetupPacket {
                request_type: 1 << 7,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        });
        en.output_buffer_size = 1024;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
            header: TsUrbHeader {
                func: UrbFunction::BulkOrInterruptTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 13,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
        });
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::IsochTransfer(TsUrbIsochTransfer {
            header: TsUrbHeader {
                func: UrbFunction::IsochTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 23,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN | USBD_START_ISO_TRANSFER_ASAP,
            start_frame: 0,
            error_count: 0,
            iso_packet_offsets: vec![0, 1, 2],
        });
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlDescReq(TsUrbControlDescRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetDescriptorFromDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            index: 2,
            desc_type: 3,
            lang_id: 4,
        });
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlFeatReq(TsUrbControlFeatRequest {
            header: TsUrbHeader {
                func: UrbFunction::SetFeatureToDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            feat_selector: 1,
            index: 2,
        });
        en.output_buffer_size = 0;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlGetStatus(TsUrbControlGetStatusRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetStatusFromDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            index: 234,
        });
        en.output_buffer_size = 2;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::VendorClassReq(TsUrbControlVendorClassRequest {
            header: TsUrbHeader {
                func: UrbFunction::VendorDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
            request: 1,
            value: 2,
            index: 3,
        });
        en.output_buffer_size = 1024;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlGetConfig(TsUrbControlGetConfigRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetConfiguration,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
        });
        en.output_buffer_size = 1;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlGetIface(TsUrbControlGetInterfaceRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetInterface,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            interface: 5,
        });
        en.output_buffer_size = 1;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::OsFeatDescReq(TsUrbOsFeatDescRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetMsFeatureDescriptor,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            recipient: 0,
            interface_number: 0,
            ms_feat_desc_index: 213,
        });
        en.output_buffer_size = 1024;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlTransferEx(TsUrbControlTransferEx {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransferEx,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
            timeout: 12,
            // We only care about transfer direction for tests (bmRequestType D7)
            setup_packet: SetupPacket {
                request_type: 1 << 7,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        });
        en.output_buffer_size = 1024;
        let de = round_trip!(&en, TransferInRequest);
        assert_eq!(en, de);
    }

    #[test]
    fn transfer_out_request() {
        let mut en = TransferOutRequest {
            header: SharedMsgHeader {
                interface_id: InterfaceId(123),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 1312,
                function_id: Some(FunctionId::TRANSFER_OUT_REQUEST),
            },
            ts_urb: TsUrb::CtlTransfer(TsUrbControlTransfer {
                header: TsUrbHeader {
                    func: UrbFunction::ControlTransfer,
                    req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                    no_ack: false,
                },
                pipe: 235,
                transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
                // We only care about transfer direction for tests (bmRequestType D7)
                setup_packet: SetupPacket {
                    request_type: 0,
                    request: 23,
                    value: 76,
                    index: 12,
                    length: 34,
                },
            }),
            output_buffer: vec![1, 2, 3],
        };
        let de = round_trip!(en, TransferOutRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
            header: TsUrbHeader {
                func: UrbFunction::BulkOrInterruptTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 13,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
        });
        let de = round_trip!(en, TransferOutRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::IsochTransfer(TsUrbIsochTransfer {
            header: TsUrbHeader {
                func: UrbFunction::IsochTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 23,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT | USBD_START_ISO_TRANSFER_ASAP,
            start_frame: 0,
            error_count: 0,
            iso_packet_offsets: vec![0, 1, 2],
        });
        let de = round_trip!(en, TransferOutRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlDescReq(TsUrbControlDescRequest {
            header: TsUrbHeader {
                func: UrbFunction::SetDescriptorToDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            index: 2,
            desc_type: 3,
            lang_id: 4,
        });
        let de = round_trip!(en, TransferOutRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::VendorClassReq(TsUrbControlVendorClassRequest {
            header: TsUrbHeader {
                func: UrbFunction::VendorDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
            request: 10,
            value: 11,
            index: 12,
        });
        let de = round_trip!(en, TransferOutRequest);
        assert_eq!(en, de);

        en.ts_urb = TsUrb::CtlTransferEx(TsUrbControlTransferEx {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransferEx,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
            timeout: 234,
            // We only care about transfer direction for tests (bmRequestType D7)
            setup_packet: SetupPacket {
                request_type: 0,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        });
        let de = round_trip!(en, TransferOutRequest);
        assert_eq!(en, de);
    }

    #[test]
    fn retract_device() {
        let en = RetractDevice {
            header: SharedMsgHeader {
                interface_id: InterfaceId(34),
                mask: crate::pdu::header::Mask::StreamIdProxy,
                msg_id: 123412,
                function_id: Some(FunctionId::RETRACT_DEVICE),
            },
            reason: UsbRetractReason::BlockedByPolicy,
        };
        let de = round_trip!(en, RetractDevice);
        assert_eq!(en, de);
    }
}
