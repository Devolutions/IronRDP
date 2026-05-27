//! Message packets from [\[MS-RDPEUSB\]][1], and helpers for encoding and decoding from wire.
//!
//! These messages are split into four enums by direction (server, client) and DVC role
//! (the singleton control DVC vs. per-device DVCs): [`UrbdrcServerControlPdu`],
//! [`UrbdrcServerDevicePdu`], [`UrbdrcClientControlPdu`], [`UrbdrcClientDevicePdu`].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};

use crate::pdu::caps::{RimExchangeCapabilityRequest, RimExchangeCapabilityResponse};
use crate::pdu::completion::{IoControlCompletion, UrbCompletion, UrbCompletionNoData};
use crate::pdu::header::{FunctionId, InterfaceId, Mask, SharedMsgHeader, unpack};
use crate::pdu::iface_manipulation::{InterfaceRelease, QueryInterfaceRequest};
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
pub enum UrbdrcServerControlPdu {
    Caps(RimExchangeCapabilityRequest),
    ChanCreated(ChannelCreated),
    IfaceRelease(InterfaceRelease),
    QueryIfaceReq(QueryInterfaceRequest),
}

impl UrbdrcServerControlPdu {
    fn decode_caps(src: &mut ReadCursor<'_>, f_id: FunctionId, header: SharedMsgHeader) -> DecodeResult<Self> {
        match f_id {
            FunctionId::RIM_EXCHANGE_CAPABILITY_REQUEST => {
                RimExchangeCapabilityRequest::decode(src, header).map(Self::Caps)
            }
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid RIM_EXCHANGE_CAPABILITY_REQUEST header"
            )),
        }
    }

    fn decode_notification(src: &mut ReadCursor<'_>, f_id: FunctionId, header: SharedMsgHeader) -> DecodeResult<Self> {
        match f_id {
            FunctionId::CHANNEL_CREATED => ChannelCreated::decode(src, header).map(Self::ChanCreated),
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid CHANNEL_CREATED header"
            )),
        }
    }
}

pub enum UrbdrcServerDevicePdu {
    ChanCreated(ChannelCreated),
    IfaceRelease(InterfaceRelease),
    QueryIfaceReq(QueryInterfaceRequest),
    CancelReq(CancelRequest),
    RegReqCb(RegisterRequestCallback),
    IoCtl(IoControl),
    InternalIoCtl(InternalIoControl),
    DevText(QueryDeviceText),
    TransferIn(TransferInRequest),
    TransferOut(TransferOutRequest),
    Retract(RetractDevice),
}

impl UrbdrcServerDevicePdu {
    fn decode_notification(src: &mut ReadCursor<'_>, f_id: FunctionId, header: SharedMsgHeader) -> DecodeResult<Self> {
        match f_id {
            FunctionId::CHANNEL_CREATED => ChannelCreated::decode(src, header).map(Self::ChanCreated),
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid CHANNEL_CREATED header"
            )),
        }
    }
}

impl Decode<'_> for UrbdrcServerControlPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode_with_function_id(src)?;
        let f_id = header.function_id.expect("missing function id");

        match unpack(header.iface_id)? {
            (InterfaceId::CAPABILITIES, Mask::None) => Self::decode_caps(src, f_id, header),
            (InterfaceId::NOTIFY_CLIENT, Mask::Proxy) => Self::decode_notification(src, f_id, header),
            _ => Err(invalid_field_err!("SHARED_MSG_HEADER", "invalid header")),
        }
    }
}

