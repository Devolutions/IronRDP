//! Message packets from [\[MS-RDPEUSB\]][1], and helpers for encoding and decoding from wire.
//!
//! These messages are divided into [`UrbdrcServerPdu`] and [`UrbdrcClientPdu`].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};

use crate::pdu::caps::{RimExchangeCapabilityRequest, RimExchangeCapabilityResponse};
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{FunctionId, InterfaceId, Mask, SharedMsgHeader, unpack};
use crate::pdu::notify::ChannelCreated;
use crate::pdu::sink::{AddDevice, AddVirtualChannel};
use crate::pdu::usb_dev::{
    CancelRequest, InternalIoControl, IoControl, QueryDeviceText, QueryDeviceTextRsp, RegisterRequestCallback,
    RetractDevice, TransferInRequest, TransferOutRequest,
};

pub mod caps;
pub mod completion;
pub mod header;
pub mod iface_manipulation;
pub mod notify;
pub mod sink;
pub mod usb_dev;
pub mod utils;

/// A message sent from the server to the client.
pub enum UrbdrcServerPdu {
    Caps(RimExchangeCapabilityRequest),
    ChanCreated(ChannelCreated),
    CancelReq(CancelRequest),
    RegReqCb(RegisterRequestCallback),
    IoCtl(IoControl),
    InternalIoCtl(InternalIoControl),
    DevText(QueryDeviceText),
    TransferIn(TransferInRequest),
    TransferOut(TransferOutRequest),
    Retract(RetractDevice),
}

impl Decode<'_> for UrbdrcServerPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode_with_function_id(src)?;
        let f_id = header.function_id.expect("missing function id");

        match unpack(header.interface_id)? {
            (InterfaceId::CAPABILITIES, Mask::None) => {
                if f_id == FunctionId::RIM_EXCHANGE_CAPABILITY_REQUEST {
                    RimExchangeCapabilityRequest::decode(src, header).map(Self::Caps)
                } else {
                    Err(invalid_field_err!(
                        "SHARED_MSG_HEADER",
                        "invalid RIM_EXCHANGE_CAPABILITY_REQUEST header"
                    ))
                }
            }
            (InterfaceId::NOTIFY_CLIENT, Mask::Proxy) => {
                if f_id == FunctionId::CHANNEL_CREATED {
                    ChannelCreated::decode(src, header).map(Self::ChanCreated)
                } else {
                    Err(invalid_field_err!(
                        "SHARED_MSG_HEADER",
                        "invalid CHANNEL_CREATED header"
                    ))
                }
            }
            (udev_iface, Mask::Proxy) => match f_id {
                FunctionId::CANCEL_REQUEST => {
                    CancelRequest::decode(src, header.msg_id, udev_iface).map(Self::CancelReq)
                }
                FunctionId::REGISTER_REQUEST_CALLBACK => {
                    RegisterRequestCallback::decode(src, header.msg_id, udev_iface).map(Self::RegReqCb)
                }
                FunctionId::IO_CONTROL => IoControl::decode(src, header.msg_id, udev_iface).map(Self::IoCtl),
                FunctionId::INTERNAL_IO_CONTROL => {
                    InternalIoControl::decode(src, header.msg_id, udev_iface).map(Self::InternalIoCtl)
                }
                FunctionId::QUERY_DEVICE_TEXT => {
                    QueryDeviceText::decode(src, header.msg_id, udev_iface).map(Self::DevText)
                }
                FunctionId::TRANSFER_IN_REQUEST => {
                    TransferInRequest::decode(src, header.msg_id, udev_iface).map(Self::TransferIn)
                }
                FunctionId::TRANSFER_OUT_REQUEST => {
                    TransferOutRequest::decode(src, header.msg_id, udev_iface).map(Self::TransferOut)
                }
                FunctionId::RETRACT_DEVICE => RetractDevice::decode(src, header.msg_id, udev_iface).map(Self::Retract),
                _ => Err(invalid_field_err!(
                    "SHARED_MSG_HEADER::FunctionId",
                    "unsupported function id for USB device interface"
                )),
            },
            _ => Err(invalid_field_err!("SHARED_MSG_HEADER", "invalid header")),
        }
    }
}

macro_rules! fill_server_pdu_arms {
    ($pdu:expr, $($tokens:tt)*) => {{
        use UrbdrcServerPdu::*;
        match <&UrbdrcServerPdu>::from($pdu) {
            Caps(rim_exchange_capability_request) => rim_exchange_capability_request$($tokens)*,
            ChanCreated(channel_created) => channel_created$($tokens)*,
            CancelReq(cancel_request) => cancel_request$($tokens)*,
            RegReqCb(register_request_callback) => register_request_callback$($tokens)*,
            IoCtl(io_control) => io_control$($tokens)*,
            InternalIoCtl(internal_io_ctl) => internal_io_ctl$($tokens)*,
            DevText(query_device_text) => query_device_text$($tokens)*,
            TransferIn(transfer_in_request) => transfer_in_request$($tokens)*,
            TransferOut(transfer_out_request) => transfer_out_request$($tokens)*,
            Retract(retract_device) => retract_device$($tokens)*,
        }
    }};
}

