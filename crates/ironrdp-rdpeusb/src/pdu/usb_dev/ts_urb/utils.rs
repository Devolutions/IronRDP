//! Contains valid URB Functions, the common header [`TsUrbHeader`] for all [`TsUrb`] structures,
//! and utility data types.

use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size,
    invalid_field_err, other_err, read_padding, write_padding,
};

use crate::pdu::utils::RequestIdTransferInOut;
#[cfg(doc)]
use crate::pdu::{
    header::SharedMsgHeader,
    usb_dev::ts_urb::{
        TsUrb, TsUrbBulkOrInterruptTransfer, TsUrbControlDescRequest, TsUrbControlFeatRequest,
        TsUrbControlGetConfigRequest, TsUrbControlGetInterfaceRequest, TsUrbControlGetStatusRequest,
        TsUrbControlTransfer, TsUrbControlTransferEx, TsUrbControlVendorClassRequest, TsUrbGetCurrFrameNum,
        TsUrbIsochTransfer, TsUrbOsFeatDescRequest, TsUrbPipeRequest, TsUrbSelectConfig, TsUrbSelectInterface,
    },
};

/// Numeric code that indicates the requested operation for a [USB Request Block][1].
///
/// URB Function codes are used with [`TsUrbHeader`]s. This code indicates to an RDP client which
/// `TS_URB` structure the header is used with. See [`URB_HEADER`][2] for valid URB Function codes
/// and what they indicate.
///
/// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/communicating-with-a-usb-device
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header
//
// NOTE: There are a few variants for Memory Descriptor Lists (MDL). Should a client just behave
// like it did not receive any of the MDL variants? Cause the client receives the data buffer over
// the network, so MDL's don't really make a point. [EDIT] Same behavior for MDL and non-MDL
// variants.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct UrbFunction(u16);

impl UrbFunction {
    /// Represents [`URB_FUNCTION_SELECT_CONFIGURATION`][1]. Used with [`TsUrbSelectConfig`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_select_configuration
    pub const URB_FUNCTION_SELECT_CONFIGURATION: Self = Self(0);

    /// Represents [`URB_FUNCTION_SELECT_INTERFACE`][1]. Used with [`TsUrbSelectInterface`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_select_interface
    pub const URB_FUNCTION_SELECT_INTERFACE: Self = Self(1);

    /// Represents [`URB_FUNCTION_ABORT_PIPE`][1]. Used with [`TsUrbPipeRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_abort_pipe
    pub const URB_FUNCTION_ABORT_PIPE: Self = Self(2);

    /// Represents [`URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL`][1]. Used with [`TsUrbPipeRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_sync_reset_pipe_and_clear_stall
    pub const URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL: Self = Self(30);

    /// Represents [`URB_FUNCTION_SYNC_RESET_PIPE`][1]. Used with [`TsUrbPipeRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_sync_reset_pipe
    pub const URB_FUNCTION_SYNC_RESET_PIPE: Self = Self(48);

    /// Represents [`URB_FUNCTION_SYNC_CLEAR_STALL`][1]. Used with [`TsUrbPipeRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_sync_clear_stall
    pub const URB_FUNCTION_SYNC_CLEAR_STALL: Self = Self(49);

    /// Represents [`URB_FUNCTION_CLOSE_STATIC_STREAMS`][1]. Used with [`TsUrbPipeRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_close_static_streams
    pub const URB_FUNCTION_CLOSE_STATIC_STREAMS: Self = Self(54);

    /// Represents [`URB_FUNCTION_GET_CURRENT_FRAME_NUMBER`][1]. Used with [`TsUrbGetCurrFrameNum`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_current_frame_number
    pub const URB_FUNCTION_GET_CURRENT_FRAME_NUMBER: Self = Self(7);

    /// Represents [`URB_FUNCTION_CONTROL_TRANSFER`][1]. Used with [`TsUrbControlTransfer`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_control_transfer
    pub const URB_FUNCTION_CONTROL_TRANSFER: Self = Self(8);

