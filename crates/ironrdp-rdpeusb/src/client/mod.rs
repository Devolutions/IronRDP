use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};
use ironrdp_core::{Decode as _, EncodeResult, ReadCursor, impl_as_any, other_err};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};

use crate::CHANNEL_NAME;
use crate::pdu::UrbdrcServerDevicePdu;
use crate::pdu::completion::ts_urb_result::TsUrbResult;
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{InterfaceId, Mask, MessageId};
use crate::pdu::iface_manipulation::{InterfaceRelease, QueryInterfaceFailureResponse};
use crate::pdu::sink::AddVirtualChannel;
use crate::pdu::usb_dev::ts_urb::TsUrbOut;
use crate::pdu::usb_dev::{InternalIoControl, IoControl, QueryDeviceTextRsp, TransferInRequest, TransferOutRequest};
use crate::pdu::utils::{HResult, RequestId, RequestIdTransferInOut};
use crate::pdu::{
    UrbdrcServerControlPdu,
    caps::{Capability, RimExchangeCapabilityResponse},
    notify::ChannelCreated,
};

pub mod device;
pub use device::*;

/// A client for the URBDRC Control Virtual Channel.
pub struct UrbdrcControlClient {
    /// Indicates whether the channel is ready for add virtual channel.
    ready: bool,

    /// Spec [3.1]:
    /// Exchange-completed event: Signifies that the capability exchange is completed, that is,
    /// the client has sent a Channel Created message.
    ///
    /// [3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/511b4cd7-1940-4631-90ac-bf2189ba6735
    on_capability_exchanged: OnCapabilityExchanged,
}

type OnCapabilityExchanged = Box<dyn Fn() -> PduResult<Vec<DvcMessage>> + Send>;

impl UrbdrcControlClient {
    /// Create a new [UrbdrcControlClient] with the given callback.
    ///
    /// The `callback` will be called when the capability exchange is completed and the channel is
    /// ready to redirect new devices.
    ///
    /// Please note the `callback` will be called only once.
    pub fn new<F: Fn() -> PduResult<Vec<DvcMessage>> + Send + 'static>(callback: F) -> Self {
        Self {
            ready: false,
            on_capability_exchanged: Box::new(callback),
        }
    }

    /// Whether the channel is ready for add virtual channel.
    pub const fn ready(&self) -> bool {
        self.ready
    }

    /// Spec [3.3.5.1.1]:
    ///
    /// The client sends the ADD_VIRTUAL_CHANNEL message to server to request the server to create a
    /// new instance of dynamic virtual channel for USB redirection. The client sends this message
    /// for every USB device to be redirected. This isolates messages for each USB device in its own
    /// instance of a dynamic virtual channel.
    ///
    /// [3.3.5.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c7b1920a-d632-46d2-b62a-5c7e53570628
    pub fn add_virtual_channel(&self, channel_id: u32, dev_id: u32) -> EncodeResult<Vec<SvcMessage>> {
        if !self.ready {
            return Err(other_err!("is not ready for ADD_VIRTUAL_CHANNEL"));
        }
        // Follow FreeRDP use device id as message id
        encode_dvc_messages(
            channel_id,
            vec![Box::new(AddVirtualChannel { msg_id: dev_id })],
            ChannelFlags::empty(),
        )
    }
}

impl DvcProcessor for UrbdrcControlClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = UrbdrcServerControlPdu::decode(&mut ReadCursor::new(payload)).map_err(|e| decode_err!(e))?;
        use UrbdrcServerControlPdu::*;
        match pdu {
            Caps(caps_req_pdu) => Ok(vec![Box::new(RimExchangeCapabilityResponse {
                msg_id: caps_req_pdu.msg_id,
                capability: Capability::RimCapabilityVersion01,
                result: 0,
            })]),
            ChanCreated(chan_created_pdu) => Ok(vec![Box::new(ChannelCreated {
                msg_id: chan_created_pdu.msg_id,
                direction: crate::pdu::notify::Direction::ToServer,
            })]),
            QueryIfaceReq(query_face_pdu) => Ok(vec![Box::new(QueryInterfaceFailureResponse {
                iface_id: query_face_pdu.iface_id,
                msg_id: query_face_pdu.msg_id,
            })]),
            IfaceRelease(InterfaceRelease {
                iface_id,
                msg_id: _msg_id,
            }) => {
                if iface_id == InterfaceId::NOTIFY_CLIENT.with_mask(Mask::Proxy) && !self.ready {
                    // NOTE: MS-RDPEUSB does not normatively define RIMCALL_RELEASE as a
                    // server-ready-proceed barrier; the semantic comes from observed Windows
                    // urbdrc-server behavior. Pattern matches FreeRDP urbdrc_main.c since 2012
                    // (commit fa4d8fca1be, Atrust contribution). Two sync points: control DVC
                    // (server -> client ADD_VIRTUAL_CHANNEL); device DVC (server -> client
                    // ADD_DEVICE).
                    self.ready = true;
                    (self.on_capability_exchanged)()
                } else {
                    Ok(Vec::new())
                }
            }
        }
    }
}

