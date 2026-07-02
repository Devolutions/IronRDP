use alloc::collections::btree_map::{BTreeMap, Entry};
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};
use ironrdp_core::{Decode as _, ReadCursor, impl_as_any};
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};

use crate::io::device::add_device_from_info;
use crate::io::{
    DeviceText, InternalIoControlPacket, IoControlCompletionResult, IoControlPacket, TransferInCompletionResult,
    TransferInPacket, TransferOutCompletionResult, TransferOutPacket, device::DeviceInfo,
};
use crate::pdu::UrbdrcServerDevicePdu;
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{InterfaceId, Mask, MessageId};
use crate::pdu::iface_manipulation::{InterfaceRelease, QueryInterfaceFailureResponse};
use crate::pdu::sink::AddVirtualChannel;
use crate::pdu::usb_dev::QueryDeviceTextRsp;
use crate::pdu::utils::{RequestId, RequestIdTransferInOut};
use crate::pdu::{
    UrbdrcServerControlPdu,
    caps::{Capability, RimExchangeCapabilityResponse},
    notify::ChannelCreated,
};
use crate::{CHANNEL_NAME, InvalidDeviceInterfaceId};

const ADD_VIRTUAL_CHANNEL_MSG_ID: u32 = 0;

pub trait DeviceManagerBackend: Send {
    /// Called when the first URBDRC DVC is assigned as the control DVC.
    ///
    /// This happens from listener.create(channel_id), before the DVC is fully open.
    fn control_channel_assigned(&mut self, channel_id: u32);

    /// Called for each later URBDRC DVC create request.
    ///
    /// The manager should pop the pending device that caused ADD_VIRTUAL_CHANNEL
    fn take_device_for_channel(&mut self, channel_id: u32) -> Option<Box<dyn UrbdrcDeviceBackend>>;
}

pub struct UrbdrcListener {
    on_capability_exchanged: Option<OnCapabilityExchanged>,
    device_man: Box<dyn DeviceManagerBackend>,
    iface_man: InterfaceAlloc,
}

impl UrbdrcListener {
    pub fn new(callback: OnCapabilityExchanged, device_man: Box<dyn DeviceManagerBackend>) -> Self {
        Self {
            on_capability_exchanged: Some(callback),
            device_man,
            iface_man: InterfaceAlloc::new(),
        }
    }
}

struct InterfaceAlloc {
    id: u32,
}

impl InterfaceAlloc {
    #[inline]
    const fn new() -> Self {
        Self { id: 3 }
    }

    #[inline]
    const fn alloc(&mut self) -> Option<InterfaceId> {
        self.id += 1;
        if self.id > 0x3F_FF_FF_FF {
            None
        } else {
            Some(InterfaceId::from_raw(self.id))
        }
    }
}

impl DvcChannelListener for UrbdrcListener {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn create(&mut self, channel_id: u32) -> Option<Box<dyn DvcProcessor>> {
        if let Some(callback) = self.on_capability_exchanged.take() {
            self.device_man.control_channel_assigned(channel_id);
            Some(Box::new(UrbdrcControlClient::new(callback)))
        } else {
            let udev_iface = self.iface_man.alloc()?;
            #[expect(clippy::as_conversions)]
            self.device_man.take_device_for_channel(channel_id).map(|backend| {
                Box::new(UrbdrcDeviceClient::new(udev_iface, backend).expect("invalid interface id"))
                    as Box<dyn DvcProcessor>
            })
        }
    }
}

/// A client for the URBDRC Control Virtual Channel.
pub struct UrbdrcControlClient {
    /// Spec [3.1]:
    /// Exchange-completed event: Signifies that the capability exchange is completed, that is,
    /// the client has sent a Channel Created message.
    ///
    /// [3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/511b4cd7-1940-4631-90ac-bf2189ba6735
    on_capability_exchanged: Option<OnCapabilityExchanged>,
}

type OnCapabilityExchanged = Box<dyn FnOnce() -> PduResult<Vec<DvcMessage>> + Send>;

impl UrbdrcControlClient {
    /// Create a new [UrbdrcControlClient] with the given callback.
    ///
    /// The `callback` will be called when the capability exchange is completed and the channel is
    /// ready to redirect new devices.
    pub fn new(callback: OnCapabilityExchanged) -> Self {
        Self {
            on_capability_exchanged: Some(callback),
        }
    }

    /// Whether the channel is ready for add virtual channel.
    pub const fn ready(&self) -> bool {
        self.on_capability_exchanged.is_none()
    }