    /// Represents [`URB_FUNCTION_CONTROL_TRANSFER_EX`][1]. Used with [`TsUrbControlTransferEx`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_control_transfer_ex
    pub const URB_FUNCTION_CONTROL_TRANSFER_EX: Self = Self(50);

    /// Represents [`URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER`][1]. Used with
    /// [`TsUrbBulkOrInterruptTransfer`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_bulk_or_interrupt_transfer
    pub const URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER: Self = Self(9);

    /// Represents [`URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER_USING_CHAINED_MDL`][1]. Used with
    /// [`TsUrbBulkOrInterruptTransfer`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_bulk_or_interrupt_transfer_using_chained_mdl
    pub const URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER_USING_CHAINED_MDL: Self = Self(55);

    /// Represents [`URB_FUNCTION_ISOCH_TRANSFER`][1]. Used with [`TsUrbIsochTransfer`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_isoch_transfer
    pub const URB_FUNCTION_ISOCH_TRANSFER: Self = Self(10);

    /// Represents [`URB_FUNCTION_ISOCH_TRANSFER_USING_CHAINED_MDL`][1]. Used with
    /// [`TsUrbIsochTransfer`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_isoch_transfer_using_chained_mdl
    pub const URB_FUNCTION_ISOCH_TRANSFER_USING_CHAINED_MDL: Self = Self(56);

    /// Represents [`URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE`][1]. Used with
    /// [`TsUrbControlDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_descriptor_from_device
    pub const URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE: Self = Self(11);

    /// Represents [`URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT`][1]. Used with
    /// [`TsUrbControlDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_descriptor_from_endpoint
    pub const URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT: Self = Self(36);

    /// Represents [`URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE`][1]. Used with
    /// [`TsUrbControlDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_descriptor_from_interface
    pub const URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE: Self = Self(40);

    /// Represents [`URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE`][1]. Used with
    /// [`TsUrbControlDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_descriptor_to_device
    pub const URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE: Self = Self(12);

    /// Represents [`URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT`][1]. Used with
    /// [`TsUrbControlDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_descriptor_to_endpoint
    pub const URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT: Self = Self(37);

    /// Represents [`URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE`][1]. Used with
    /// [`TsUrbControlDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_descriptor_to_interface
    pub const URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE: Self = Self(41);

    /// Represents [`URB_FUNCTION_SET_FEATURE_TO_DEVICE`][1]. Used with [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_feature_to_device
    pub const URB_FUNCTION_SET_FEATURE_TO_DEVICE: Self = Self(13);

    /// Represents [`URB_FUNCTION_SET_FEATURE_TO_INTERFACE`][1]. Used with
    /// [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_feature_to_interface
    pub const URB_FUNCTION_SET_FEATURE_TO_INTERFACE: Self = Self(14);

    /// Represents [`URB_FUNCTION_SET_FEATURE_TO_ENDPOINT`][1]. Used with
    /// [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_feature_to_endpoint
    pub const URB_FUNCTION_SET_FEATURE_TO_ENDPOINT: Self = Self(15);

    /// Represents [`URB_FUNCTION_SET_FEATURE_TO_OTHER`][1]. Used with [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_set_feature_to_other
    pub const URB_FUNCTION_SET_FEATURE_TO_OTHER: Self = Self(35);

    /// Represents [`URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE`][1]. Used with
    /// [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_clear_feature_to_device
    pub const URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE: Self = Self(16);

    /// Represents [`URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE`][1]. Used with
    /// [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_clear_feature_to_interface
    pub const URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE: Self = Self(17);

    /// Represents [`URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT`][1]. Used with
    /// [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_clear_feature_to_endpoint
    pub const URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT: Self = Self(18);

    /// Represents [`URB_FUNCTION_CLEAR_FEATURE_TO_OTHER`][1]. Used with
    /// [`TsUrbControlFeatRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_clear_feature_to_other
    pub const URB_FUNCTION_CLEAR_FEATURE_TO_OTHER: Self = Self(34);

