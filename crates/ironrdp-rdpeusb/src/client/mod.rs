use alloc::collections::btree_map::{BTreeMap, Entry};
use alloc::string::String;
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};
use ironrdp_core::{Decode as _, EncodeResult, ReadCursor, impl_as_any, other_err};
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};

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
use crate::{CHANNEL_NAME, InvalidDeviceInterfaceId};

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
    pub fn add_virtual_channel(&self, dev_id: u32) -> EncodeResult<DvcMessage> {
        if !self.ready {
            return Err(other_err!("is not ready for ADD_VIRTUAL_CHANNEL"));
        }
        // Follow FreeRDP use device id as message id
        Ok(Box::new(AddVirtualChannel { msg_id: dev_id }))
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

