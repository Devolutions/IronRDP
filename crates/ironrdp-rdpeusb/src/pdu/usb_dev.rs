//! Messages specific to the [USB Device][1] interface.
//!
//! This interface is used by the client to communicate with the server about new USB devices. Has
//! no default ID, is allotted an interface ID during the lifetime of a USB Redirection Channel.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/034257d7-f7a8-4fe1-b8c2-87ac8dc4f50e

use alloc::format;

use ironrdp_core::{
    DecodeError, DecodeOwned as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size,
    ensure_size, unsupported_value_err,
};
use ironrdp_pdu::utils::strict_sum;
use ironrdp_str::prefixed::Cch32String;

use crate::ensure_payload_size;
use crate::pdu::header::{InterfaceId, SharedMsgHeader};
use crate::pdu::utils::{HResult, RequestId, RequestIdIoctl};

/// The `CANCEL_REQUEST` message is sent from the server to the client to cancel an outstanding IO
/// request.
///
/// * [MS-RDPEUSB § 2.2.6.1 Cancel Request Message (CANCEL_REQUEST)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/93912b05-1fc8-4a43-8abd-78d9aab65d71
#[doc(alias = "CANCEL_REQUEST")]
pub struct CancelRequest {
    /// The `InterfaceId` field **MUST** match the value sent previously in the `UsbDevice` field
    /// of the [`ADD_DEVICE`][1] message. The `Mask` field **MUST** be set to
    /// [`STREAM_ID_PROXY`][2]. The `FunctionId` field **MUST** be set to [`CANCEL_REQUEST`][3].
    ///
    /// [1]: crate::pdu::dev_sink::AddDevice
    /// [2]: crate::pdu::common::Mask::StreamIdProxy
    /// [3]: crate::pdu::common::FunctionId::CANCEL_REQUEST
    pub header: SharedMsgHeader,
    /// Request ID of the oustanding IO request to cancel previously sent via [`IO_CONTROL`],
    /// [`INTERNAL_IO_CONTROL`], [`TRANSFER_IN_REQUEST`], or [`TRANSFER_OUT_REQUEST`] message.
    pub request_id: RequestId,
}

impl CancelRequest {
    const PAYLOAD_SIZE: usize = size_of::<RequestId>();

    const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_NOT_RSP;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);
        let request_id = src.read_u32();

        Ok(Self { header, request_id })
    }
}

impl Encode for CancelRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header.encode(dst)?;
        dst.write_u32(self.request_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "CANCEL_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// The `REGISTER_REQUEST_CALLBACK` message is sent from the server to the client to provide an
/// interface ID for the **Request Completion** interface to the client.
///
/// * [MS-RDPEUSB § 2.2.6.2 Register Request Callback Message (REGISTER_REQUEST_CALLBACK)][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/8693de72-5e87-4b64-a252-101e865311a5
#[doc(alias = "REGISTER_REQUEST_CALLBACK")]
pub struct RegisterRequestCallback {
    /// The `InterfaceId` field **MUST** match the value sent previously in the `UsbDevice` field
    /// of the [`ADD_DEVICE`][1] message. The `Mask` field **MUST** be set to
    /// [`STREAM_ID_PROXY`][2]. The `FunctionId` field **MUST** be set to
    /// [`REGISTER_REQUEST_CALLBACK`][3].
    ///
    /// [1]: crate::pdu::dev_sink::AddDevice
    /// [2]: crate::pdu::common::Mask::StreamIdProxy
    /// [3]: crate::pdu::common::FunctionId::REGISTER_REQUEST_CALLBACK
    pub header: SharedMsgHeader,
    /// A unique `InterfaceID` to be used by all messages defined in the **Request Completion**
    /// interface.
    ///
    /// NOTE: `Interface` **MUST** be the [`NonDefault`][1] variant.
    ///
    /// [1]: crate::pdu::common::Interface::NonDefault
    pub request_completion: Option<InterfaceId>,
}

impl RegisterRequestCallback {
    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>());
        let request_completion = if src.read_u32(/* NumRequestCompletion */) == 0 {
            None
        } else {
            ensure_size!(in: src, size: InterfaceId::FIXED_PART_SIZE);
            let id = src.read_u32();
            Some(InterfaceId::from(id))
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

        strict_sum(&[SharedMsgHeader::SIZE_WHEN_NOT_RSP + NUM_REQUEST_COMPLETION + request_completion])
    }
}