impl Decode<'_> for UrbdrcServerDevicePdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode_with_function_id(src)?;
        let f_id = header.function_id.expect("missing function id");

        match unpack(header.iface_id)? {
            (InterfaceId::NOTIFY_CLIENT, Mask::Proxy) => Self::decode_notification(src, f_id, header),
            (udev_iface, Mask::Proxy) => match f_id {
                FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
                FunctionId::RIMCALL_QUERYINTERFACE => {
                    QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq)
                }
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

macro_rules! fill_server_ctl_pdu_arms {
    ($pdu:expr, $($tokens:tt)*) => {{
        use UrbdrcServerControlPdu::*;
        match <&UrbdrcServerControlPdu>::from($pdu) {
            Caps(rim_exchange_capability_request) => rim_exchange_capability_request$($tokens)*,
            ChanCreated(channel_created) => channel_created$($tokens)*,
            IfaceRelease(iface_release) => iface_release$($tokens)*,
            QueryIfaceReq(query_iface_req) => query_iface_req$($tokens)*,
        }
    }};
}

macro_rules! fill_server_dev_pdu_arms {
    ($pdu:expr, $($tokens:tt)*) => {{
        use UrbdrcServerDevicePdu::*;
        match <&UrbdrcServerDevicePdu>::from($pdu) {
            ChanCreated(channel_created) => channel_created$($tokens)*,
            IfaceRelease(iface_release) => iface_release$($tokens)*,
            QueryIfaceReq(query_iface_req) => query_iface_req$($tokens)*,
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

impl Encode for UrbdrcServerControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        fill_server_ctl_pdu_arms!(self, .encode(dst))
    }

    fn name(&self) -> &'static str {
        fill_server_ctl_pdu_arms!(self, .name())
    }

    fn size(&self) -> usize {
        fill_server_ctl_pdu_arms!(self, .size())
    }
}

impl Encode for UrbdrcServerDevicePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        fill_server_dev_pdu_arms!(self, .encode(dst))
    }

    fn name(&self) -> &'static str {
        fill_server_dev_pdu_arms!(self, .name())
    }

    fn size(&self) -> usize {
        fill_server_dev_pdu_arms!(self, .size())
    }
}

/// A message sent from the client to the server.
pub enum UrbdrcClientControlPdu {
    Caps(RimExchangeCapabilityResponse),
    ChanCreated(ChannelCreated),
    AddChan(AddVirtualChannel),
    IfaceRelease(InterfaceRelease),
    QueryIfaceReq(QueryInterfaceRequest),
}

pub enum UrbdrcClientDevicePdu {
    ChanCreated(ChannelCreated),
    AddDev(AddDevice),
    DevTextRsp(QueryDeviceTextRsp),
    IoctlComp(IoControlCompletion),
    UrbComp(UrbCompletion),
    UrbCompNoData(UrbCompletionNoData),
    IfaceRelease(InterfaceRelease),
    QueryIfaceReq(QueryInterfaceRequest),
}

impl UrbdrcClientControlPdu {
    fn decode_sink(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* function id */);
        let f_id = FunctionId(src.read_u32());
        match f_id {
            FunctionId::ADD_VIRTUAL_CHANNEL => AddVirtualChannel::decode(src, header).map(Self::AddChan),
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid function id in DEVICE_SINK"
            )),
        }
    }
    fn decode_notification(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* function id */);
        let f_id = FunctionId(src.read_u32());
        match f_id {
            FunctionId::CHANNEL_CREATED => ChannelCreated::decode(src, header).map(Self::ChanCreated),
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid function id in CHANNEL_CREATED"
            )),
        }
    }
}

impl Decode<'_> for UrbdrcClientControlPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;

        match unpack(header.iface_id)? {
            (InterfaceId::CAPABILITIES, Mask::None) => {
                RimExchangeCapabilityResponse::decode(src, header).map(Self::Caps)
            }
            (InterfaceId::DEVICE_SINK, Mask::Proxy) => Self::decode_sink(src, header),
            (InterfaceId::NOTIFY_SERVER, Mask::Proxy) => Self::decode_notification(src, header),
            _ => Err(invalid_field_err!("SHARED_MSG_HEADER", "invalid header")),
        }
    }
}

impl UrbdrcClientDevicePdu {
    fn decode_sink(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* function id */);
        let f_id = FunctionId(src.read_u32());
        match f_id {
            FunctionId::ADD_DEVICE => AddDevice::decode(src, header).map(Self::AddDev),
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid function id in DEVICE_SINK"
            )),
        }
    }
    fn decode_notification(src: &mut ReadCursor<'_>, header: SharedMsgHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* function id */);
        let f_id = FunctionId(src.read_u32());
        match f_id {
            FunctionId::CHANNEL_CREATED => ChannelCreated::decode(src, header).map(Self::ChanCreated),
            FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
            FunctionId::RIMCALL_QUERYINTERFACE => QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq),
            _ => Err(invalid_field_err!(
                "SHARED_MSG_HEADER",
                "invalid function id in CHANNEL_CREATED"
            )),
        }
    }
}

