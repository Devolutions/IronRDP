use ironrdp_core::{
    Decode, DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size,
    unsupported_value_err,
};
use windows_sys::Win32::Devices::Usb;

use alloc::format;

use crate::pdu::common::RequestIdTsUrb;

/// Numeric code that indicates the requested operation for a USB Request Block (URB).
///
/// URB Function codes are used with [`TS_URB_HEADER`][1]'s. This code should represent the
/// `TS_URB` structure the [`TS_URB_HEADER`][1] is used with.
///
/// * [WDK: USB: _URB_HEADER][2]
/// * [USB request blocks (URBs)][3]
///
/// [1]: TsUrbHeader
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_header
/// [3]: https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/communicating-with-a-usb-device
//
// NOTE: There are a few variants for Memory Descriptor Lists (MDL). Should a client just behave
// like it did not receive any of the MDL variants? Cause the client receives the data buffer over
// the network, so MDL's don't really make a point.
#[expect(
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    reason = "MS-RDPEUSB 2.2.9.1.1 specifies URB Function as being 2 bytes"
)]
#[repr(u16)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum UrbFunction {
    /// Indicates to the host controller driver that a configuration is to be selected. If set,
    /// the URB is used with URB_SELECT_CONFIGURATION as the data structure.
    #[doc(alias = "URB_FUNCTION_SELECT_CONFIGURATION")]
    SelectConfiguration = Usb::URB_FUNCTION_SELECT_CONFIGURATION as u16,

    /// Indicates to the host controller driver that an alternate interface setting is being
    /// selected for an interface. If set, the URB is used with _URB_SELECT_INTERFACE as the data
    /// structure.
    #[doc(alias = "URB_FUNCTION_SELECT_INTERFACE")]
    SelectInterface = Usb::URB_FUNCTION_SELECT_INTERFACE as u16,

    /// Indicates that all outstanding requests for a pipe should be canceled. If set, the URB is
    /// used with _URB_PIPE_REQUEST as the data structure. This general-purpose request enables a
    /// client to cancel any pending transfers for the specified pipe. Pipe state and endpoint
    /// state are unaffected. The abort request might complete before all outstanding requests
    /// have completed. Do not assume that completion of the abort request implies that all other
    /// outstanding requests have completed.
    #[doc(alias = "URB_FUNCTION_ABORT_PIPE")]
    AbortPipe = Usb::URB_FUNCTION_ABORT_PIPE as u16,

    /// Requests the current frame number from the host controller driver. If set, the URB is used
    /// with _URB_GET_CURRENT_FRAME_NUMBER as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_CURRENT_FRAME_NUMBER")]
    GetCurrentFrameNumber = Usb::URB_FUNCTION_GET_CURRENT_FRAME_NUMBER as u16,

    /// Transfers data to or from a control pipe. If set, the URB is used with
    /// _URB_CONTROL_TRANSFER as the data structure.
    #[doc(alias = "URB_FUNCTION_CONTROL_TRANSFER")]
    ControlTransfer = Usb::URB_FUNCTION_CONTROL_TRANSFER as u16,

    /// Transfers data to or from a control pipe without a time limit specified by a timeout
    /// value. If set, the URB is used with URB_CONTROL_TRANSFER_EX as the data structure.
    #[doc(alias = "URB_FUNCTION_CONTROL_TRANSFER_EX")]
    ControlTransferEx = Usb::URB_FUNCTION_CONTROL_TRANSFER_EX as u16,

    /// Transfers data from a bulk pipe or interrupt pipe or to a bulk pipe. If set, the URB is
    /// used with _URB_BULK_OR_INTERRUPT_TRANSFER as the data structure.
    #[doc(alias = "URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER")]
    BulkOrInterruptTransfer = Usb::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER as u16,

    /// Transfers data to and from a bulk pipe or interrupt pipe, by using chained MDLs. If set,
    /// the URB is used with _URB_BULK_OR_INTERRUPT_TRANSFER as the data structure. The client
    /// driver must set the TransferBufferMDL member to the first MDL structure in the chain that
    /// contains the transfer buffer. The USB driver stack ignores the TransferBuffer member when
    /// processing this URB.
    #[doc(alias = "URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER_USING_CHAINED_MDL")]
    BulkOrInterruptTransferUsingChainedMdl = Usb::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER_USING_CHAINED_MDL as u16,

    /// Transfers data to or from an isochronous pipe. If set, the URB is used with
    /// _URB_ISOCH_TRANSFER as the data structure.
    #[doc(alias = "URB_FUNCTION_ISOCH_TRANSFER")]
    IsochTransfer = Usb::URB_FUNCTION_ISOCH_TRANSFER as u16,

    /// Transfers data to or from an isochronous pipe by using chained MDLs. If set, the URB is
    /// used with _URB_ISOCH_TRANSFER as the data structure. The client driver must set the
    /// TransferBufferMDL member to the first MDL in the chain that contains the transfer buffer.
    /// The USB driver stack ignores the TransferBuffer member when processing this URB.
    #[doc(alias = "URB_FUNCTION_ISOCH_TRANSFER_USING_CHAINED_MDL")]
    IsochTransferUsingChainedMdl = Usb::URB_FUNCTION_ISOCH_TRANSFER_USING_CHAINED_MDL as u16,

    /// Resets the indicated pipe. If set, this URB is used with _URB_PIPE_REQUEST. The bus driver
    /// accomplishes three tasks in response to this URB:
    ///
    /// First, for all pipes except isochronous pipes, this URB sends a CLEAR_FEATURE request to
    /// clear the device's ENDPOINT_HALT feature.
    ///
    /// Second, the USB bus driver resets the data
    /// toggle on the host side, as required by the USB specification. The USB device should reset
    /// the data toggle on the device side when the bus driver clears its ENDPOINT_HALT feature.
    /// Since some non-compliant devices do not support this feature, Microsoft provides the two
    /// additional URBs: URB_FUNCTION_SYNC_CLEAR_STALL and URB_FUNCTION_SYNC_RESET_PIPE. These
    /// allow client drivers to clear the ENDPOINT_HALT feature on the device, or reset the pipe
    /// on the host side, respectively, without affecting the data toggle on the host side. If the
    /// device does not reset the data toggle when it should, then the client driver can compensate
    /// for this defect by not resetting the host-side data toggle. If the data toggle is reset on
    /// the host side but not on the device side, packets will get out of sequence, and the device
    /// might drop packets.
    ///
    /// Third, after the bus driver has successfully reset the pipe, it resumes transfers with the
    /// next queued URB.  After a pipe reset, transfers resume with the next queued URB.  It is not
    /// necessary to clear a halt condition on a default control pipe. The default control pipe
    /// must always accept setup packets, and so if it halts, the USB stack will clear the halt
    /// condition automatically. The client driver does not need to take any special action to
    /// clear the halt condition on a default pipe.  All transfers must be aborted or canceled
    /// before attempting to reset the pipe.  This URB must be sent at PASSIVE_LEVEL.
    #[doc(alias = "URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL")]
    SyncResetPipeAndClearStall = Usb::URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL as u16,

    /// Clears the halt condition on the host side of a pipe. If set, this URB is used with
    /// _URB_PIPE_REQUEST as the data structure.
    ///
    /// This URB allows a client to clear the halted state of a pipe without resetting the data
    /// toggle and without clearing the endpoint stall condition (feature ENDPOINT_HALT). To clear
    /// a halt condition on the pipe, reset the host-side data toggle and clear a stall on the
    /// device with a single operation, use SYNC_RESET_PIPE_AND_CLEAR_STALL.
    ///
    /// The following status codes are important and have the indicated meaning:
    ///
    /// USBD_STATUS_INVALID_PIPE_HANDLE: The PipeHandle is not valid
    ///
    /// USBD_STATUS_ERROR_BUSY: The endpoint has active transfers pending.
    ///
    /// It is not necessary to clear a halt condition on a default control pipe. The default
    /// control pipe must always accept setup packets, and so if it halts, the USB stack will clear
    /// the halt condition automatically. The client driver does not need to take any special
    /// action to clear the halt condition on a default pipe.
    ///
    /// All transfers must be aborted or canceled before attempting to reset the pipe.
    ///
    /// This URB must be sent at PASSIVE_LEVEL.
    #[doc(alias = "URB_FUNCTION_SYNC_RESET_PIPE")]
    SyncResetPipe = Usb::URB_FUNCTION_SYNC_RESET_PIPE as u16,

    /// Clears the stall condition on the endpoint. For all pipes except isochronous pipes, this
    /// URB sends a CLEAR_FEATURE request to clear the device's ENDPOINT_HALT feature. However,
    /// unlike the RB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL function, this URB function does
    /// not reset the data toggle on the host side of the pipe. The USB specification requires
    /// devices to reset the device-side data toggle after the client clears the device's
    /// ENDPOINT_HALT feature, but some non-compliant devices do not reset their data toggle
    /// properly. Client drivers that manage such devices can compensate for this defect by
    /// clearing the stall condition directly with SYNC_CLEAR_STALL instead of
    /// resetting the pipe with SYNC_RESET_PIPE_AND_CLEAR_STALL.
    /// SYNC_CLEAR_STALL clears a stall condition on the device without resetting
    /// the host-side data toggle. This prevents a non-compliant device from interpreting the
    /// next packet as a retransmission and dropping the packet.
    ///
    /// If set, the URB is used with _URB_PIPE_REQUEST as the data structure.
    ///
    /// This URB function should be sent at PASSIVE_LEVEL
    #[doc(alias = "URB_FUNCTION_SYNC_CLEAR_STALL")]
    SyncClearStall = Usb::URB_FUNCTION_SYNC_CLEAR_STALL as u16,

    /// Retrieves the device descriptor from a specific USB device. If set, the URB is used with
    /// _URB_CONTROL_DESCRIPTOR_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE")]
    GetDescriptorFromDevice = Usb::URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE as u16,

    /// Retrieves the descriptor from an endpoint on an interface for a USB device. If set, the
    /// URB is used with _URB_CONTROL_DESCRIPTOR_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT")]
    GetDescriptorFromEndpoint = Usb::URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT as u16,

    /// Sets a device descriptor on a device. If set, the URB is used with
    /// _URB_CONTROL_DESCRIPTOR_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE")]
    SetDescriptorToDevice = Usb::URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE as u16,

    /// Sets an endpoint descriptor on an endpoint for an interface. If set, the URB is used with
    /// _URB_CONTROL_DESCRIPTOR_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT")]
    SetDescriptorToEndpoint = Usb::URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT as u16,

    /// Sets a USB-defined feature on a device. If set, the URB is used with
    /// _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_FEATURE_TO_DEVICE")]
    SetFeatureToDevice = Usb::URB_FUNCTION_SET_FEATURE_TO_DEVICE as u16,

    /// Sets a USB-defined feature on an interface for a device. If set, the URB is used with
    /// _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_FEATURE_TO_INTERFACE")]
    SetFeatureToInterface = Usb::URB_FUNCTION_SET_FEATURE_TO_INTERFACE as u16,

    /// Sets a USB-defined feature on an endpoint for an interface on a USB device. If set, the
    /// URB is used with _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_FEATURE_TO_ENDPOINT")]
    SetFeatureToEndpoint = Usb::URB_FUNCTION_SET_FEATURE_TO_ENDPOINT as u16,

    /// Sets a USB-defined feature on a device-defined target on a USB device. If set, the URB is
    /// used with _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_FEATURE_TO_OTHER")]
    SetFeatureToOther = Usb::URB_FUNCTION_SET_FEATURE_TO_OTHER as u16,

    /// Clears a USB-defined feature on a device. If set, the URB is used with
    /// _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE")]
    ClearFeatureToDevice = Usb::URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE as u16,

    /// Clears a USB-defined feature on an interface for a device. If set, the URB is used with
    /// _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE")]
    ClearFeatureToInterface = Usb::URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE as u16,

    /// Clears a USB-defined feature on an endpoint, for an interface, on a USB device. If set,
    /// the URB is used with _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT")]
    ClearFeatureToEndpoint = Usb::URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT as u16,

    /// Clears a USB-defined feature on a device defined target on a USB device. If set, the URB
    /// is used with _URB_CONTROL_FEATURE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLEAR_FEATURE_TO_OTHER")]
    ClearFeatureToOther = Usb::URB_FUNCTION_CLEAR_FEATURE_TO_OTHER as u16,

    /// Retrieves status from a USB device. If set, the URB is used with
    /// _URB_CONTROL_GET_STATUS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_STATUS_FROM_DEVICE")]
    GetStatusFromDevice = Usb::URB_FUNCTION_GET_STATUS_FROM_DEVICE as u16,

    /// Retrieves status from an interface on a USB device. If set, the URB is used with
    /// _URB_CONTROL_GET_STATUS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_STATUS_FROM_INTERFACE")]
    GetStatusFromInterface = Usb::URB_FUNCTION_GET_STATUS_FROM_INTERFACE as u16,

    /// Retrieves status from an endpoint for an interface on a USB device. If set, the URB is
    /// used with _URB_CONTROL_GET_STATUS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_STATUS_FROM_ENDPOINT")]
    GetStatusFromEndpoint = Usb::URB_FUNCTION_GET_STATUS_FROM_ENDPOINT as u16,

    /// Retrieves status from a device-defined target on a USB device. If set, the URB is
    /// used with _URB_CONTROL_GET_STATUS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_STATUS_FROM_OTHER")]
    GetStatusFromOther = Usb::URB_FUNCTION_GET_STATUS_FROM_OTHER as u16,

    /// Sends a vendor-specific command to a USB device. If set, the URB is used with
    /// _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_VENDOR_DEVICE")]
    VendorDevice = Usb::URB_FUNCTION_VENDOR_DEVICE as u16,

    /// Sends a vendor-specific command for an interface on a USB device. If set, the URB is
    /// used with _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_VENDOR_INTERFACE")]
    VendorInterface = Usb::URB_FUNCTION_VENDOR_INTERFACE as u16,

    /// Sends a vendor-specific command for an endpoint on an interface on a USB device. If set,
    /// the URB is used with _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_VENDOR_ENDPOINT")]
    VendorEndpoint = Usb::URB_FUNCTION_VENDOR_ENDPOINT as u16,

    /// Sends a vendor-specific command to a device-defined target on a USB device. If set, the
    /// URB is used with _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_VENDOR_OTHER")]
    VendorOther = Usb::URB_FUNCTION_VENDOR_OTHER as u16,

    /// Sends a USB-defined class-specific command to a USB device. If set, the URB is used with
    /// _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLASS_DEVICE")]
    ClassDevice = Usb::URB_FUNCTION_CLASS_DEVICE as u16,

    /// Sends a USB-defined class-specific command to an interface on a USB device. If set, the
    /// URB is used with _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLASS_INTERFACE")]
    ClassInterface = Usb::URB_FUNCTION_CLASS_INTERFACE as u16,

    /// Sends a USB-defined class-specific command to an endpoint, on an interface, on a USB
    /// device. If set, the URB is used with _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data
    /// structure.
    #[doc(alias = "URB_FUNCTION_CLASS_ENDPOINT")]
    ClassEndpoint = Usb::URB_FUNCTION_CLASS_ENDPOINT as u16,

    /// Sends a USB-defined class-specific command to a device defined target on a USB device. If
    /// set, the URB is used with _URB_CONTROL_VENDOR_OR_CLASS_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLASS_OTHER")]
    ClassOther = Usb::URB_FUNCTION_CLASS_OTHER as u16,

    /// Retrieves the current configuration on a USB device. If set, the URB is used with
    /// _URB_CONTROL_GET_CONFIGURATION_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_CONFIGURATION")]
    GetConfiguration = Usb::URB_FUNCTION_GET_CONFIGURATION as u16,

    /// Retrieves the current settings for an interface on a USB device. If set, the URB is used
    /// with _URB_CONTROL_GET_INTERFACE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_INTERFACE")]
    GetInterface = Usb::URB_FUNCTION_GET_INTERFACE as u16,

    /// Retrieves the descriptor from an interface for a USB device. If set, the URB is used with
    /// _URB_CONTROL_DESCRIPTOR_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE")]
    GetDescriptorFromInterface = Usb::URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE as u16,

    /// Sets a descriptor for an interface on a USB device. If set, the URB is used with
    /// _URB_CONTROL_DESCRIPTOR_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE")]
    SetDescriptorToInterface = Usb::URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE as u16,

    /// Retrieves a Microsoft OS feature descriptor from a USB device or an interface on a USB
    /// device. If set, the URB is used with _URB_OS_FEATURE_DESCRIPTOR_REQUEST as the data
    /// structure.
    #[doc(alias = "URB_FUNCTION_GET_MS_FEATURE_DESCRIPTOR")]
    GetMsFeatureDescriptor = Usb::URB_FUNCTION_GET_MS_FEATURE_DESCRIPTOR as u16,

    /// Closes all opened streams in the specified bulk endpoint. If set, the URB is used with
    /// _URB_PIPE_REQUEST as the data structure.
    #[doc(alias = "URB_FUNCTION_CLOSE_STATIC_STREAMS")]
    CloseStaticStreams = Usb::URB_FUNCTION_CLOSE_STATIC_STREAMS as u16,
}