#[repr(u32)]
#[non_exhaustive]
#[doc(alias = "IOCTL_INTERNAL_USB")]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum UsbIoctlCode {
    /// `IOCTL_INTERNAL_USB_RESET_PORT` I/O control request. Used by a driver to reset the
    /// upstream port of the device it manages.
    #[doc(alias = "IOCTL_INTERNAL_USB_RESET_PORT")]
    ResetPort = 0x220_007,

    #[doc(alias = "IOCTL_INTERNAL_USB_GET_PORT_STATUS")]
    GetPortStatus = 0x220_013,

    #[doc(alias = "IOCTL_INTERNAL_USB_GET_HUB_COUNT")]
    GetHubCount = 0x220_01B,

    #[doc(alias = "IOCTL_INTERNAL_USB_CYCLE_PORT")]
    CyclePort = 0x220_01F,

    #[doc(alias = "IOCTL_INTERNAL_USB_GET_HUB_NAME")]
    GetHubName = 0x220_020,

    #[doc(alias = "IOCTL_INTERNAL_USB_GET_BUS_INFO")]
    GetBusInfo = 0x220_420,

    #[doc(alias = "IOCTL_INTERNAL_USB_GET_CONTROLLER_NAME")]
    GetControllerName = 0x220_424,
}

impl UsbIoctlCode {
    pub const FIZED_PART_SIZE: usize = size_of::<Self>();
}

impl TryFrom<u32> for UsbIoctlCode {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        use UsbIoctlCode::*;

        match value {
            0x220_007 => Ok(ResetPort),
            0x220_013 => Ok(GetPortStatus),
            0x220_01B => Ok(GetHubCount),
            0x220_01F => Ok(CyclePort),
            0x220_020 => Ok(GetHubName),
            0x220_420 => Ok(GetBusInfo),
            0x220_424 => Ok(GetControllerName),
            value => Err(unsupported_value_err!(
                "IoControlCode",
                format!(
                    "is: {value}; is not one of: \
IOCTL_INTERNAL_USB_RESET_PORT (0x00220007), \
IOCTL_INTERNAL_USB_GET_PORT_STATUS (0x00220013) \
IOCTL_INTERNAL_USB_GET_HUB_COUNT (0x0022001B) \
IOCTL_INTERNAL_USB_CYCLE_PORT (0x0022001F) \
IOCTL_INTERNAL_USB_GET_HUB_NAME (0x00220020) \
IOCTL_INTERNAL_USB_GET_BUS_INFO (0x00220420) \
IOCTL_INTERNAL_USB_GET_CONTROLLER_NAME (0x00220424)",
                )
            )),
        }
    }
}

#[doc(alias = "IO_CONTROL")]
pub struct IoCtl {
    pub header: SharedMsgHeader,
    pub ioctl_code: UsbIoctlCode,
    // As of v20240423, all USB IO Control Code's ([MS-RDPEUSB] 2.2.12 USB IO Control Code) used
    // in the protocol require setting input_buffer_size = 0
    //
    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/4f4574f0-9368-4708-8f98-06aa2f44e198
    // pub input_buffer_size: u32,
    // pub input_buffer: Vec<u8>,
    pub output_buffer_size: u32,
    pub request_id: RequestIdIoctl,
}

impl IoCtl {
    #[expect(clippy::identity_op, reason = "for developer documentation purposes?")]
    pub const PAYLOAD_SIZE: usize = UsbIoctlCode::FIZED_PART_SIZE
        + size_of::<u32>(/* InputBufferSize */)
        + 0 /* InputBuffer */
        + size_of::<u32>(/* OutputBufferSize */)
        + size_of::<RequestIdIoctl>(/* RequestId */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_NOT_RSP;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);

        let ioctl_code = UsbIoctlCode::try_from(src.read_u32())?;

        if let size @ 1.. = src.read_u32(/* InputBufferSize */) {
            return Err(unsupported_value_err!(
                "IO_CONTROL::InputBufferSize",
                format!("is: {size:#X}; should be: 0x0")
            ));
        }

        let output_buffer_size = {
            let size = src.read_u32();

            const NAME: &str = "IO_CONTROL::OutputBufferSize";

            use UsbIoctlCode::*;
            match ioctl_code {
                ResetPort | CyclePort if size != 0x0 => {
                    return Err(unsupported_value_err!(NAME, format!("is: {size:#X}; should be: 0x0")));
                }
                GetPortStatus | GetHubCount if size != 0x4 => {
                    return Err(unsupported_value_err!(NAME, format!("is: {size:#X}; should be: 0x4")));
                }
                _ => size,
            }
        };

        let request_id = src.read_u32();

        Ok(Self {
            header,
            ioctl_code,
            output_buffer_size,
            request_id,
        })
    }
}

impl Encode for IoCtl {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        self.header.encode(dst)?;

        #[expect(clippy::as_conversions)]
        dst.write_u32(self.ioctl_code as u32);

        dst.write_u32(0x0); // InputBufferSize
        // dst.write_slice(Vec::from(...); // since InputBufferSize = 0x0

