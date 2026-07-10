use alloc::collections::btree_map::{BTreeMap, Entry};
use alloc::vec::Vec;
use alloc::{boxed::Box, vec};
use ironrdp_core::{Decode as _, ReadCursor, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};

use crate::io::{
    DeviceAnnounce, DeviceText, InternalIoControlPacket, IoControlCompletionResult, IoControlPacket, ServerIoRequest,
    TransferInCompletionResult, TransferInPacket, TransferOutCompletionResult, TransferOutPacket, UsbRetractReason,
};
use crate::pdu::caps::RimExchangeCapabilityRequest;
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{InterfaceId, Mask, MessageId};
use crate::pdu::iface_manipulation::{InterfaceRelease, QueryInterfaceFailureResponse};
use crate::pdu::notify::ChannelCreated;
use crate::pdu::sink::NoAckIsochWriteJitterBufSizeInMs;
use crate::pdu::usb_dev::{
    CancelRequest, QueryDeviceText, RegisterRequestCallback, RetractDevice, TransferInRequest, TransferOutRequest,
};
use crate::pdu::utils::RequestId;
use crate::pdu::{UrbdrcClientControlPdu, UrbdrcClientDevicePdu};
use crate::{CHANNEL_NAME, InvalidDeviceInterfaceId};

pub struct UrbdrcControlServer {
    msg_id_alloc: IdAllocator,
    state: State,
    backend: Box<dyn UrbdrcControlServerBackend>,
}

pub trait UrbdrcControlServerBackend: Send {
    /// The server makes a new instance of a dynamic virtual channel for USB redirection.
    fn create_device_chan(&mut self) -> PduResult<()>;
}

#[derive(PartialEq)]
enum State {
    CapsExchanging,
    CapsExchanged,
    Ready,
}

impl UrbdrcControlServer {
    pub fn new(backend: Box<dyn UrbdrcControlServerBackend>) -> Self {
        Self {
            msg_id_alloc: IdAllocator::new(),
            state: State::CapsExchanging,
            backend,
        }
    }
}

struct IdAllocator {
    id: u32,
}

impl IdAllocator {
    #[inline]
    const fn new() -> Self {
        Self { id: 0 }
    }

    #[inline]
    const fn alloc(&mut self) -> MessageId {
        self.id += 1;
        self.id
    }
}

struct RequestIdAllocator {
    id: u32,
}

impl RequestIdAllocator {
    #[inline]
    const fn new() -> Self {
        Self { id: 0 }
    }

    #[inline]
    const fn alloc(&mut self) -> u32 {
        self.id += 1;
        if self.id > 0x7F_FF_FF_FF {
            self.id = 0;
        }
        self.id
    }
}

impl DvcProcessor for UrbdrcControlServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(vec![Box::new(RimExchangeCapabilityRequest {
            msg_id: self.msg_id_alloc.alloc(),
            capability: crate::pdu::caps::Capability::RimCapabilityVersion01,
        })])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = UrbdrcClientControlPdu::decode(&mut ReadCursor::new(payload)).map_err(|e| decode_err!(e))?;

        let mut resp: Vec<DvcMessage> = Vec::new();
        use UrbdrcClientControlPdu::*;
        match pdu {
            IfaceRelease(_iface_release_pdu) => Ok(resp),
            QueryIfaceReq(query_req_pdu) => {
                resp.push(Box::new(QueryInterfaceFailureResponse {
                    msg_id: query_req_pdu.msg_id,
                    iface_id: query_req_pdu.iface_id,
                }));
                Ok(resp)
            }
            Caps(_caps_response_pdu) => {
                if self.state != State::CapsExchanging {
                    return Err(pdu_other_err!("invalid state"));
                }
                resp.push(Box::new(InterfaceRelease {
                    iface_id: InterfaceId::CAPABILITIES.with_mask(Mask::None),
                    msg_id: self.msg_id_alloc.alloc(),
                }));
                resp.push(Box::new(ChannelCreated {
                    msg_id: self.msg_id_alloc.alloc(),
                    direction: crate::pdu::notify::Direction::ToClient,
                }));
                self.state = State::CapsExchanged;
                Ok(resp)
            }
            ChanCreated(_chan_created_pdu) => {
                if self.state != State::CapsExchanged {
                    return Err(pdu_other_err!("invalid state"));
                }
                resp.push(Box::new(InterfaceRelease {
                    msg_id: self.msg_id_alloc.alloc(),
                    iface_id: InterfaceId::NOTIFY_CLIENT.with_mask(Mask::Proxy),
                }));
                self.state = State::Ready;
                Ok(resp)
            }
            AddChan(_add_channel_pdu) => {
                if self.state != State::Ready {
                    return Err(pdu_other_err!("invalid state"));
                }
                self.backend.create_device_chan()?;
                Ok(resp)
            }
        }
    }
}