    /// Represents [`URB_FUNCTION_GET_STATUS_FROM_DEVICE`][1]. Used with
    /// [`TsUrbControlGetStatusRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_status_from_device
    pub const URB_FUNCTION_GET_STATUS_FROM_DEVICE: Self = Self(19);

    /// Represents [`URB_FUNCTION_GET_STATUS_FROM_INTERFACE`][1]. Used with
    /// [`TsUrbControlGetStatusRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_status_from_interface
    pub const URB_FUNCTION_GET_STATUS_FROM_INTERFACE: Self = Self(20);

    /// Represents [`URB_FUNCTION_GET_STATUS_FROM_ENDPOINT`][1]. Used with
    /// [`TsUrbControlGetStatusRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_status_from_endpoint
    pub const URB_FUNCTION_GET_STATUS_FROM_ENDPOINT: Self = Self(21);

    /// Represents [`URB_FUNCTION_GET_STATUS_FROM_OTHER`][1]. Used with
    /// [`TsUrbControlGetStatusRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_status_from_other
    pub const URB_FUNCTION_GET_STATUS_FROM_OTHER: Self = Self(33);

    /// Represents [`URB_FUNCTION_VENDOR_DEVICE`][1]. Used with [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_vendor_device
    pub const URB_FUNCTION_VENDOR_DEVICE: Self = Self(23);

    /// Represents [`URB_FUNCTION_VENDOR_INTERFACE`][1]. Used with
    /// [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_vendor_interface
    pub const URB_FUNCTION_VENDOR_INTERFACE: Self = Self(24);

    /// Represents [`URB_FUNCTION_VENDOR_ENDPOINT`][1]. Used with
    /// [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_vendor_endpoint
    pub const URB_FUNCTION_VENDOR_ENDPOINT: Self = Self(25);

    /// Represents [`URB_FUNCTION_VENDOR_OTHER`][1]. Used with [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_vendor_other
    pub const URB_FUNCTION_VENDOR_OTHER: Self = Self(32);

    /// Represents [`URB_FUNCTION_CLASS_DEVICE`][1]. Used with [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_class_device
    pub const URB_FUNCTION_CLASS_DEVICE: Self = Self(26);

    /// Represents [`URB_FUNCTION_CLASS_INTERFACE`][1]. Used with
    /// [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_class_interface
    pub const URB_FUNCTION_CLASS_INTERFACE: Self = Self(27);

    /// Represents [`URB_FUNCTION_CLASS_ENDPOINT`][1]. Used with [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_class_endpoint
    pub const URB_FUNCTION_CLASS_ENDPOINT: Self = Self(28);

    /// Represents [`URB_FUNCTION_CLASS_OTHER`][1]. Used with [`TsUrbControlVendorClassRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_class_other
    pub const URB_FUNCTION_CLASS_OTHER: Self = Self(31);

    /// Represents [`URB_FUNCTION_GET_CONFIGURATION`][1]. Used with
    /// [`TsUrbControlGetConfigRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_configuration
    pub const URB_FUNCTION_GET_CONFIGURATION: Self = Self(38);

    /// Represents [`URB_FUNCTION_GET_INTERFACE`][1]. Used with [`TsUrbControlGetInterfaceRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_interface
    pub const URB_FUNCTION_GET_INTERFACE: Self = Self(39);

    /// Represents [`URB_FUNCTION_GET_MS_FEATURE_DESCRIPTOR`][1]. Used with
    /// [`TsUrbOsFeatDescRequest`].
    ///
    /// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header#urb_function_get_ms_feature_descriptor
    pub const URB_FUNCTION_GET_MS_FEATURE_DESCRIPTOR: Self = Self(42);
}

