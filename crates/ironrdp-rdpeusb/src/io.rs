use alloc::{string::String, vec::Vec};
use ironrdp_dvc::DvcMessage;
use ironrdp_pdu::{PduError, PduResult, pdu_other_err};

use crate::pdu::{
    completion::ts_urb_result::TsUrbResult,
    header::{InterfaceId, MessageId},
    sink::{AddDevice, NoAckIsochWriteJitterBufSizeInMs, UsbDeviceCaps},
    usb_dev::{
        InternalIoControl, IoControl, IoctlInternalUsb, UsbInternalIoctlCode,
        ts_urb::{
            TsUrbIn, TsUrbInKind, TsUrbOut, TsUrbOutKind,
            utils::{TsUrbHeader, UrbFunction},
        },
    },
    utils::{HResult, RequestId},
};

pub use crate::pdu::usb_dev::UsbRetractReason;

#[derive(Debug, Clone)]
pub struct DeviceText {
    pub hresult: u32,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct IoControlCompletionResult {
    pub hresult: HResult,
    pub information: u32,
    pub output_buffer: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TransferInCompletionResult {
    pub ts_urb_result: TsUrbResult,
    pub hresult: HResult,
    pub output_buffer: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TransferOutCompletionResult {
    pub ts_urb_result: TsUrbResult,
    pub hresult: HResult,
    pub output_buffer_size: u32,
}

#[derive(Debug, Clone)]
pub struct IoControlPacket {
    pub ioctl_code: IoctlInternalUsb,
    pub input_buffer: Vec<u8>,
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

#[derive(Debug, Clone)]
pub struct InternalIoControlPacket {
    pub ioctl_code: UsbInternalIoctlCode,
    pub input_buffer: Vec<u8>,
    pub output_buffer_size: u32,
}

impl InternalIoControlPacket {
    pub(crate) fn into_pdu(self, msg_id: MessageId, req_id: RequestId, udev_iface: InterfaceId) -> InternalIoControl {
        InternalIoControl {
            msg_id,
            udev_iface,
            ioctl_code: self.ioctl_code,
            input_buffer: self.input_buffer,
            output_buffer_size: self.output_buffer_size,
            req_id,
        }
    }
}

impl From<InternalIoControl> for InternalIoControlPacket {
    fn from(value: InternalIoControl) -> Self {
        Self {
            ioctl_code: value.ioctl_code,
            input_buffer: value.input_buffer,
            output_buffer_size: value.output_buffer_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferInPacket {
    pub ts_urb: TsUrbInPacket,
    pub output_buffer_size: u32,
}

#[derive(Debug, Clone)]
pub struct TsUrbInPacket {
    pub kind: TsUrbInKind,
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

#[derive(Debug, Clone)]
pub struct TransferOutPacket {
    pub ts_urb: TsUrbOutPacket,
    pub output_buffer: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TsUrbOutPacket {
    pub kind: TsUrbOutKind,
    pub no_ack: bool,
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

pub struct ServerIoRequest {
    pub request_id: RequestId,
    pub expects_completion: bool,
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

/// Device instance creation
#[derive(Debug)]
pub struct Device {
    pub device_instance_id: String,
    pub hw_ids: Vec<String>,
    pub compat_ids: Vec<String>,
    pub container_id: String,
    pub usb_device_caps: UsbDeviceCaps,
}

impl TryFrom<AddDevice> for Device {
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
