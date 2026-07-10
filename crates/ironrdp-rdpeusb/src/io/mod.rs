//! Backend-facing USB I/O types.
//!
//! This module is the data-model boundary between the RDPEUSB state machines and USB backend
//! implementations. The types intentionally omit RDPEUSB routing fields such as message,
//! interface, and request IDs when the state machine can manage those fields itself.
//!
//! On the client side, [`UrbdrcDeviceBackend`] receives the `*Packet` request types and returns
//! the corresponding `*CompletionResult`. Returning `None` from an I/O method leaves the request
//! pending; the backend can later pass a completion and its `RequestId` to the matching
//! completion method on [`UrbdrcDeviceClient`].
//!
//! On the server side, methods on [`UrbdrcDeviceServer`] accept the `*Packet` request types and
//! return a [`ServerIoRequest`] ready for the DVC transport. Completion results received from the
//! client are delivered to [`UrbdrcDeviceServerBackend`].
//!
//! `TransferIn` and `TransferOut` are named from the USB device's perspective: an IN transfer
//! reads data from the device, while an OUT transfer writes data to it.
//!
//! [`UrbdrcDeviceBackend`]: crate::client::UrbdrcDeviceBackend
//! [`UrbdrcDeviceClient`]: crate::client::UrbdrcDeviceClient
//! [`UrbdrcDeviceServer`]: crate::server::UrbdrcDeviceServer
//! [`UrbdrcDeviceServerBackend`]: crate::server::UrbdrcDeviceServerBackend

use alloc::{string::String, vec::Vec};
use ironrdp_dvc::DvcMessage;
use ironrdp_pdu::{PduError, PduResult, pdu_other_err};

pub use crate::pdu::{
    completion::ts_urb_result::TsUrbResult,
    sink::UsbDeviceCaps,
    usb_dev::{
        InternalIoControl, IoControl, IoctlInternalUsb, UsbInternalIoctlCode, UsbRetractReason,
        ts_urb::{TsUrbInKind, TsUrbOutKind, utils::UrbFunction},
    },
    utils::{HResult, RequestId},
};
use crate::pdu::{
    header::{InterfaceId, MessageId},
    sink::{AddDevice, NoAckIsochWriteJitterBufSizeInMs},
    usb_dev::ts_urb::{TsUrbIn, TsUrbOut, utils::TsUrbHeader},
};

pub mod device;
pub use device::DeviceInfo;

/// Result of a device-text query.
///
/// A client backend returns this from [`UrbdrcDeviceBackend::query_device_text`]. A server backend
/// receives the decoded response through [`UrbdrcDeviceServerBackend::device_text`].
///
/// [`UrbdrcDeviceBackend::query_device_text`]: crate::client::UrbdrcDeviceBackend::query_device_text
/// [`UrbdrcDeviceServerBackend::device_text`]: crate::server::UrbdrcDeviceServerBackend::device_text
#[derive(Debug, Clone)]
pub struct DeviceText {
    pub hresult: u32,
    pub description: String,
}

/// Completion of an I/O control request.
///
/// This completes either an [`IoControlPacket`] or an [`InternalIoControlPacket`]. The request ID
/// is carried separately by the backend and completion APIs.
#[derive(Debug, Clone)]
pub struct IoControlCompletionResult {
    pub hresult: HResult,
    /// Number of bytes transferred, or the required buffer size for an insufficient-buffer result.
    ///
    /// On success, this must equal `output_buffer.len()`. For other failures, except an
    /// insufficient-buffer result, this value is ignored by the peer.
    pub information: u32,
    /// Data produced by the request.
    ///
    /// Its length must not exceed the request's output buffer size. For failures other than an
    /// insufficient-buffer result, this must be empty.
    pub output_buffer: Vec<u8>,
}

/// Completion of a USB IN transfer.
#[derive(Debug, Clone)]
pub struct TransferInCompletionResult {
    /// USB request-block result, including the USBD status and any operation-specific result.
    pub ts_urb_result: TsUrbResult,
    /// HRESULT returned by the transfer operation.
    pub hresult: HResult,
    /// Data read from the USB device.
    ///
    /// Its length must not exceed [`TransferInPacket::output_buffer_size`]. An empty buffer is
    /// encoded as `URB_COMPLETION_NO_DATA`.
    pub output_buffer: Vec<u8>,
}

/// Completion of a USB OUT transfer.
///
/// This is used only when [`TsUrbOutPacket::no_ack`] is `false`.
#[derive(Debug, Clone)]
pub struct TransferOutCompletionResult {
    /// USB request-block result, including the USBD status and any operation-specific result.
    pub ts_urb_result: TsUrbResult,
    /// HRESULT returned by the transfer operation.
    pub hresult: HResult,
    /// Number of bytes written to the USB device.
    ///
    /// This must not exceed the length of [`TransferOutPacket::output_buffer`].
    pub output_buffer_size: u32,
}