impl From<u16> for UrbFunction {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<UrbFunction> for u16 {
    fn from(value: UrbFunction) -> Self {
        value.0
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.1.1 TS_URB_HEADER][1].
///
/// Common header for all of the [`TsUrb`] variants. Analogous to how [`SharedMsgHeader`] is for
/// all the "top-level" packets defined in the spec.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/578da9ca-3116-4608-9737-1bf3df4de3d1
#[doc(alias = "TS_URB_HEADER")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbHeader {
    /// Indicates what function to perform (see [`UrbFunction`]).
    pub func: UrbFunction,
    // pub(crate) urb_function: u16,
    /// An ID that uniquely identifies a [`TRANSFER_IN_REQUEST`][1] or [`TRANSFER_OUT_REQUEST`][2]
    /// message.
    pub req_id: RequestIdTransferInOut,
    /// Determines if the client is to send a **Request Completion** message for a
    /// [`TRANSFER_IN_REQUEST`] or [`TRANSFER_OUT_REQUEST`] message.
    ///
    /// * If the header is for a [`TRANSFER_IN_REQUEST`] message, this field **MUST** be `false`;
    ///   and the client is to send a message in response (either [`URB_COMPLETION`][3] or
    ///   [`URB_COMPLETION_NO_DATA`][4]).
    ///
    /// * If the header is for a [`TRANSFER_OUT_REQUEST`] message and this field is `false`;
    ///   the client is to send a ([`URB_COMPLETION_NO_DATA`]) message in response.
    ///
    /// * If the header is for a [`TRANSFER_OUT_REQUEST`] message and this field is `true`;
    ///   the client is *not* to send a ([`URB_COMPLETION_NO_DATA`]) message in response. This field
    ///   *can* be `true` if:
    ///
    ///     1. `urb_function` is set to [`UrbFunc::IsochTransfer`] (so the header is being used for
    ///        a [`TS_URB_ISOCH_TRANSFER`][5] structure), and
    ///
    ///     2. the [`USB_DEVICE_CAPABILITIES.NoAckIsochWriteJitterBufferSizeInMs`][6] field is
    ///        non-zero, which represents the amount of outstanding isochronous data the client
    ///        expects from the server (can be checked with
    ///        [`NoAckIsochWriteJitterBufSizeInMs::outstanding_isoch_data`][7]).
    ///
    ///
    /// [6]: crate::pdu::dev_sink::UsbDeviceCaps::no_ack_isoch_write_jitter_buf_size
    /// [7]: crate::pdu::dev_sink::NoAckIsochWriteJitterBufSizeInMs::outstanding_isoch_data
    pub no_ack: bool,
}

impl TsUrbHeader {
    pub const FIXED_PART_SIZE: usize =
        /* size_of::<u16>(/* Size */) + */ /* SHOULD BE managed by the outer TS_URB */
        size_of::<u16>(/* URB Function */) + size_of::<u32>(/* RequestId, NoAck */);
}

impl Encode for TsUrbHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.func.into());

        let no_ack = u32::from(self.no_ack) << 31;
        let last32 = u32::from(self.req_id) | no_ack;
        dst.write_u32(last32);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_HEADER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for TsUrbHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let func = UrbFunction::from(src.read_u16());
        let last32 = src.read_u32();
        let req_id = RequestIdTransferInOut::try_from(last32 & 0x7F_FF_FF_FF).expect("value clamped");
        let no_ack = (last32 >> 31) != 0;

        Ok(Self { func, req_id, no_ack })
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.1.3 TS_USBD_PIPE_INFORMATION][1].
///
/// Based on the [`USBD_PIPE_INFORMATION`][2] structure.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/cc12d23f-9712-4bf1-9235-76c3bd70115b
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_usbd_pipe_information
#[doc(alias = "TS_USBD_PIPE_INFORMATION")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUsbdPipeInfo {
    pub max_packet_size: u16,
    pub max_transfer_size: u32,
    pub pipe_flags: u32,
}

impl TsUsbdPipeInfo {
    pub const FIXED_PART_SIZE: usize = size_of::<u16>(/* MaximumPacketSize */)
        + size_of::<u16>(/* Padding */)
        + size_of::<u32>(/* MaximumTransferSize */)
        + size_of::<u32>(/* PipeFlags */);
}