impl_as_any!(UrbdrcControlClient);

impl DvcClientProcessor for UrbdrcControlClient {}

pub trait UrbdrcDeviceBackend: Send {
    /// Get the USB device information.
    fn device_info(&mut self, channel_id: u32) -> DeviceInfo;
    /// [Processing a Cancel Request Message][3.3.5.3.1]:
    ///
    /// The client MUST attempt to stop processing the request identified by the RequestId field in
    /// the CANCEL_REQUEST message. If the current request has not been completed it MUST be
    /// canceled. If the request has been completed, the client MUST ignore this CANCEL_REQUEST
    /// message.
    ///
    /// [3.3.5.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/d5315234-d9ba-42dc-bc1b-b421c57a21ae
    fn cancel_request(&mut self, request_id: RequestId, channel_id: u32);
    /// [Processing a Query Device Text Message][3.3.5.3.5]:
    ///
    /// After receiving the QUERY_DEVICE_TEXT message, the client forwards the request to the
    /// physical device. When the physical device completes the request, the client sends the result
    /// of the request to the server via QUERY_DEVICE_TEXT_RSP message and the RequestId field in
    /// the message MUST match the RequestId in the QUERY_DEVICE_TEXT message.
    ///
    /// [3.3.5.3.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/834f56cc-cfed-4649-8952-0b6486638c28
    fn query_device_text(&mut self, channel_id: u32, text_type: u32, locale_id: u32) -> Option<DeviceText>;
    /// Process an [`IoControl`] request.
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn io_control(&mut self, channel_id: u32, request_id: RequestId, request: IoControl) -> Option<IoControlResponse>;
    /// Process an [`InternalIoControl`] request.
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn internal_io_control(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: InternalIoControl,
    ) -> Option<IoControlResponse>;
    /// Process a [`TransferInRequest`].
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn transfer_in(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferInRequest,
    ) -> Option<UrbInResponse>;
    /// Process a [`TransferOutRequest`].
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn transfer_out(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutRequest,
    ) -> Option<UrbOutResponse>;
    /// [Processing a Retract Device Message][3.3.5.3.8]:
    ///
    /// After receiving the RETRACT_DEVICE message, the client SHOULD terminate the dynamic channel
    /// and stop redirecting the physical USB device.
    ///
    /// [3.3.5.3.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/77dc8e12-ddd6-4cb8-a3cc-247aacea7d6f
    fn retract(&mut self, channel_id: u32);
}

