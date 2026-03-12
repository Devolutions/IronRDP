use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

use crate::pdu::header::{InterfaceId, SharedMsgHeader};

pub mod caps;
pub mod chan_notify;
pub mod dev_sink;
pub mod req_complete;
pub mod usb_dev;

pub mod header;

pub mod utils;

pub mod ts_urb;

pub enum UrbdrcServerPdu {
    Caps(caps::RimExchangeCapabilityRequest),
    CancelReq(usb_dev::CancelRequest),
    RegReqCallback(usb_dev::RegisterRequestCallback),
}

// impl UrbdrcServerPdu {
//     pub fn decode<I>(src: &mut ReadCursor<'_>, device_ifaces: I) -> DecodeResult<Self>
//     where
//         I: IntoIterator,
//         I::Item: Into<InterfaceId>,
//     {
//         use UrbdrcServerPdu::*;
//
//         let header = SharedMsgHeader::decode(src)?;
//         match header.interface_id {
//             // InterfaceId::CAPABILITIES => Ok(Caps(caps::RimExchangeCapabilityRequest::decode(src, header)?)),
//         }
//     }
// }

impl Encode for UrbdrcServerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbdrcServerPdu::*;
        match self {
            Caps(rim_exchange_capability_request) => rim_exchange_capability_request.encode(dst),
            CancelReq(cancel_request) => cancel_request.encode(dst),
            RegReqCallback(register_request_callback) => register_request_callback.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        use UrbdrcServerPdu::*;
        match self {
            Caps(rim_exchange_capability_request) => rim_exchange_capability_request.name(),
            CancelReq(cancel_request) => cancel_request.name(),
            RegReqCallback(register_request_callback) => register_request_callback.name(),
        }
    }

    fn size(&self) -> usize {
        use UrbdrcServerPdu::*;
        match self {
            Caps(rim_exchange_capability_request) => rim_exchange_capability_request.size(),
            CancelReq(cancel_request) => cancel_request.size(),
            RegReqCallback(register_request_callback) => register_request_callback.size(),
        }
    }
}