impl Encode for TsUsbdPipeInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.max_packet_size);
        write_padding!(dst, 2);
        dst.write_u32(self.max_transfer_size);
        dst.write_u32(self.pipe_flags);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_USBD_PIPE_INFORMATION"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for TsUsbdPipeInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let max_packet_size = src.read_u16();
        read_padding(src, 2);
        let max_transfer_size = src.read_u32();
        let pipe_flags = src.read_u32();

        Ok(Self {
            max_packet_size,
            max_transfer_size,
            pipe_flags,
        })
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.1.2 TS_USBD_INTERFACE_INFORMATION][1].
///
/// Based on the [`USBD_INTERFACE_INFORMATION`][2] structure.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e8377327-1d22-48d2-b0f1-006f08cddcab
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_usbd_interface_information
#[doc(alias = "TS_USBD_INTERFACE_INFORMATION")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUsbdInterfaceInfo {
    pub interface_number: u8,
    pub alternate_setting: u8,
    /// **MUST NOT** have more than 30 pipe information structures.
    pub ts_usbd_pipe_info: Vec<TsUsbdPipeInfo>,
}

impl TsUsbdInterfaceInfo {
    pub const FIXED_SIZED_FIELDS_SIZE: usize = size_of::<u16>(/* Length */)
        + size_of::<u16>(/* NumberOfPipesExpected */)
        + size_of::<u8>(/* InterfaceNumber */)
        + size_of::<u8>(/* AlternateSetting */)
        + size_of::<u16>(/* Padding */)
        + size_of::<u32>(/* NumberOfPipes */);

    /// # Panics
    ///
    /// If *(number-of-pipes * 12) + 12* is greater than `u16::MAX`.
    #[inline]
    pub fn length(&self) -> u16 {
        (Self::FIXED_SIZED_FIELDS_SIZE + self.ts_usbd_pipe_info.len() * TsUsbdPipeInfo::FIXED_PART_SIZE)
            .try_into()
            .expect("Max: 12 + 30 * 12 = 372")
    }
}

impl Encode for TsUsbdInterfaceInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.length());

        // // NOTE: Do *WE* really need to enforce this stuff?
        // if self.ts_usbd_pipe_info.len() > MAX_NON_DEFAULT_EP_COUNT {
        //     return Err(invalid_field_err!(
        //         "TS_USBD_INTERFACE_INFORMATION::TS_USBD_PIPE_INFORMATION[..]",
        //         "has more than 30 TS_USBD_PIPE_INFORMATION structures"
        //     ));
        // }
        dst.write_u16(
            self.ts_usbd_pipe_info
                .len()
                .try_into()
                .map_err(|e| other_err!(source: e))?,
        );
        dst.write_u8(self.interface_number);
        dst.write_u8(self.alternate_setting);
        write_padding!(dst, 2);
        dst.write_u32(
            self.ts_usbd_pipe_info
                .len()
                .try_into()
                .map_err(|e| other_err!(source: e))?,
        );
        self.ts_usbd_pipe_info.iter().try_for_each(|pipe| pipe.encode(dst))?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_USBD_INTERFACE_INFORMATION"
    }

    fn size(&self) -> usize {
        self.length().into()
    }
}