impl_as_any!(UrbdrcControlServer);

impl DvcServerProcessor for UrbdrcControlServer {}

pub trait UrbdrcDeviceServerBackend: Send {
    /// [Add Device Message][2.2.4.2]:
    ///
    /// After receiving the ADD_DEVICE message, the server creates a remote device instance that
    /// represents the client-side physical device.
    ///
    /// [2.2.4.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a26bcb6d-d45d-48a9-b9bd-22e0107d8393
    fn add_device(&mut self, device: DeviceAnnounce) -> PduResult<()>;

    /// [Query Device Text Response Message][2.2.6.6]:
    ///
    /// Delivers the device description returned by the client to the server backend.
    ///
    /// [2.2.6.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/acffdcfa-c792-40a4-a8ee-c545ea5b0a38
    fn device_text(&mut self, device_text: DeviceText);

    /// [IO Control Completion Message][2.2.7.1]:
    ///
    /// Completes the IO control request identified by `request_id`.
    ///
    /// [2.2.7.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b1722374-0658-47ba-8368-87bf9d3db4d4
    fn io_control_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: IoControlCompletionResult,
    ) -> PduResult<()>;

    /// [IO Control Completion Message][2.2.7.1]:
    ///
    /// Completes the internal IO control request identified by `request_id`.
    ///
    /// [2.2.7.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b1722374-0658-47ba-8368-87bf9d3db4d4
    fn internal_io_control_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: IoControlCompletionResult,
    ) -> PduResult<()>;

    /// [URB Completion Message][2.2.7.2] and [URB Completion No Data Message][2.2.7.3]:
    ///
    /// Completes the transfer-in request identified by `request_id`.
    ///
    /// [2.2.7.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5bfa9c84-a74b-4942-9d09-e770b21081eb
    /// [2.2.7.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/994fac8f-d258-47a6-aa35-48783abe49ec
    fn transfer_in_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: TransferInCompletionResult,
    ) -> PduResult<()>;

    /// [URB Completion No Data Message][2.2.7.3]:
    ///
    /// Completes the transfer-out request identified by `request_id`.
    ///
    /// [2.2.7.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/994fac8f-d258-47a6-aa35-48783abe49ec
    fn transfer_out_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: TransferOutCompletionResult,
    ) -> PduResult<()>;
}

pub struct UrbdrcDeviceServer {
    msg_alloc: IdAllocator,
    request_id_alloc: RequestIdAllocator,
    udev_iface: Option<InterfaceId>,
    comp_iface: InterfaceId,
    no_ack_isoch_write_jitter_buf_size: Option<NoAckIsochWriteJitterBufSizeInMs>,
    pending_io: BTreeMap<RequestId, Pending>,
    backend: Box<dyn UrbdrcDeviceServerBackend>,
}

enum Pending {
    IoCtl { max_output_buf_size: u32 },
    InternalIoCtl { max_output_buf_size: u32 },
    TransferIn { max_output_buf_size: u32 },
    TransferOut { max_output_buf_size: u32 },
}

impl UrbdrcDeviceServer {
    pub fn new(
        backend: Box<dyn UrbdrcDeviceServerBackend>,
        comp_iface: InterfaceId,
    ) -> Result<Self, InvalidDeviceInterfaceId<Box<dyn UrbdrcDeviceServerBackend>>> {
        if u32::from(comp_iface) <= u32::from(InterfaceId::NOTIFY_SERVER) {
            return Err(InvalidDeviceInterfaceId::new(backend));
        }

        Ok(Self {
            msg_alloc: IdAllocator::new(),
            request_id_alloc: RequestIdAllocator::new(),
            udev_iface: None,
            comp_iface,
            no_ack_isoch_write_jitter_buf_size: None,
            pending_io: BTreeMap::new(),
            backend,
        })
    }

