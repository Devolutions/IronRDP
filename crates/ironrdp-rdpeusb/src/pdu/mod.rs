//! Message packets from [\[MS-RDPEUSB\]][1], and helpers for encoding and decoding from wire.
//!
//! These messages are divided into [`UrbdrcServerPdu`] and [`UrbdrcClientPdu`].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

use ironrdp_core::{
    Decode as _, DecodeError, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, invalid_field_err,
};

use crate::pdu::caps::{RimExchangeCapabilityRequest, RimExchangeCapabilityResponse};
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{FunctionId, FunctionIdErr, InterfaceId, SharedMsgHeader};
use crate::pdu::notify::ChannelCreated;
use crate::pdu::sink::{AddDevice, AddVirtualChannel};
use crate::pdu::usb_dev::{
    CancelRequest, InternalIoControl, IoControl, QueryDeviceText, QueryDeviceTextRsp, RegisterRequestCallback,
    RetractDevice, TransferInRequest, TransferOutRequest,
};

pub mod caps;
pub mod completion;
pub mod header;
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

impl UrbdrcServerPdu {
    pub fn decode<I>(src: &mut ReadCursor<'_>, usb_device_s: I) -> DecodeResult<Self>
    where
        I: IntoIterator<Item: PartialEq<InterfaceId>>,
    {
        let header = SharedMsgHeader::decode(src)?;
        let f_id = header.function_id.ok_or_else(|| {
            let e: DecodeError = invalid_field_err!("SHARED_MSG_HEADER::FunctionId", "is absent");
            e.with_source(FunctionIdErr::Missing)
        })?;

        match header.interface_id {
            InterfaceId::CAPABILITIES => RimExchangeCapabilityRequest::decode(src, header).map(Self::Caps),
            InterfaceId::NOTIFY_CLIENT => {
                if f_id == FunctionId::CHANNEL_CREATED {
                    ChannelCreated::decode(src, header).map(Self::ChanCreated)
                } else {
                    let e: DecodeError =
                        invalid_field_err!("CHANNEL_CREATED::SHARED_MSG_HEADER::FunctionId", "is not: 0x100");
                    Err(e.with_source(FunctionIdErr::InvalidForInterface(InterfaceId::NOTIFY_CLIENT, f_id)))
                }
            }
            id if usb_device_s.into_iter().any(|iface| iface == id) => match f_id {
                FunctionId::CANCEL_REQUEST => CancelRequest::decode(src, header).map(Self::CancelReq),
                FunctionId::REGISTER_REQUEST_CALLBACK => {
                    RegisterRequestCallback::decode(src, header).map(Self::RegReqCb)
                }
                FunctionId::IO_CONTROL => IoControl::decode(src, header).map(Self::IoCtl),
                FunctionId::INTERNAL_IO_CONTROL => InternalIoControl::decode(src, header).map(Self::InternalIoCtl),
                FunctionId::QUERY_DEVICE_TEXT => QueryDeviceText::decode(src, header).map(Self::DevText),
                FunctionId::TRANSFER_IN_REQUEST => TransferInRequest::decode(src, header).map(Self::TransferIn),
                FunctionId::TRANSFER_OUT_REQUEST => TransferOutRequest::decode(src, header).map(Self::TransferOut),
                FunctionId::RETRACT_DEVICE => RetractDevice::decode(src, header).map(Self::Retract),
                _ => {
                    let e: DecodeError = invalid_field_err!(
                        "SHARED_MSG_HEADER::FunctionId (USB Devices Interface)",
                        "is not one of: 0x100 (CANCEL_REQUEST), 0x101 (REGISTER_REQUEST_CALLBACK), \
                            0x102 (IO_CONTROL), 0x103 (INTERNAL_IO_CONTROL), 0x104 (QUERY_DEVICE_TEXT), \
                            0x105 (TRANSFER_IN_REQUEST), 0x106 (TRANSFER_OUT_REQUEST), 0x107 (RETRACT_DEVICE)"
                    );
                    Err(e.with_source(FunctionIdErr::InvalidForInterface(id, f_id)))
                }
            },
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER::InterfaceId",
                "server sent message on an interface that is currently closed, or not supposed to be used by the server"
            )),
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

impl UrbdrcClientPdu {
    pub fn decode<I>(src: &mut ReadCursor<'_>, usb_dev_s: I, completion_s: I) -> DecodeResult<Self>
    where
        I: IntoIterator<Item: PartialEq<InterfaceId>>,
    {
        let header = SharedMsgHeader::decode(src)?;
        match header.interface_id {
            InterfaceId::CAPABILITIES => RimExchangeCapabilityResponse::decode(src, header).map(Self::Caps),
            InterfaceId::DEVICE_SINK => match header.function_id {
                Some(FunctionId::ADD_VIRTUAL_CHANNEL) => AddVirtualChannel::decode(src, header).map(Self::AddChan),
                Some(FunctionId::ADD_DEVICE) => AddDevice::decode(src, header).map(Self::AddDev),
                Some(f_id) => {
                    let e: DecodeError = invalid_field_err!(
                        "SHARED_MSG_HEADER::FunctionId (Device Sink)",
                        "is not one of: 0x100 (ADD_VIRTUAL_CHANNEL), 0x101 (ADD_DEVICE)"
                    );
                    Err(e.with_source(FunctionIdErr::InvalidForInterface(InterfaceId::DEVICE_SINK, f_id)))
                }
                None => {
                    let e: DecodeError = invalid_field_err!("SHARED_MSG_HEADER::FunctionId (Device Sink)", "is absent");
                    Err(e.with_source(FunctionIdErr::Missing))
                }
            },
            InterfaceId::NOTIFY_SERVER => {
                const FIELD: &str = "CHANNEL_CREATED::SHARED_MSG_HEADER::FunctionId (Device Sink)";
                match header.function_id {
                    Some(FunctionId::CHANNEL_CREATED) => ChannelCreated::decode(src, header).map(Self::ChanCreated),
                    Some(f_id) => {
                        let e: DecodeError = invalid_field_err!(FIELD, "is not: 0x100");
                        Err(e.with_source(FunctionIdErr::InvalidForInterface(InterfaceId::NOTIFY_SERVER, f_id)))
                    }
                    None => {
                        let e: DecodeError = invalid_field_err!(FIELD, "is absent");
                        Err(e.with_source(FunctionIdErr::Missing))
                    }
                }
            }
            id if usb_dev_s.into_iter().any(|iface| iface == id) => match header.function_id {
                Some(_) => {
                    let e: DecodeError =
                        invalid_field_err!("QUERY_DEVICE_TEXT_RSP::SHARED_MSG_HEADER::FunctionId", "is not absent");
                    Err(e.with_source(FunctionIdErr::NotAbsent))
                }
                None => QueryDeviceTextRsp::decode(src, header).map(Self::DevTextRsp),
            },
            id if completion_s.into_iter().any(|iface| iface == id) => match header.function_id {
                Some(FunctionId::IOCONTROL_COMPLETION) => IoControlCompletion::decode(src, header).map(Self::IoctlComp),
                Some(FunctionId::URB_COMPLETION) => UrbCompletion::decode(src, header).map(Self::UrbComp),
                Some(FunctionId::URB_COMPLETION_NO_DATA) => {
                    UrbCompletionNoData::decode(src, header).map(Self::UrbCompNoData)
                }
                Some(f) => {
                    let e: DecodeError = invalid_field_err!(
                        "SHARED_MSG_HEADER::FunctionId (Request Completion)",
                        "is not one of: 0x100 (IOCONTROL_COMPLETION), 0x101 (URB_COMPLETION), 0x102 (URB_COMPLETION_NO_DATA)"
                    );
                    Err(e.with_source(FunctionIdErr::InvalidForInterface(id, f)))
                }
                None => {
                    let e: DecodeError =
                        invalid_field_err!("SHARED_MSG_HEADER::FunctionId (Request Completion)", "is missing");
                    Err(e.with_source(FunctionIdErr::Missing))
                }
            },
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER::InterfaceId",
                "client sent message on an interface that is currently closed, or not supposed to be used by the client"
            )),
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