impl Decode<'_> for TsUsbdInterfaceInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_SIZED_FIELDS_SIZE);

        let length @ 12.. = src.read_u16() else {
            return Err(invalid_field_err!(
                "TS_USBD_INTERFACE_INFORMATION::Length",
                "is less than min reqd value of 12"
            ));
        };

        let remaining_length = usize::from(length) - 2 /* Length */;
        ensure_size!(in: src, size: remaining_length);
        let mut src = ReadCursor::new(src.read_slice(remaining_length));

        let number_of_pipes_expected = src.read_u16();
        let interface_number = src.read_u8();
        let alternate_setting = src.read_u8();
        read_padding!(&mut src, 2);
        let number_of_pipes = src.read_u32();

        if number_of_pipes != number_of_pipes_expected.into() {
            return Err(invalid_field_err!(
                "TS_USBD_INTERFACE_INFORMATION::NumberOfPipesExpected",
                "is not equal to TS_USBD_INTERFACE_INFORMATION::NumberOfPipes"
            ));
        }

        {
            let length_suggested_size = length.checked_sub(Self::FIXED_SIZED_FIELDS_SIZE.try_into().expect("is 12"));
            let Some(length_suggested_size) = length_suggested_size else {
                return Err(invalid_field_err!(
                    "TS_USBD_INTERFACE_INFORMATION::Length",
                    "is too small"
                ));
            };

            if usize::from(length_suggested_size) / TsUsbdPipeInfo::FIXED_PART_SIZE
                != number_of_pipes.try_into().map_err(|e| other_err!(source: e))?
            {
                return Err(invalid_field_err!(
                    "TS_USBD_INTERFACE_INFORMATION::NumberOfPipes",
                    "does not reflect number of pipes suggested by TS_USBD_INTERFACE_INFORMATION::Length"
                ));
            }
        }

        #[expect(clippy::map_with_unused_argument_over_ranges)]
        let ts_usbd_pipe_info: Vec<TsUsbdPipeInfo> = (0..number_of_pipes)
            .map(|_| TsUsbdPipeInfo::decode(&mut src))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            interface_number,
            alternate_setting,
            ts_usbd_pipe_info,
        })
    }
}

/// USB2.0 spec: 9.6.3 Configuration
#[doc(alias = "USB_CONFIGURATION_DESCRIPTOR")]
#[derive(Debug, PartialEq, Clone)]
pub struct UsbConfigDesc {
    pub length: u8,
    pub descriptor_type: u8,
    pub total_length: u16,
    pub num_interfaces: u8,
    pub configuration_value: u8,
    pub configuration: u8,
    pub attributes: u8,
    pub max_power: u8,
}

impl UsbConfigDesc {
    pub const FIXED_PART_SIZE: usize = size_of::<u8>(/* bLength */)
        + size_of::<u8>(/* bDescriptorType */)
        + size_of::<u16>(/* wTotalLength */)
        + size_of::<u8>(/* bNumInterfaces */)
        + size_of::<u8>(/* bConfigurationValue */)
        + size_of::<u8>(/* iConfiguration */)
        + size_of::<u8>(/* bmAttributes */)
        + size_of::<u8>(/* MaxPower */);
}

impl Encode for UsbConfigDesc {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u8(self.length);
        dst.write_u8(self.descriptor_type);
        dst.write_u16(self.total_length);
        dst.write_u8(self.num_interfaces);
        dst.write_u8(self.configuration_value);
        dst.write_u8(self.configuration);
        dst.write_u8(self.attributes);
        dst.write_u8(self.max_power);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "USB_CONFIGURATION_DESCRIPTOR"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for UsbConfigDesc {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let length = src.read_u8();
        let descriptor_type = src.read_u8();
        let total_length = src.read_u16();
        let num_interfaces = src.read_u8();
        let configuration_value = src.read_u8();
        let configuration = src.read_u8();
        let attributes = src.read_u8();
        let max_power = src.read_u8();

        Ok(Self {
            length,
            descriptor_type,
            total_length,
            num_interfaces,
            configuration_value,
            configuration,
            attributes,
            max_power,
        })
    }
}

/// USB2.0 spec: 9.3 USB Device Requests: Table 9-2. Format of Setup Data
#[repr(C)]
#[derive(Debug, PartialEq, Clone)]
pub struct SetupPacket {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl SetupPacket {
    pub const FIXED_PART_SIZE: usize = 8;
}

impl Encode for SetupPacket {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u8(self.request_type);
        dst.write_u8(self.request);
        dst.write_u16(self.value);
        dst.write_u16(self.index);
        dst.write_u16(self.length);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "USB2SetupData"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for SetupPacket {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let request_type = src.read_u8();
        let request = src.read_u8();
        let value = src.read_u16();
        let index = src.read_u16();
        let length = src.read_u16();

        Ok(Self {
            request_type,
            request,
            value,
            index,
            length,
        })
    }
}