    pub fn query_device_text(&mut self, text_type: u32, locale_id: u32) -> PduResult<DvcMessage> {
        let udev_iface = self.usb_device_iface()?;
        Ok(Box::new(QueryDeviceText {
            msg_id: self.msg_alloc.alloc(),
            udev_iface,
            text_type,
            locale_id,
        }))
    }

    /// [IO Control Message][2.2.6.3]:
    ///
    /// Builds an IO control request to be sent to the client-side physical device.
    ///
    /// [2.2.6.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/021733cb-8e3b-49ac-b3e3-f7a764b11141
    pub fn io_control(&mut self, io_control_packet: IoControlPacket) -> PduResult<ServerIoRequest> {
        let udev_iface = self.usb_device_iface()?;
        let request_id = self.request_id_alloc.alloc();
        let request = io_control_packet.into_pdu(self.msg_alloc.alloc(), request_id, udev_iface);

        request
            .check_output_buffer_size()
            .map_err(|_| pdu_other_err!("invalid IO_CONTROL output buffer size"))?;

        self.insert_pending_io(
            request_id,
            Pending::IoCtl {
                max_output_buf_size: request.output_buffer_size,
            },
        )?;

        Ok(ServerIoRequest {
            request_id,
            expects_completion: true,
            message: Box::new(request),
        })
    }

    /// [Internal IO Control Message][2.2.6.4]:
    ///
    /// Builds an internal IO control request to be sent to the client-side physical device.
    ///
    /// [2.2.6.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c3f3e320-336d-4d1b-84c9-51e0ed330ffe
    pub fn internal_io_control(
        &mut self,
        internal_io_ctl_packet: InternalIoControlPacket,
    ) -> PduResult<ServerIoRequest> {
        let udev_iface = self.usb_device_iface()?;
        let request_id = self.request_id_alloc.alloc();

        let request = internal_io_ctl_packet.into_pdu(self.msg_alloc.alloc(), request_id, udev_iface);
        self.insert_pending_io(
            request_id,
            Pending::InternalIoCtl {
                max_output_buf_size: request.output_buffer_size,
            },
        )?;

        Ok(ServerIoRequest {
            request_id,
            expects_completion: true,
            message: Box::new(request),
        })
    }

    /// [Transfer In Request][2.2.6.7]:
    ///
    /// Builds a transfer request that reads data from the client-side physical device.
    ///
    /// [2.2.6.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/e40f7738-bdd3-480f-a8bb-e1557a83a151
    pub fn transfer_in(&mut self, request: TransferInPacket) -> PduResult<ServerIoRequest> {
        let udev_iface = self.usb_device_iface()?;
        let request_id = self.request_id_alloc.alloc();
        let output_buffer_size = request.output_buffer_size;
        let ts_urb = request.ts_urb.into_ts_urb(request_id)?;
        let pdu = TransferInRequest {
            msg_id: self.msg_alloc.alloc(),
            udev_iface,
            ts_urb,
            output_buffer_size,
        };
        pdu.check_output_buffer_size()
            .map_err(|_| pdu_other_err!("invalid TRANSFER_IN_REQUEST output buffer size"))?;

        self.insert_pending_io(
            request_id,
            Pending::TransferIn {
                max_output_buf_size: output_buffer_size,
            },
        )?;

        Ok(ServerIoRequest {
            request_id,
            expects_completion: true,
            message: Box::new(pdu),
        })
    }

    /// [Transfer Out Request][2.2.6.8]:
    ///
    /// Builds a transfer request that writes data to the client-side physical device.
    ///
    /// [2.2.6.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6d6c85b2-47bb-4674-975a-dc7d8ed684cd
    pub fn transfer_out(&mut self, request: TransferOutPacket) -> PduResult<ServerIoRequest> {
        let udev_iface = self.usb_device_iface()?;
        let output_buffer_size =
            u32::try_from(request.output_buffer.len()).map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;

        let request_id = self.request_id_alloc.alloc();
        let no_ack = request.ts_urb.no_ack;
        let no_ack_isoch_write_jitter_buf_size = self
            .no_ack_isoch_write_jitter_buf_size
            .ok_or_else(|| pdu_other_err!("USB device capabilities uninitialized"))?;
        let ts_urb = request
            .ts_urb
            .into_ts_urb(request_id, no_ack_isoch_write_jitter_buf_size)?;
        let pdu = TransferOutRequest {
            msg_id: self.msg_alloc.alloc(),
            udev_iface,
            ts_urb,
            output_buffer: request.output_buffer,
        };

        if !no_ack {
            self.insert_pending_io(
                request_id,
                Pending::TransferOut {
                    max_output_buf_size: output_buffer_size,
                },
            )?;
        }

        Ok(ServerIoRequest {
            request_id,
            expects_completion: !no_ack,
            message: Box::new(pdu),
        })
    }