impl From<UrbFunction> for u16 {
    #[expect(clippy::as_conversions)]
    fn from(value: UrbFunction) -> Self {
        value as Self
    }
}

impl TryFrom<u16> for UrbFunction {
    type Error = DecodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        use UrbFunction::*;

        match value {
            0 => Ok(SelectConfiguration),
            1 => Ok(SelectInterface),
            2 => Ok(AbortPipe),
            7 => Ok(GetCurrentFrameNumber),
            8 => Ok(ControlTransfer),
            9 => Ok(BulkOrInterruptTransfer),
            10 => Ok(IsochTransfer),
            11 => Ok(GetDescriptorFromDevice),
            12 => Ok(SetDescriptorToDevice),
            13 => Ok(SetFeatureToDevice),
            14 => Ok(SetFeatureToInterface),
            15 => Ok(SetFeatureToEndpoint),
            16 => Ok(ClearFeatureToDevice),
            17 => Ok(ClearFeatureToInterface),
            18 => Ok(ClearFeatureToEndpoint),
            19 => Ok(GetStatusFromDevice),
            20 => Ok(GetStatusFromInterface),
            21 => Ok(GetStatusFromEndpoint),
            23 => Ok(VendorDevice),
            24 => Ok(VendorInterface),
            25 => Ok(VendorEndpoint),
            26 => Ok(ClassDevice),
            27 => Ok(ClassInterface),
            28 => Ok(ClassEndpoint),
            30 => Ok(SyncResetPipeAndClearStall),
            31 => Ok(ClassOther),
            32 => Ok(VendorOther),
            33 => Ok(GetStatusFromOther),
            34 => Ok(ClearFeatureToOther),
            35 => Ok(SetFeatureToOther),
            36 => Ok(GetDescriptorFromEndpoint),
            37 => Ok(SetDescriptorToEndpoint),
            38 => Ok(GetConfiguration),
            39 => Ok(GetInterface),
            40 => Ok(GetDescriptorFromInterface),
            41 => Ok(SetDescriptorToInterface),
            42 => Ok(GetMsFeatureDescriptor),
            48 => Ok(SyncResetPipe),
            49 => Ok(SyncClearStall),
            50 => Ok(ControlTransferEx),
            54 => Ok(CloseStaticStreams),
            55 => Ok(BulkOrInterruptTransferUsingChainedMdl),
            56 => Ok(IsochTransferUsingChainedMdl),

            value => Err(unsupported_value_err!(
                "URB Function",
                format!("unsupported value: {value}")
            )),
        }
    }
}