impl Decode<'_> for UrbdrcClientDevicePdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;

        match unpack(header.iface_id)? {
            (InterfaceId::DEVICE_SINK, Mask::Proxy) => Self::decode_sink(src, header),
            (InterfaceId::NOTIFY_SERVER, Mask::Proxy) => Self::decode_notification(src, header),
            (udev_iface, Mask::Stub) => {
                QueryDeviceTextRsp::decode(src, header.msg_id, udev_iface).map(Self::DevTextRsp)
            }
            (udev_iface, Mask::Proxy) => {
                ensure_size!(in: src, size: 4 /* function id */);
                match FunctionId(src.read_u32()) {
                    FunctionId::RIMCALL_RELEASE => Ok(Self::IfaceRelease(InterfaceRelease::from_header(header))),
                    FunctionId::RIMCALL_QUERYINTERFACE => {
                        QueryInterfaceRequest::decode(src, header).map(Self::QueryIfaceReq)
                    }
                    FunctionId::IOCONTROL_COMPLETION => {
                        IoControlCompletion::decode(src, header.msg_id, udev_iface).map(Self::IoctlComp)
                    }
                    FunctionId::URB_COMPLETION => {
                        UrbCompletion::decode(src, header.msg_id, udev_iface).map(Self::UrbComp)
                    }
                    FunctionId::URB_COMPLETION_NO_DATA => {
                        UrbCompletionNoData::decode(src, header.msg_id, udev_iface).map(Self::UrbCompNoData)
                    }
                    _ => Err(invalid_field_err!(
                        "SHARED_MSG_HEADER::InterfaceId",
                        "unknown interface id"
                    )),
                }
            }
            _ => Err(invalid_field_err!("SHARED_MSG_HEADER", "invalid header")),
        }
    }
}

macro_rules! fill_client_ctl_pdu_arms {
    ($pdu:expr, $($tokens:tt)*) => {{
        use UrbdrcClientControlPdu::*;
        match <&UrbdrcClientControlPdu>::from($pdu) {
            Caps(rim_exchange_capability_response) => rim_exchange_capability_response$($tokens)*,
            AddChan(add_virtual_channel) => add_virtual_channel$($tokens)*,
            ChanCreated(channel_created) => channel_created$($tokens)*,
            IfaceRelease(iface_release) => iface_release$($tokens)*,
            QueryIfaceReq(query_iface_req) => query_iface_req$($tokens)*,
        }
    }};
}

macro_rules! fill_client_dev_pdu_arms {
    ($pdu:expr, $($tokens:tt)*) => {{
        use UrbdrcClientDevicePdu::*;
        match <&UrbdrcClientDevicePdu>::from($pdu) {
            ChanCreated(channel_created) => channel_created$($tokens)*,
            IfaceRelease(iface_release) => iface_release$($tokens)*,
            QueryIfaceReq(query_iface_req) => query_iface_req$($tokens)*,
            AddDev(add_dev) => add_dev$($tokens)*,
            DevTextRsp(query_device_text_rsp) => query_device_text_rsp$($tokens)*,
            IoctlComp(iocontrol_completion) => iocontrol_completion$($tokens)*,
            UrbComp(urb_completion) => urb_completion$($tokens)*,
            UrbCompNoData(urb_completion_no_data) => urb_completion_no_data$($tokens)*,
        }
    }};
}

impl Encode for UrbdrcClientControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        fill_client_ctl_pdu_arms!(self, .encode(dst))
    }

    fn name(&self) -> &'static str {
        fill_client_ctl_pdu_arms!(self, .name())
    }

    fn size(&self) -> usize {
        fill_client_ctl_pdu_arms!(self, .size())
    }
}

impl Encode for UrbdrcClientDevicePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        fill_client_dev_pdu_arms!(self, .encode(dst))
    }

    fn name(&self) -> &'static str {
        fill_client_dev_pdu_arms!(self, .name())
    }

    fn size(&self) -> usize {
        fill_client_dev_pdu_arms!(self, .size())
    }
}