    pub fn cancel_request(&mut self, request_id: RequestId) -> PduResult<DvcMessage> {
        let udev_iface = self.usb_device_iface()?;
        Ok(Box::new(CancelRequest {
            msg_id: self.msg_alloc.alloc(),
            udev_iface,
            req_id: request_id,
        }))
    }

    pub fn retract_device(&mut self, reason: UsbRetractReason) -> PduResult<DvcMessage> {
        let udev_iface = self.usb_device_iface()?;
        self.pending_io.clear();
        self.no_ack_isoch_write_jitter_buf_size = None;
        Ok(Box::new(RetractDevice {
            msg_id: self.msg_alloc.alloc(),
            udev_iface,
            reason,
        }))
    }

    fn usb_device_iface(&self) -> PduResult<InterfaceId> {
        self.udev_iface
            .ok_or_else(|| pdu_other_err!("USB device uninitialized"))
    }

    fn insert_pending_io(&mut self, request_id: RequestId, pending: Pending) -> PduResult<()> {
        match self.pending_io.entry(request_id) {
            Entry::Vacant(entry) => {
                entry.insert(pending);
                Ok(())
            }
            Entry::Occupied(_) => Err(pdu_other_err!("request id collision")),
        }
    }

    fn handle_io_control_completion(
        &mut self,
        channel_id: u32,
        completion: IoControlCompletion,
    ) -> PduResult<Vec<DvcMessage>> {
        if completion.completion_iface != self.comp_iface {
            return Ok(Vec::new());
        }

        let IoControlCompletion {
            request_id,
            hresult,
            information,
            output_buffer_size,
            output_buffer,
            ..
        } = completion;

        let Some(pending) = self.pending_io.remove(&request_id) else {
            return Err(pdu_other_err!("completion mismatch"));
        };

        let (is_internal, max_output_buf_size) = match pending {
            Pending::IoCtl { max_output_buf_size } => (false, max_output_buf_size),
            Pending::InternalIoCtl { max_output_buf_size } => (true, max_output_buf_size),
            Pending::TransferIn { .. } | Pending::TransferOut { .. } => {
                return Err(pdu_other_err!("completion mismatch"));
            }
        };

        if output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }

        let result = IoControlCompletionResult {
            hresult,
            information,
            output_buffer,
        };

        if is_internal {
            self.backend
                .internal_io_control_completed(channel_id, request_id, result)?;
        } else {
            self.backend.io_control_completed(channel_id, request_id, result)?;
        }

        Ok(Vec::new())
    }

    fn handle_urb_completion(&mut self, channel_id: u32, completion: UrbCompletion) -> PduResult<Vec<DvcMessage>> {
        if completion.completion_iface != self.comp_iface {
            return Ok(Vec::new());
        }

        let request_id = RequestId::from(completion.req_id);

        let Some(Pending::TransferIn { max_output_buf_size }) = self.pending_io.remove(&request_id) else {
            return Err(pdu_other_err!("completion mismatch"));
        };

        let output_buffer_size =
            u32::try_from(completion.output_buffer.len()).map_err(|_| pdu_other_err!("convert usize to u32 failed"))?;
        if output_buffer_size > max_output_buf_size {
            return Err(pdu_other_err!("output buffer exceeds maximum amount"));
        }

        self.backend.transfer_in_completed(
            channel_id,
            request_id,
            TransferInCompletionResult {
                ts_urb_result: completion.ts_urb_result,
                hresult: completion.hresult,
                output_buffer: completion.output_buffer,
            },
        )?;

        Ok(Vec::new())
    }

    fn handle_urb_completion_no_data(
        &mut self,
        channel_id: u32,
        completion: UrbCompletionNoData,
    ) -> PduResult<Vec<DvcMessage>> {
        if completion.completion_iface != self.comp_iface {
            return Ok(Vec::new());
        }

        let request_id = RequestId::from(completion.req_id);
        let Some(pending) = self.pending_io.remove(&request_id) else {
            return Err(pdu_other_err!("completion mismatch"));
        };

        let is_transfer_out = match pending {
            Pending::TransferIn { .. } => {
                if completion.output_buffer_size != 0 {
                    return Err(pdu_other_err!("output buffer size must be zero"));
                }
                false
            }
            Pending::TransferOut { max_output_buf_size } => {
                if completion.output_buffer_size > max_output_buf_size {
                    return Err(pdu_other_err!("output buffer exceeds maximum amount"));
                }
                true
            }
            Pending::IoCtl { .. } | Pending::InternalIoCtl { .. } => {
                return Err(pdu_other_err!("completion mismatch"));
            }
        };

        if is_transfer_out {
            self.backend.transfer_out_completed(
                channel_id,
                request_id,
                TransferOutCompletionResult {
                    ts_urb_result: completion.ts_urb_result,
                    hresult: completion.hresult,
                    output_buffer_size: completion.output_buffer_size,
                },
            )?;
        } else {
            self.backend.transfer_in_completed(
                channel_id,
                request_id,
                TransferInCompletionResult {
                    ts_urb_result: completion.ts_urb_result,
                    hresult: completion.hresult,
                    output_buffer: Vec::new(),
                },
            )?;
        }

        Ok(Vec::new())
    }
}