impl Encode for UrbdrcServerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        fill_server_pdu_arms!(self, .encode(dst))
    }

    fn name(&self) -> &'static str {
        fill_server_pdu_arms!(self, .name())
    }

    fn size(&self) -> usize {
        fill_server_pdu_arms!(self, .size())
    }
}

/// A message sent from the client to the server.
pub enum UrbdrcClientPdu {
    Caps(RimExchangeCapabilityResponse),
    AddChan(AddVirtualChannel),
    AddDev(AddDevice),
    ChanCreated(ChannelCreated),
    DevTextRsp(QueryDeviceTextRsp),
    IoctlComp(IoControlCompletion),
    UrbComp(UrbCompletion),
    UrbCompNoData(UrbCompletionNoData),
}

impl Decode<'_> for UrbdrcClientPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;
        ensure_size!(in: src, size: 4 /* function id */);

        match unpack(header.interface_id)? {
            (InterfaceId::CAPABILITIES, Mask::None) => {
                RimExchangeCapabilityResponse::decode(src, header).map(Self::Caps)
            }
            (InterfaceId::DEVICE_SINK, Mask::Proxy) => match FunctionId(src.read_u32()) {
                FunctionId::ADD_VIRTUAL_CHANNEL => AddVirtualChannel::decode(src, header).map(Self::AddChan),
                FunctionId::ADD_DEVICE => AddDevice::decode(src, header).map(Self::AddDev),
                _ => Err(invalid_field_err!(
                    "SHARED_MSG_HEADER",
                    "invalid Device Sink interface header"
                )),
            },
            (InterfaceId::NOTIFY_SERVER, Mask::Proxy) => {
                if FunctionId(src.read_u32()) == FunctionId::CHANNEL_CREATED {
                    ChannelCreated::decode(src, header).map(Self::ChanCreated)
                } else {
                    Err(invalid_field_err!(
                        "SHARED_MSG_HEADER",
                        "invalid CHANNEL_CREATED header"
                    ))
                }
            }
            (udev_iface, Mask::Stub) => QueryDeviceTextRsp::decode(src, header.msg_id, udev_iface).map(Self::DevTextRsp),
            (udev_iface, Mask::Proxy) => match FunctionId(src.read_u32()) {
                FunctionId::IOCONTROL_COMPLETION => {
                    IoControlCompletion::decode(src, header.msg_id, udev_iface).map(Self::IoctlComp)
                }
                FunctionId::URB_COMPLETION => UrbCompletion::decode(src, header.msg_id, udev_iface).map(Self::UrbComp),
                FunctionId::URB_COMPLETION_NO_DATA => {
                    UrbCompletionNoData::decode(src, header.msg_id, udev_iface).map(Self::UrbCompNoData)
                }
                _ => Err(invalid_field_err!(
                    "SHARED_MSG_HEADER::InterfaceId",
                    "unknown interface id"
                )),
            },
            _ => Err(invalid_field_err!("SHARED_MSG_HEADER", "invalid header")),
        }
    }
}

macro_rules! fill_client_pdu_arms {
    ($pdu:expr, $($tokens:tt)*) => {{
        use UrbdrcClientPdu::*;
        match <&UrbdrcClientPdu>::from($pdu) {
            Caps(rim_exchange_capability_response) => rim_exchange_capability_response$($tokens)*,
            AddChan(add_virtual_channel) => add_virtual_channel$($tokens)*,
            AddDev(add_device) => add_device$($tokens)*,
            ChanCreated(channel_created) => channel_created$($tokens)*,
            DevTextRsp(query_device_text_rsp) => query_device_text_rsp$($tokens)*,
            IoctlComp(iocontrol_completion) => iocontrol_completion$($tokens)*,
            UrbComp(urb_completion) => urb_completion$($tokens)*,
            UrbCompNoData(urb_completion_no_data) => urb_completion_no_data$($tokens)*,
        }
    }};
}

impl Encode for UrbdrcClientPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        fill_client_pdu_arms!(self, .encode(dst))
    }

    fn name(&self) -> &'static str {
        fill_client_pdu_arms!(self, .name())
    }

    fn size(&self) -> usize {
        fill_client_pdu_arms!(self, .size())
    }
}
