use alloc::collections::btree_map::{BTreeMap, Entry};
use alloc::vec::Vec;
use alloc::{boxed::Box, vec};
use ironrdp_core::{Decode as _, ReadCursor, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};

use crate::pdu::caps::RimExchangeCapabilityRequest;
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{InterfaceId, Mask, MessageId};
use crate::pdu::iface_manipulation::{InterfaceRelease, QueryInterfaceFailureResponse};
use crate::pdu::notify::ChannelCreated;
use crate::pdu::sink::NoAckIsochWriteJitterBufSizeInMs;
use crate::pdu::usb_dev::{
    CancelRequest, InternalIoControl, IoControl, IoctlInternalUsb, QueryDeviceText, RegisterRequestCallback,
    RetractDevice, TransferInRequest, TransferOutRequest, UsbInternalIoctlCode, UsbRetractReason,
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
                    iface_id: InterfaceId::CAPABILITIES.with_mask(Mask::Proxy),
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