#[derive(Debug, Clone)]
pub struct DeviceText {
    pub hresult: u32,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct IoControlResponse {
    pub hresult: HResult,
    pub information: u32,
    pub output_buffer: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UrbInResponse {
    pub ts_urb_result: TsUrbResult,
    pub hresult: HResult,
    pub output_buffer: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UrbOutResponse {
    pub ts_urb_result: TsUrbResult,
    pub hresult: HResult,
    pub output_buffer_size: u32,
}

/// A client for the URBDRC Device Virtual Channel.
pub struct UrbdrcDeviceClient {
    /// Indicates whether the channel is ready for handling IO request.
    ready_for_io: bool,
    /// Per-device USB interface ID allocated by the DVC layer. This is intentionally kept out of
    /// `DeviceInfo`, which only describes backend USB facts.
    udev_iface: InterfaceId,
    request_completion: Option<InterfaceId>,
    backend: Box<dyn UrbdrcDeviceBackend>,
    pending_io: BTreeMap<RequestId, Pending>,
}

impl UrbdrcDeviceClient {
    pub fn new(udev_iface: InterfaceId, backend: Box<dyn UrbdrcDeviceBackend>) -> Self {
        Self {
            ready_for_io: false,
            udev_iface,
            request_completion: None,
            backend,
            pending_io: BTreeMap::new(),
        }
    }

    pub const fn ready_for_io(&self) -> bool {
        self.ready_for_io
    }

    pub const fn udev_iface(&self) -> InterfaceId {
        self.udev_iface
    }

    pub fn io_ctl_completion(&mut self, request_id: RequestId, response: IoControlResponse) -> PduResult<DvcMessage> {
        let Some(completion_iface) = self.request_completion else {
            return Err(pdu_other_err!("request completion uninitialized"));
        };
        let Some(Pending::IoCtl {
            msg_id,
            max_output_buf_size,
        }) = self.pending_io.remove(&request_id)
        else {
            return Err(pdu_other_err!("completion mismatch"));
        };
        let output_buffer_size =
            u32::try_from(response.output_buffer.len()).map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
        if output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }
        Ok(Box::new(IoControlCompletion {
            msg_id,
            completion_iface,
            hresult: response.hresult,
            request_id,
            information: response.information,
            output_buffer_size,
            output_buffer: response.output_buffer,
        }))
    }

    pub fn internal_io_ctl_completion(
        &mut self,
        request_id: RequestId,
        response: IoControlResponse,
    ) -> PduResult<DvcMessage> {
        let Some(completion_iface) = self.request_completion else {
            return Err(pdu_other_err!("request completion uninitialized"));
        };
        let Some(Pending::InternalIoCtl {
            msg_id,
            max_output_buf_size,
        }) = self.pending_io.remove(&request_id)
        else {
            return Err(pdu_other_err!("completion mismatch"));
        };
        let output_buffer_size =
            u32::try_from(response.output_buffer.len()).map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
        if output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }
        Ok(Box::new(IoControlCompletion {
            msg_id,
            completion_iface,
            hresult: response.hresult,
            request_id,
            information: response.information,
            output_buffer_size,
            output_buffer: response.output_buffer,
        }))
    }

    pub fn transfer_in_completion(&mut self, request_id: RequestId, response: UrbInResponse) -> PduResult<DvcMessage> {
        let Some(completion_iface) = self.request_completion else {
            return Err(pdu_other_err!("request completion uninitialized"));
        };
        let Some(Pending::TransferIn {
            msg_id,
            max_output_buf_size,
        }) = self.pending_io.remove(&request_id)
        else {
            return Err(pdu_other_err!("completion mismatch"));
        };
        let output_buffer_size =
            u32::try_from(response.output_buffer.len()).map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
        if output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }
        let request_id =
            RequestIdTransferInOut::try_from(request_id).map_err(|_| pdu_other_err!("invalid transfer request id"))?;
        if response.output_buffer.is_empty() {
            Ok(Box::new(UrbCompletionNoData {
                msg_id,
                completion_iface,
                req_id: request_id,
                ts_urb_result: response.ts_urb_result,
                hresult: response.hresult,
                output_buffer_size,
            }))
        } else {
            Ok(Box::new(UrbCompletion {
                msg_id,
                completion_iface,
                req_id: request_id,
                ts_urb_result: response.ts_urb_result,
                hresult: response.hresult,
                output_buffer: response.output_buffer,
            }))
        }
    }

    pub fn transfer_out_completion(
        &mut self,
        request_id: RequestId,
        response: UrbOutResponse,
    ) -> PduResult<DvcMessage> {
        let Some(Pending::TransferOut {
            msg_id,
            request_id,
            max_output_buf_size,
        }) = self.pending_io.remove(&request_id)
        else {
            return Err(pdu_other_err!("completion mismatch"));
        };
        if response.output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }
        let Some(completion_iface) = self.request_completion else {
            return Err(pdu_other_err!("request completion uninitialized"));
        };
        Ok(Box::new(UrbCompletionNoData {
            msg_id,
            completion_iface,
            req_id: request_id,
            ts_urb_result: response.ts_urb_result,
            hresult: response.hresult,
            output_buffer_size: response.output_buffer_size,
        }))
    }
}