impl DvcProcessor for UrbdrcDeviceServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(vec![Box::new(ChannelCreated {
            msg_id: self.msg_alloc.alloc(),
            direction: crate::pdu::notify::Direction::ToClient,
        })])
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = UrbdrcClientDevicePdu::decode(&mut ReadCursor::new(payload)).map_err(|e| decode_err!(e))?;
        let mut resp: Vec<DvcMessage> = Vec::new();

        use UrbdrcClientDevicePdu::*;
        match pdu {
            ChanCreated(_channel_created_pdu) => {
                resp.push(Box::new(InterfaceRelease {
                    msg_id: self.msg_alloc.alloc(),
                    iface_id: InterfaceId::NOTIFY_CLIENT.with_mask(Mask::Proxy),
                }));
                Ok(resp)
            }
            AddDev(add_dev_pdu) => {
                // In the case of the server receiving a duplicate interface ID, the server MUST
                // ignore the ADD_DEVICE message.
                if self.udev_iface.is_some() {
                    return Ok(resp);
                }
                let udev_iface = add_dev_pdu.usb_device;
                let no_ack_isoch_write_jitter_buf_size = add_dev_pdu.usb_device_caps.no_ack_isoch_write_jitter_buf_size;
                self.udev_iface = Some(udev_iface);

                let device = add_dev_pdu.try_into()?;

                self.backend.add_device(device)?;
                self.no_ack_isoch_write_jitter_buf_size = Some(no_ack_isoch_write_jitter_buf_size);
                resp.push(Box::new(InterfaceRelease {
                    msg_id: self.msg_alloc.alloc(),
                    iface_id: InterfaceId::DEVICE_SINK.with_mask(Mask::Proxy),
                }));
                resp.push(Box::new(RegisterRequestCallback {
                    msg_id: self.msg_alloc.alloc(),
                    udev_iface,
                    request_completion: Some(self.comp_iface),
                }));
                Ok(resp)
            }
            IfaceRelease(_iface_release_pdu) => Ok(resp),
            DevTextRsp(dev_text_rsp_pdu) => {
                let device_text = DeviceText {
                    hresult: dev_text_rsp_pdu.hresult,
                    description: dev_text_rsp_pdu
                        .device_description
                        .into_native()
                        .map_err(|e| pdu_other_err!("invalid device description").with_source(e))?,
                };
                self.backend.device_text(device_text);
                Ok(resp)
            }
            IoctlComp(ioctl_comp_pdu) => self.handle_io_control_completion(channel_id, ioctl_comp_pdu),
            UrbComp(urb_comp_pdu) => self.handle_urb_completion(channel_id, urb_comp_pdu),
            UrbCompNoData(urb_comp_no_data_pdu) => self.handle_urb_completion_no_data(channel_id, urb_comp_no_data_pdu),
            QueryIfaceReq(query_iface_req_pdu) => {
                resp.push(Box::new(QueryInterfaceFailureResponse {
                    msg_id: query_iface_req_pdu.msg_id,
                    iface_id: query_iface_req_pdu.iface_id,
                }));
                Ok(resp)
            }
        }
    }
}

impl_as_any!(UrbdrcDeviceServer);

impl DvcServerProcessor for UrbdrcDeviceServer {}