        dst.write_u32(self.output_buffer_size);
        dst.write_u32(self.request_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "IO_CONTROL"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

// #[repr(u32)]
// #[non_exhaustive]
// #[doc(alias = "IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME")]
// #[derive(Debug, PartialEq, Eq, Clone, Copy)]
// pub enum UsbInternalIoctlCode {
//     #[doc(alias = "IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME")]
//     IoctlTsusbgdIoctlUsbdiQueryBusTime = 0x00224000,
// }

const IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME: u32 = 0x00224000;

#[doc(alias = "INTERNAL_IO_CONTROL")]
pub struct InternalIoCtl {
    pub header: SharedMsgHeader,
    // As of v20240423, only USB Internal IO Control Code ([MS-RDPEUSB] 2.2.13 USB Internal IO
    // Control Code) used in the protocol is IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME (0x00224000)
    //
    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55d1cd44-eda3-4cba-931c-c3cb8b3c3c92
    // pub ioctl_code: UsbInternalIoctlCode,
    //
    // As of v20240423, all USB Internal IO Control Code's ([MS-RDPEUSB] 2.2.13 USB Internal IO
    // Control Code) used in the protocol require setting input_buffer_size = 0
    //
    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55d1cd44-eda3-4cba-931c-c3cb8b3c3c92
    // pub input_buffer_size: u32,
    // pub input_buffer: Vec<u8>,
    //
    // As of v20240423, all USB Internal IO Control Code's ([MS-RDPEUSB] 2.2.13 USB Internal IO
    // Control Code) used in the protocol require setting output_buffer_size = 0x4
    //
    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55d1cd44-eda3-4cba-931c-c3cb8b3c3c92
    // pub output_buffer_size: u32,
    pub request_id: RequestIdIoctl,
}

impl InternalIoCtl {
    #[expect(clippy::identity_op, reason = "for developer documentation purposes?")]
    pub const PAYLOAD_SIZE: usize = size_of::<u32>() // IoControlCode
        + size_of::<u32>(/* InputBufferSize */)
        + 0 // InputBuffer
        + size_of::<u32>(/* OutputBufferSize */)
        + size_of::<RequestIdIoctl>(/* RequestId */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_NOT_RSP;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);

        {
            let code = src.read_u32();
            if code != IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME {
                return Err(unsupported_value_err!(
                    "INTERNAL_IO_CONTROL::IoControlCode",
                    format!("is: {code:#X}; should be: {IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME:#X}")
                ));
            }
        }
        {
            let size = src.read_u32(/* InputBufferSize */);
            if size != 0x0 {
                return Err(unsupported_value_err!(
                    "INTERNAL_IO_CONTROL::InputBufferSize",
                    format!("is: {size:#X}; should be: 0x0")
                ));
            }
        }
        {
            let size = src.read_u32(/* OutputBufferSize */);
            if size != 0x4 {
                return Err(unsupported_value_err!(
                    "INTERNAL_IO_CONTROL::InputBufferSize",
                    format!("is: {size:#X}; should be: 0x4")
                ));
            }
        }
        let request_id = src.read_u32();

        Ok(Self { header, request_id })
    }
}

impl Encode for InternalIoCtl {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.header.encode(dst)?;
        dst.write_u32(IOCTL_TSUSBGD_IOCTL_USBDI_QUERY_BUS_TIME); // IoControlCode
        dst.write_u32(0x0); // InputBufferSize
        dst.write_u32(0x4); // OutputBufferSize
        dst.write_u32(self.request_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "INTERNAL_IO_CONTROL"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

#[doc(alias = "QUERY_DEVICE_TEXT")]
pub struct QueryDeviceText {
    pub header: SharedMsgHeader,
    // NOTE: TextType and LocaleId fields aren't just merely "numbers", they can be made into an
    // enum ([1]) and struct ([2]) respectively.
    //
    // But QUERY_DEVICE_TEXT is just a "bridge" for IRP_MN_QUERY_DEVICE_TEXT ([3]) sent by the USB
    // driver stack on the server side. For the server, these don't *need* to mean anything more
    // than "just mere numbers". At the client side, the client just needs to hand these off to the
    // USB host controller.
    //
    // [3]: https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/irp-mn-query-device-text
    // [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ne-wdm-device_text_type
    // [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-lcid/70feba9f-294e-491e-b6eb-56532684c37f
    pub text_type: u32,
    pub locale_id: u32,
}

impl QueryDeviceText {
    pub const PAYLOAD_SIZE: usize = size_of::<u32>(/* TextType */) + size_of::<u32>(/* LocaleId */);

    pub const FIXED_PART_SIZE: usize = Self::PAYLOAD_SIZE + SharedMsgHeader::SIZE_WHEN_NOT_RSP;

    pub fn decode(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_payload_size!(in: src);

        let text_type = src.read_u32();
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

#[doc(alias = "QUERY_DEVICE_TEXT_RSP")]
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
        strict_sum(&[SharedMsgHeader::SIZE_WHEN_RSP + self.device_description.size() + const { size_of::<HResult>() }])
    }
}