impl DvcProcessor for UrbdrcDeviceClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = UrbdrcServerDevicePdu::decode(&mut ReadCursor::new(payload)).map_err(|e| decode_err!(e))?;

        use UrbdrcServerDevicePdu::*;
        match pdu {
            ChanCreated(chan_created_pdu) => Ok(vec![Box::new(ChannelCreated {
                msg_id: chan_created_pdu.msg_id,
                direction: crate::pdu::notify::Direction::ToServer,
            })]),
            QueryIfaceReq(query_face_pdu) => Ok(vec![Box::new(QueryInterfaceFailureResponse {
                iface_id: query_face_pdu.iface_id,
                msg_id: query_face_pdu.msg_id,
            })]),
            IfaceRelease(iface_release_pdu) => {
                if iface_release_pdu.iface_id == InterfaceId::NOTIFY_CLIENT.with_mask(Mask::Proxy) && !self.ready_for_io
                {
                    // NOTE: MS-RDPEUSB does not normatively define RIMCALL_RELEASE as a
                    // server-ready-proceed barrier; the semantic comes from observed Windows
                    // urbdrc-server behavior. Pattern matches FreeRDP urbdrc_main.c since 2012
                    // (commit fa4d8fca1be, Atrust contribution). Two sync points: control DVC
                    // (server -> client ADD_VIRTUAL_CHANNEL); device DVC (server -> client
                    // ADD_DEVICE).
                    self.ready_for_io = true;
                    let device_info = self.backend.device_info(channel_id);
                    let add_device = add_device_from_info(self.udev_iface, &device_info)?;

                    Ok(vec![Box::new(add_device)])
                } else {
                    Ok(Vec::new())
                }
            }
            CancelReq(cancel_req_pdu) => {
                if cancel_req_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                self.backend.cancel_request(cancel_req_pdu.req_id, channel_id);
                Ok(Vec::new())
            }
            RegReqCb(register_request_callback_pdu) => {
                if register_request_callback_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                self.request_completion = register_request_callback_pdu.request_completion;
                Ok(Vec::new())
            }
            Retract(retract_pdu) => {
                if retract_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                self.backend.retract(channel_id);
                Ok(Vec::new())
            }
            DevText(dev_text_pdu) => {
                if dev_text_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                if let Some(device_text) =
                    self.backend
                        .query_device_text(channel_id, dev_text_pdu.text_type, dev_text_pdu.locale_id)
                {
                    Ok(vec![Box::new(QueryDeviceTextRsp {
                        msg_id: dev_text_pdu.msg_id,
                        udev_iface: dev_text_pdu.udev_iface,
                        hresult: device_text.hresult,
                        device_description: device_text.description.into(),
                    })])
                } else {
                    Ok(Vec::new())
                }
            }
            IoCtl(io_ctl_pdu) => {
                if io_ctl_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                let msg_id = io_ctl_pdu.msg_id;
                let request_id = io_ctl_pdu.req_id;
                let max_output_buf_size = io_ctl_pdu.output_buffer_size;
                if let Some(io_ctl_response) = self.backend.io_control(channel_id, request_id, io_ctl_pdu) {
                    let output_buffer_size = u32::try_from(io_ctl_response.output_buffer.len())
                        .map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
                    if output_buffer_size > max_output_buf_size {
                        return Err(pdu_other_err!("output buffer exceeds maximum amount"));
                    }
                    let Some(completion_iface) = self.request_completion else {
                        return Err(pdu_other_err!("request completion uninitialized"));
                    };
                    Ok(vec![Box::new(IoControlCompletion {
                        msg_id,
                        completion_iface,
                        hresult: io_ctl_response.hresult,
                        request_id,
                        information: io_ctl_response.information,
                        output_buffer_size,
                        output_buffer: io_ctl_response.output_buffer,
                    })])
                } else {
                    self.pending_io.insert(
                        request_id,
                        Pending::IoCtl {
                            msg_id,
                            max_output_buf_size,
                        },
                    );
                    Ok(Vec::new())
                }
            }
            InternalIoCtl(internal_io_ctl_pdu) => {
                if internal_io_ctl_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                let msg_id = internal_io_ctl_pdu.msg_id;
                let request_id = internal_io_ctl_pdu.req_id;
                let max_output_buf_size = internal_io_ctl_pdu.output_buffer_size;
                if let Some(internal_io_ctl_response) =
                    self.backend
                        .internal_io_control(channel_id, request_id, internal_io_ctl_pdu)
                {
                    let output_buffer_size = u32::try_from(internal_io_ctl_response.output_buffer.len())
                        .map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
                    if output_buffer_size > max_output_buf_size {
                        return Err(pdu_other_err!("output buffer exceeds maximum amount"));
                    }
                    let Some(completion_iface) = self.request_completion else {
                        return Err(pdu_other_err!("request completion uninitialized"));
                    };
                    Ok(vec![Box::new(IoControlCompletion {
                        msg_id,
                        completion_iface,
                        hresult: internal_io_ctl_response.hresult,
                        request_id,
                        information: internal_io_ctl_response.information,
                        output_buffer_size,
                        output_buffer: internal_io_ctl_response.output_buffer,
                    })])
                } else {
                    self.pending_io.insert(
                        request_id,
                        Pending::InternalIoCtl {
                            msg_id,
                            max_output_buf_size,
                        },
                    );
                    Ok(Vec::new())
                }
            }
            TransferIn(transfer_in_pdu) => {
                if transfer_in_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                let msg_id = transfer_in_pdu.msg_id;
                let max_output_buf_size = transfer_in_pdu.output_buffer_size;
                let request_id = transfer_in_pdu.request_id();
                if let Some(urb_response) = self.backend.transfer_in(channel_id, request_id.into(), transfer_in_pdu) {
                    let output_buffer_size = u32::try_from(urb_response.output_buffer.len())
                        .map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
                    if output_buffer_size > max_output_buf_size {
                        return Err(pdu_other_err!("output buffer exceeds maximum amount"));
                    }
                    let Some(completion_iface) = self.request_completion else {
                        return Err(pdu_other_err!("request completion uninitialized"));
                    };
                    if urb_response.output_buffer.is_empty() {
                        Ok(vec![Box::new(UrbCompletionNoData {
                            msg_id,
                            completion_iface,
                            req_id: request_id,
                            ts_urb_result: urb_response.ts_urb_result,
                            hresult: urb_response.hresult,
                            output_buffer_size,
                        })])
                    } else {
                        Ok(vec![Box::new(UrbCompletion {
                            msg_id,
                            completion_iface,
                            req_id: request_id,
                            ts_urb_result: urb_response.ts_urb_result,
                            hresult: urb_response.hresult,
                            output_buffer: urb_response.output_buffer,
                        })])
                    }
                } else {
                    self.pending_io.insert(
                        request_id.into(),
                        Pending::TransferIn {
                            msg_id,
                            max_output_buf_size,
                        },
                    );
                    Ok(Vec::new())
                }
            }
            TransferOut(transfer_out_pdu) => {
                if transfer_out_pdu.udev_iface != self.udev_iface {
                    return Err(pdu_other_err!("usb device interface mismatch"));
                }
                let msg_id = transfer_out_pdu.msg_id;
                let output_buffer_size = u32::try_from(transfer_out_pdu.output_buffer.len())
                    .map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
                let (request_id, no_ack) = match &transfer_out_pdu.ts_urb {
                    TsUrbOut::CtlTransfer(urb) => (urb.header.req_id, urb.header.no_ack),
                    TsUrbOut::BulkInterruptTransfer(urb) => (urb.header.req_id, urb.header.no_ack),
                    TsUrbOut::IsochTransfer(urb) => (urb.header.req_id, urb.header.no_ack),
                    TsUrbOut::CtlDescReq(urb) => (urb.header.req_id, urb.header.no_ack),
                    TsUrbOut::VendorClassReq(urb) => (urb.header.req_id, urb.header.no_ack),
                    TsUrbOut::CtlTransferEx(urb) => (urb.header.req_id, urb.header.no_ack),
                };
                if let Some(urb_response) = self
                    .backend
                    .transfer_out(channel_id, request_id.into(), transfer_out_pdu)
                {
                    if no_ack {
                        return Ok(Vec::new());
                    }
                    if urb_response.output_buffer_size > output_buffer_size {
                        return Err(pdu_other_err!("output buffer exceeds maximum amount"));
                    }
                    let Some(completion_iface) = self.request_completion else {
                        return Err(pdu_other_err!("request completion uninitialized"));
                    };
                    Ok(vec![Box::new(UrbCompletionNoData {
                        msg_id,
                        completion_iface,
                        req_id: request_id,
                        ts_urb_result: urb_response.ts_urb_result,
                        hresult: urb_response.hresult,
                        output_buffer_size: urb_response.output_buffer_size,
                    })])
                } else if no_ack {
                    Ok(Vec::new())
                } else {
                    self.pending_io.insert(
                        request_id.into(),
                        Pending::TransferOut {
                            msg_id,
                            request_id,
                            max_output_buf_size: output_buffer_size,
                        },
                    );
                    Ok(Vec::new())
                }
            }
        }
    }
}

impl_as_any!(UrbdrcDeviceClient);

impl DvcClientProcessor for UrbdrcDeviceClient {}

enum Pending {
    IoCtl {
        msg_id: MessageId,
        max_output_buf_size: u32,
    },
    InternalIoCtl {
        msg_id: MessageId,
        max_output_buf_size: u32,
    },
    TransferIn {
        msg_id: MessageId,
        max_output_buf_size: u32,
    },
    TransferOut {
        msg_id: MessageId,
        request_id: RequestIdTransferInOut,
        max_output_buf_size: u32,
    },
}