/// Backend-facing form of an RDPEUSB `IO_CONTROL` request.
#[derive(Debug, Clone)]
pub struct IoControlPacket {
    /// Operation to perform on the USB device or its upstream port.
    pub ioctl_code: IoctlInternalUsb,
    /// Raw input supplied to the operation.
    pub input_buffer: Vec<u8>,
    /// Maximum number of bytes that may be returned in the completion's output buffer.
    pub output_buffer_size: u32,
}

impl From<IoControl> for IoControlPacket {
    fn from(value: IoControl) -> Self {
        Self {
            ioctl_code: value.ioctl_code,
            input_buffer: value.input_buffer,
            output_buffer_size: value.output_buffer_size,
        }
    }
}

impl IoControlPacket {
    pub(crate) fn into_pdu(self, msg_id: MessageId, req_id: RequestId, udev_iface: InterfaceId) -> IoControl {
        IoControl {
            msg_id,
            udev_iface,
            ioctl_code: self.ioctl_code,
            input_buffer: self.input_buffer,
            output_buffer_size: self.output_buffer_size,
            req_id,
        }
    }
}

/// Backend-facing form of an RDPEUSB `INTERNAL_IO_CONTROL` request.
#[derive(Debug, Clone)]
pub enum InternalIoControlPacket {
    QueryBusTime,
}

impl InternalIoControlPacket {
    pub(crate) fn into_pdu(self, msg_id: MessageId, req_id: RequestId, udev_iface: InterfaceId) -> InternalIoControl {
        match self {
            Self::QueryBusTime => InternalIoControl {
                msg_id,
                udev_iface,
                ioctl_code: UsbInternalIoctlCode::QUERY_BUS_TIME,
                input_buffer: Vec::new(),
                output_buffer_size: 4,
                req_id,
            },
        }
    }
}

impl TryFrom<InternalIoControl> for InternalIoControlPacket {
    type Error = PduError;
    fn try_from(value: InternalIoControl) -> PduResult<Self> {
        match value.ioctl_code {
            UsbInternalIoctlCode::QUERY_BUS_TIME => {
                if !value.input_buffer.is_empty() {
                    return Err(pdu_other_err!("internal io control input buffer must be empty"));
                }
                if value.output_buffer_size != 4 {
                    return Err(pdu_other_err!("internal io control output buffer size must be 4"));
                }
                Ok(Self::QueryBusTime)
            }
            _ => Err(pdu_other_err!("unsupported InternalIoControl ioctl code")),
        }
    }
}

/// Backend-facing form of a USB IN transfer request.
///
/// An IN transfer requests data from the USB device.
#[derive(Debug, Clone)]
pub struct TransferInPacket {
    /// USB request block describing the operation.
    pub ts_urb: TsUrbInPacket,
    /// Maximum number of bytes requested from the USB device.
    pub output_buffer_size: u32,
}

/// USB request block carried by a [`TransferInPacket`].
#[derive(Debug, Clone)]
pub struct TsUrbInPacket {
    /// Operation-specific TS_URB payload.
    pub kind: TsUrbInKind,
    /// URB function code identifying `kind`.
    ///
    /// The function and payload variant must match; conversion to a wire PDU rejects a mismatch.
    pub func: UrbFunction,
}

impl TsUrbInPacket {
    pub(crate) fn into_ts_urb(self, request_id: u32) -> PduResult<TsUrbIn> {
        if !self.kind.matches_func(self.func) {
            return Err(pdu_other_err!("URB function does not match TS_URB payload"));
        }

        let ts_urb_size = self.kind.ts_urb_size()?;
        Ok(TsUrbIn {
            kind: self.kind,
            header: TsUrbHeader {
                ts_urb_size,
                func: self.func,
                req_id: request_id
                    .try_into()
                    .map_err(|_| pdu_other_err!("invalid transfer request id"))?,
                no_ack: false,
            },
        })
    }
}

impl From<TsUrbIn> for TsUrbInPacket {
    fn from(value: TsUrbIn) -> Self {
        Self {
            kind: value.kind,
            func: value.header.func,
        }
    }
}

/// Backend-facing form of a USB OUT transfer request.
///
/// An OUT transfer submits `output_buffer` to the USB device.
#[derive(Debug, Clone)]
pub struct TransferOutPacket {
    /// USB request block describing the operation.
    pub ts_urb: TsUrbOutPacket,
    /// Raw data to write to the USB device.
    pub output_buffer: Vec<u8>,
}