    /// Spec [3.3.5.1.1]:
    ///
    /// The client sends the ADD_VIRTUAL_CHANNEL message to server to request the server to create a
    /// new instance of dynamic virtual channel for USB redirection. The client sends this message
    /// for every USB device to be redirected. This isolates messages for each USB device in its own
    /// instance of a dynamic virtual channel.
    ///
    /// [3.3.5.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c7b1920a-d632-46d2-b62a-5c7e53570628
    pub fn add_virtual_channel(&self) -> PduResult<DvcMessage> {
        if !self.ready() {
            return Err(pdu_other_err!("is not ready for ADD_VIRTUAL_CHANNEL"));
        }
        Ok(Box::new(AddVirtualChannel {
            msg_id: ADD_VIRTUAL_CHANNEL_MSG_ID,
        }))
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
                if iface_id == InterfaceId::NOTIFY_CLIENT.with_mask(Mask::Proxy)
                    && let Some(callback) = self.on_capability_exchanged.take()
                {
                    // NOTE: MS-RDPEUSB does not normatively define RIMCALL_RELEASE as a
                    // server-ready-proceed barrier; the semantic comes from observed Windows
                    // urbdrc-server behavior. Pattern matches FreeRDP urbdrc_main.c since 2012
                    // (commit fa4d8fca1be, Atrust contribution). Two sync points: control DVC
                    // (server -> client ADD_VIRTUAL_CHANNEL); device DVC (server -> client
                    // ADD_DEVICE).
                    callback()
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
    fn device_info(&mut self, channel_id: u32) -> PduResult<DeviceInfo>;

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
    fn query_device_text(&mut self, channel_id: u32, text_type: u32, locale_id: u32) -> PduResult<Option<DeviceText>>;

    /// Process an `IoControl` request.
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn io_control(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: IoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>>;

    /// Process an `InternalIoControl` request.
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn internal_io_control(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: InternalIoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>>;

    /// Process a `TransferInRequest`.
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn transfer_in(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferInPacket,
    ) -> PduResult<Option<TransferInCompletionResult>>;

    /// Process a `TransferOutRequest`.
    ///
    /// Returning [`None`] means the request remains pending and no immediate completion is sent.
    fn transfer_out(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<Option<TransferOutCompletionResult>>;

    /// Process a no_ack `TransferOutRequest`.
    fn transfer_out_no_ack(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<()>;

    /// [Processing a Retract Device Message][3.3.5.3.8]:
    ///
    /// After receiving the RETRACT_DEVICE message, the client SHOULD terminate the dynamic channel
    /// and stop redirecting the physical USB device.
    ///
    /// [3.3.5.3.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/77dc8e12-ddd6-4cb8-a3cc-247aacea7d6f
    fn retract(&mut self, channel_id: u32) -> PduResult<()>;
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
    pub fn new(
        udev_iface: InterfaceId,
        backend: Box<dyn UrbdrcDeviceBackend>,
    ) -> Result<Self, InvalidDeviceInterfaceId<Box<dyn UrbdrcDeviceBackend>>> {
        if u32::from(udev_iface) <= u32::from(InterfaceId::NOTIFY_SERVER) {
            return Err(InvalidDeviceInterfaceId::new(backend));
        }
        Ok(Self {
            ready_for_io: false,
            udev_iface,
            request_completion: None,
            backend,
            pending_io: BTreeMap::new(),
        })
    }

    pub const fn ready_for_io(&self) -> bool {
        self.ready_for_io
    }

    pub const fn udev_iface(&self) -> InterfaceId {
        self.udev_iface
    }

    fn completion_iface_and_entry(
        &mut self,
        request_id: RequestId,
    ) -> PduResult<(
        InterfaceId,
        alloc::collections::btree_map::OccupiedEntry<'_, u32, Pending>,
    )> {
        let Some(completion_iface) = self.request_completion else {
            return Err(pdu_other_err!("request completion uninitialized"));
        };
        let Entry::Occupied(entry) = self.pending_io.entry(request_id) else {
            return Err(pdu_other_err!("completion mismatch"));
        };
        Ok((completion_iface, entry))
    }

    pub fn io_ctl_completion(
        &mut self,
        request_id: RequestId,
        response: IoControlCompletionResult,
    ) -> PduResult<DvcMessage> {
        let (completion_iface, entry) = self.completion_iface_and_entry(request_id)?;
        let (msg_id, max_output_buf_size) = match entry.get() {
            Pending::IoCtl {
                msg_id,
                max_output_buf_size,
            } => (*msg_id, *max_output_buf_size),
            _ => return Err(pdu_other_err!("completion mismatch")),
        };

        let output_buffer_size = check_output_buffer_size(response.output_buffer.len(), max_output_buf_size)?;
        entry.remove();

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
        response: IoControlCompletionResult,
    ) -> PduResult<DvcMessage> {
        let (completion_iface, entry) = self.completion_iface_and_entry(request_id)?;
        let (msg_id, max_output_buf_size) = match entry.get() {
            Pending::InternalIoCtl {
                msg_id,
                max_output_buf_size,
            } => (*msg_id, *max_output_buf_size),
            _ => return Err(pdu_other_err!("completion mismatch")),
        };

        let output_buffer_size = check_output_buffer_size(response.output_buffer.len(), max_output_buf_size)?;
        entry.remove();

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

    pub fn transfer_in_completion(
        &mut self,
        request_id: RequestId,
        response: TransferInCompletionResult,
    ) -> PduResult<DvcMessage> {
        let (completion_iface, entry) = self.completion_iface_and_entry(request_id)?;
        let (msg_id, max_output_buf_size) = match entry.get() {
            Pending::TransferIn {
                msg_id,
                max_output_buf_size,
            } => (*msg_id, *max_output_buf_size),
            _ => return Err(pdu_other_err!("completion mismatch")),
        };

        let output_buffer_size = check_output_buffer_size(response.output_buffer.len(), max_output_buf_size)?;
        entry.remove();

        #[expect(
            clippy::missing_panics_doc,
            reason = "panic is unreachable unless the pending transfer-key invariant is broken"
        )]
        let req_id = RequestIdTransferInOut::try_from(request_id)
            .expect("pending TransferIn request id must be a TS_URB request id");

        if response.output_buffer.is_empty() {
            Ok(Box::new(UrbCompletionNoData {
                msg_id,
                completion_iface,
                req_id,
                ts_urb_result: response.ts_urb_result,
                hresult: response.hresult,
                output_buffer_size,
            }))
        } else {
            Ok(Box::new(UrbCompletion {
                msg_id,
                completion_iface,
                req_id,
                ts_urb_result: response.ts_urb_result,
                hresult: response.hresult,
                output_buffer: response.output_buffer,
            }))
        }
    }

    pub fn transfer_out_completion(
        &mut self,
        request_id: RequestId,
        response: TransferOutCompletionResult,
    ) -> PduResult<DvcMessage> {
        let (completion_iface, entry) = self.completion_iface_and_entry(request_id)?;
        let (msg_id, max_output_buf_size) = match entry.get() {
            Pending::TransferOut {
                msg_id,
                max_output_buf_size,
            } => (*msg_id, *max_output_buf_size),
            _ => return Err(pdu_other_err!("completion mismatch")),
        };

        if response.output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }

        entry.remove();

        #[expect(
            clippy::missing_panics_doc,
            reason = "panic is unreachable unless the pending transfer-key invariant is broken"
        )]
        let req_id = RequestIdTransferInOut::try_from(request_id)
            .expect("pending TransferOut request id must be a TS_URB request id");

        Ok(Box::new(UrbCompletionNoData {
            msg_id,
            completion_iface,
            req_id,
            ts_urb_result: response.ts_urb_result,
            hresult: response.hresult,
            output_buffer_size: response.output_buffer_size,
        }))
    }
}

fn check_output_buffer_size(output_buffer_size: usize, max_output_buf_size: u32) -> PduResult<u32> {
    let output_buffer_size =
        u32::try_from(output_buffer_size).map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
    if output_buffer_size > max_output_buf_size {
        return Err(pdu_other_err!("output buffer exceeds maximum amount"));
    }
    Ok(output_buffer_size)
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
                    let device_info = self.backend.device_info(channel_id)?;
                    let add_device = add_device_from_info(self.udev_iface, &device_info)?;
                    self.ready_for_io = true;

                    Ok(vec![Box::new(add_device)])
                } else {
                    Ok(Vec::new())
                }
            }
            // SPEC [3.1.5]: Out-of-sequence packets are packets that do not adhere to the rules in
            // sections 3.2.5 and 3.3.5. Malformed and out-of-sequence packets MUST be ignored by
            // the server and the client.
            //
            // [3.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/f31cc9ef-a8c3-4a4d-b64d-f027ed0752b0
            CancelReq(cancel_req_pdu) => {
                if !self.ready_for_io || cancel_req_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                if self.pending_io.remove(&cancel_req_pdu.req_id).is_some() {
                    self.backend.cancel_request(cancel_req_pdu.req_id, channel_id);
                }
                Ok(Vec::new())
            }
            RegReqCb(register_request_callback_pdu) => {
                if !self.ready_for_io || register_request_callback_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                self.request_completion = register_request_callback_pdu.request_completion;
                Ok(Vec::new())
            }
            Retract(retract_pdu) => {
                if !self.ready_for_io || retract_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                self.backend.retract(channel_id)?;
                self.ready_for_io = false;
                self.request_completion = None;
                self.pending_io.clear();
                Ok(Vec::new())
            }
            DevText(dev_text_pdu) => {
                if !self.ready_for_io || dev_text_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                if let Some(device_text) =
                    self.backend
                        .query_device_text(channel_id, dev_text_pdu.text_type, dev_text_pdu.locale_id)?
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
                if !self.ready_for_io || io_ctl_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                let msg_id = io_ctl_pdu.msg_id;
                let request_id = io_ctl_pdu.req_id;
                let max_output_buf_size = io_ctl_pdu.output_buffer_size;
                if self.pending_io.contains_key(&request_id) {
                    return Ok(Vec::new());
                }
                let Some(completion_iface) = self.request_completion else {
                    return Ok(Vec::new());
                };

                let io_ctl_packet = io_ctl_pdu.into();
                if let Some(io_ctl_response) = self.backend.io_control(channel_id, request_id, io_ctl_packet)? {
                    let output_buffer_size =
                        check_output_buffer_size(io_ctl_response.output_buffer.len(), max_output_buf_size)?;
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
                if !self.ready_for_io || internal_io_ctl_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                let msg_id = internal_io_ctl_pdu.msg_id;
                let request_id = internal_io_ctl_pdu.req_id;
                let max_output_buf_size = internal_io_ctl_pdu.output_buffer_size;
                if self.pending_io.contains_key(&request_id) {
                    return Ok(Vec::new());
                }
                let Some(completion_iface) = self.request_completion else {
                    return Ok(Vec::new());
                };

                let internal_io_ctl_packet = internal_io_ctl_pdu.try_into()?;
                if let Some(internal_io_ctl_response) =
                    self.backend
                        .internal_io_control(channel_id, request_id, internal_io_ctl_packet)?
                {
                    let output_buffer_size =
                        check_output_buffer_size(internal_io_ctl_response.output_buffer.len(), max_output_buf_size)?;
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
                if !self.ready_for_io || transfer_in_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                let msg_id = transfer_in_pdu.msg_id;
                let max_output_buf_size = transfer_in_pdu.output_buffer_size;
                let request_id = transfer_in_pdu.request_id();
                if self.pending_io.contains_key(&request_id.into()) {
                    return Ok(Vec::new());
                }
                let Some(completion_iface) = self.request_completion else {
                    return Ok(Vec::new());
                };

                let transfer_in = TransferInPacket {
                    ts_urb: transfer_in_pdu.ts_urb.into(),
                    output_buffer_size: transfer_in_pdu.output_buffer_size,
                };

                if let Some(urb_response) = self.backend.transfer_in(channel_id, request_id.into(), transfer_in)? {
                    let output_buffer_size =
                        check_output_buffer_size(urb_response.output_buffer.len(), max_output_buf_size)?;
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
                if !self.ready_for_io || transfer_out_pdu.udev_iface != self.udev_iface {
                    return Ok(Vec::new());
                }
                let msg_id = transfer_out_pdu.msg_id;
                let output_buffer_size = u32::try_from(transfer_out_pdu.output_buffer.len())
                    .map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
                let request_id = transfer_out_pdu.ts_urb.header.req_id;
                let no_ack = transfer_out_pdu.ts_urb.header.no_ack;
                if self.pending_io.contains_key(&request_id.into()) {
                    return Ok(Vec::new());
                }

                let transfer_out = TransferOutPacket {
                    ts_urb: transfer_out_pdu.ts_urb.into(),
                    output_buffer: transfer_out_pdu.output_buffer,
                };
                if no_ack {
                    self.backend
                        .transfer_out_no_ack(channel_id, request_id.into(), transfer_out)?;
                    Ok(Vec::new())
                } else {
                    let Some(completion_iface) = self.request_completion else {
                        return Ok(Vec::new());
                    };
                    if let Some(urb_response) =
                        self.backend.transfer_out(channel_id, request_id.into(), transfer_out)?
                    {
                        if urb_response.output_buffer_size > output_buffer_size {
                            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
                        }
                        Ok(vec![Box::new(UrbCompletionNoData {
                            msg_id,
                            completion_iface,
                            req_id: request_id,
                            ts_urb_result: urb_response.ts_urb_result,
                            hresult: urb_response.hresult,
                            output_buffer_size: urb_response.output_buffer_size,
                        })])
                    } else {
                        self.pending_io.insert(
                            request_id.into(),
                            Pending::TransferOut {
                                msg_id,
                                max_output_buf_size: output_buffer_size,
                            },
                        );
                        Ok(Vec::new())
                    }
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
        max_output_buf_size: u32,
    },
}