/// Header for every `TS_URB` structure, analogous to how [`SHARED_MSG_HEADER`][1] is for all
/// messages defined in [MS-RDPEUSB][2].
///
/// [1]: crate::pdu::common::SharedMsgHeader
/// [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125
#[doc(alias = "TS_URB_HEADER")]
pub struct TsUrbHeader {
    /// Size in bytes of the `TS_URB` structure the header is used for.
    pub size: u16,
    /// Indicates what function to perform (see [`UrbFunc`]).
    pub urb_func: UrbFunction,
    /// An ID that uniquely identifies a [`TRANSFER_IN_REQUEST`][1] or [`TRANSFER_OUT_REQUEST`][2]
    /// message.
    pub req_id: RequestIdTsUrb,
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
    ///     1. `urb_func` is set to [`UrbFunc::IsochTransfer`] (so the header is being used for a
    ///        [`TS_URB_ISOCH_TRANSFER`][5] structure), and
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
    const FIXED_PART_SIZE: usize = const { size_of::<u16>() + size_of::<UrbFunction>() + size_of::<RequestIdTsUrb>() };
}

impl Encode for TsUrbHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.size);
        #[expect(clippy::as_conversions)]
        dst.write_u16(self.urb_func as u16);

        let no_ack = u32::from(self.no_ack) << 31;
        let last32 = self.req_id.0 | no_ack;
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

        let size = src.read_u16();
        let urb_func = UrbFunction::try_from(src.read_u16())?;
        let last32 = src.read_u32();
        let req_id = RequestIdTsUrb::from(last32);
        let no_ack = (last32 >> 31) != 0;

        Ok(Self {
            size,
            urb_func,
            req_id,
            no_ack,
        })
    }
}