/// USB request block carried by a [`TransferOutPacket`].
#[derive(Debug, Clone)]
pub struct TsUrbOutPacket {
    /// Operation-specific TS_URB payload.
    pub kind: TsUrbOutKind,
    /// Whether the client must omit the completion for this request.
    ///
    /// RDPEUSB permits this only for isochronous OUT transfers when the device advertised a
    /// nonzero no-ack isochronous jitter buffer size.
    pub no_ack: bool,
    /// URB function code identifying `kind`.
    ///
    /// The function and payload variant must match; conversion to a wire PDU rejects a mismatch.
    pub func: UrbFunction,
}

impl From<TsUrbOut> for TsUrbOutPacket {
    fn from(value: TsUrbOut) -> Self {
        Self {
            kind: value.kind,
            no_ack: value.header.no_ack,
            func: value.header.func,
        }
    }
}

/// Server-side request ready to be sent over the device DVC.
///
/// Returned by the I/O request methods on [`UrbdrcDeviceServer`]. The server state machine has
/// already allocated and registered `request_id` before returning this value.
///
/// [`UrbdrcDeviceServer`]: crate::server::UrbdrcDeviceServer
pub struct ServerIoRequest {
    /// Request Identifier.
    pub request_id: RequestId,
    /// Whether the peer is expected to send a completion.
    ///
    /// This is `false` for a valid no-ack isochronous OUT transfer and `true` otherwise.
    pub expects_completion: bool,
    /// Request message to pass to the DVC transport.
    pub message: DvcMessage,
}

impl TsUrbOutPacket {
    pub(crate) fn into_ts_urb(
        self,
        request_id: u32,
        no_ack_isoch_write_jitter_buf_size: NoAckIsochWriteJitterBufSizeInMs,
    ) -> PduResult<TsUrbOut> {
        if !self.kind.matches_func(self.func) {
            return Err(pdu_other_err!("URB function does not match TS_URB payload"));
        }
        if self.no_ack
            && !matches!(
                self.func,
                UrbFunction::URB_FUNCTION_ISOCH_TRANSFER | UrbFunction::URB_FUNCTION_ISOCH_TRANSFER_USING_CHAINED_MDL
            )
        {
            return Err(pdu_other_err!("NoAck can only be set for TS_URB_ISOCH_TRANSFER"));
        }
        if self.no_ack && no_ack_isoch_write_jitter_buf_size.outstanding_isoch_data().is_none() {
            return Err(pdu_other_err!("NoAck is unsupported by USB device"));
        }

        let ts_urb_size = self.kind.ts_urb_size()?;
        Ok(TsUrbOut {
            kind: self.kind,
            header: TsUrbHeader {
                ts_urb_size,
                func: self.func,
                req_id: request_id
                    .try_into()
                    .map_err(|_| pdu_other_err!("invalid transfer request id"))?,
                no_ack: self.no_ack,
            },
        })
    }
}

/// Description of a redirected USB device announced by the client.
///
/// A server backend receives this through [`UrbdrcDeviceServerBackend::add_device`] after an
/// `ADD_DEVICE` message has been decoded and its UTF-16 fields converted to Rust strings.
///
/// [`UrbdrcDeviceServerBackend::add_device`]: crate::server::UrbdrcDeviceServerBackend::add_device
#[derive(Debug)]
pub struct DeviceAnnounce {
    pub device_instance_id: String,
    pub hw_ids: Vec<String>,
    pub compat_ids: Vec<String>,
    pub container_id: String,
    pub usb_device_caps: UsbDeviceCaps,
}

impl TryFrom<AddDevice> for DeviceAnnounce {
    type Error = PduError;
    fn try_from(value: AddDevice) -> Result<Self, Self::Error> {
        Ok(Self {
            device_instance_id: value
                .device_instance_id
                .into_native()
                .map_err(|e| pdu_other_err!("invalid device instance id").with_source(e))?,
            hw_ids: match value.hw_ids {
                Some(ids) => ids
                    .into_native()
                    .map_err(|e| pdu_other_err!("invalid hardware ids").with_source(e))?,
                None => Vec::new(),
            },
            compat_ids: match value.compat_ids {
                Some(ids) => ids
                    .into_native()
                    .map_err(|e| pdu_other_err!("invalid compatibility id").with_source(e))?,
                None => Vec::new(),
            },
            container_id: value
                .container_id
                .into_native()
                .map_err(|e| pdu_other_err!("invalid container id").with_source(e))?,
            usb_device_caps: value.usb_device_caps,
        })
    }
}
