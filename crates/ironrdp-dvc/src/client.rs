use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;
use core::any::TypeId;
use core::fmt;

use crate::alloc::borrow::ToOwned as _;
use ironrdp_core::{Decode as _, DecodeResult, ReadCursor, impl_as_any};
use ironrdp_pdu::{self as pdu, decode_err, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::PduResult;
use pdu::gcc::ChannelName;
use tracing::debug;

use crate::pdu::{
    CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcClientPdu,
    DrdynvcServerPdu,
};
use crate::{DvcProcessor, DynamicChannelId, DynamicChannelName, DynamicVirtualChannel, encode_dvc_messages};

pub trait DvcClientProcessor: DvcProcessor {}

pub trait DvcChannelListener: Send {
    /// Called for each incoming DYNVC_CREATE_REQ matching this name.
    /// Return `None` to reject (NO_LISTENER).
    fn create(&mut self) -> Option<Box<dyn DvcClientProcessor + Send>>;
}

pub type DynamicChannelListener = Box<dyn DvcChannelListener>;

/// For pre-registered Dvc
pub struct OnceListener {
    inner: Option<Box<dyn DvcClientProcessor + Send>>,
}

impl OnceListener {
    pub fn new(dvc_processor: impl DvcClientProcessor) -> Self {
        Self {
            inner: Some(Box::new(dvc_processor)),
        }
    }
}

impl DvcChannelListener for OnceListener {
    fn create(&mut self) -> Option<Box<dyn DvcClientProcessor + Send>> {
        self.inner.take()
    }
}

/// DRDYNVC Static Virtual Channel (the Remote Desktop Protocol: Dynamic Virtual Channel Extension)
///
/// It adds support for dynamic virtual channels (DVC).
pub struct DrdynvcClient {
    dynamic_channels: DynamicChannelSet,
    /// Indicates whether the capability request/response handshake has been completed.
    cap_handshake_done: bool,
}

impl fmt::Debug for DrdynvcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DrdynvcClient([")?;

        for (i, channel) in self.dynamic_channels.values().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", channel.channel_name())?;
        }

        write!(f, "])")
    }
}

impl DrdynvcClient {
    pub const NAME: ChannelName = ChannelName::from_static(b"drdynvc\0");

    pub fn new() -> Self {
        Self {
            dynamic_channels: DynamicChannelSet::new(),
            cap_handshake_done: false,
        }
    }

    // FIXME(#61): it’s likely we want to enable adding dynamic channels at any point during the session (message passing? other approach?)

    #[must_use]
    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DvcProcessor + 'static,
    {
        self.dynamic_channels.insert(channel);
        self
    }

    pub fn attach_dynamic_channel<T>(&mut self, channel: T)
    where
        T: DvcProcessor + 'static,
    {
        self.dynamic_channels.insert(channel);
    }

    pub fn get_dvc_by_type_id<T>(&self) -> Option<&DynamicVirtualChannel>
    where
        T: DvcProcessor,
    {
        self.dynamic_channels.get_by_type_id(TypeId::of::<T>())
    }

    pub fn get_dvc_by_channel_id(&self, channel_id: u32) -> Option<&DynamicVirtualChannel> {
        self.dynamic_channels.get_by_channel_id(channel_id)
    }

    fn create_capabilities_response(&mut self, server_version: CapsVersion) -> SvcMessage {
        let caps_response = DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(server_version));
        debug!("Send DVC Capabilities Response PDU: {caps_response:?}");
        self.cap_handshake_done = true;
        SvcMessage::from(caps_response)
    }
}

impl_as_any!(DrdynvcClient);

impl Default for DrdynvcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SvcProcessor for DrdynvcClient {
    fn channel_name(&self) -> ChannelName {
        DrdynvcClient::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode_dvc_message(payload).map_err(|e| decode_err!(e))?;
        let mut responses = Vec::new();

        match pdu {
            DrdynvcServerPdu::Capabilities(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {caps_request:?}");
                responses.push(self.create_capabilities_response(caps_request.version()));
            }
            DrdynvcServerPdu::Create(create_request) => {
                debug!("Got DVC Create Request PDU: {create_request:?}");
                let channel_id = create_request.channel_id();
                let channel_name = create_request.into_channel_name();

                if !self.cap_handshake_done {
                    debug!(
                        "Got DVC Create Request PDU before a Capabilities Request PDU. \
                        Sending Capabilities Response PDU before the Create Response PDU."
                    );
                    responses.push(self.create_capabilities_response(CapsVersion::V2));
                }

                let channel_exists = self.dynamic_channels.get_by_channel_name(&channel_name).is_some();
                let (creation_status, start_messages) = if channel_exists {
                    // If we have a handler for this channel, attach the channel ID
                    // and get any start messages.
                    self.dynamic_channels
                        .attach_channel_id(channel_name.clone(), channel_id);
                    let dynamic_channel = self
                        .dynamic_channels
                        .get_by_channel_name_mut(&channel_name)
                        .expect("channel exists");
                    (CreationStatus::OK, dynamic_channel.start()?)
                } else {
                    (CreationStatus::NO_LISTENER, Vec::new())
                };

                let create_response = DrdynvcClientPdu::Create(CreateResponsePdu::new(channel_id, creation_status));
                debug!("Send DVC Create Response PDU: {create_response:?}");
                responses.push(SvcMessage::from(create_response));

                // If this DVC has start messages, send them.
                if !start_messages.is_empty() {
                    responses.extend(
                        encode_dvc_messages(channel_id, start_messages, ChannelFlags::empty())
                            .map_err(|e| encode_err!(e))?,
                    );
                }
            }
            DrdynvcServerPdu::Close(close_request) => {
                debug!("Got DVC Close Request PDU: {close_request:?}");
                self.dynamic_channels.remove_by_channel_id(close_request.channel_id());

                let close_response = DrdynvcClientPdu::Close(ClosePdu::new(close_request.channel_id()));

                debug!("Send DVC Close Response PDU: {close_response:?}");
                responses.push(SvcMessage::from(close_response));
            }
            DrdynvcServerPdu::Data(data) => {
                let channel_id = data.channel_id();

                let messages = self
                    .dynamic_channels
                    .get_by_channel_id_mut(channel_id)
                    .ok_or_else(|| pdu_other_err!("access to non existing DVC channel"))?
                    .process(data)?;

                responses.extend(
                    encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|e| encode_err!(e))?,
                );
            }
        }

        Ok(responses)
    }
}

struct DynamicChannelSet {
    channels: BTreeMap<DynamicChannelName, DynamicVirtualChannel>,
    name_to_channel_id: BTreeMap<DynamicChannelName, DynamicChannelId>,
    channel_id_to_name: BTreeMap<DynamicChannelId, DynamicChannelName>,
    type_id_to_name: BTreeMap<TypeId, DynamicChannelName>,
}

impl DynamicChannelSet {
    #[inline]
    fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
            name_to_channel_id: BTreeMap::new(),
            channel_id_to_name: BTreeMap::new(),
            type_id_to_name: BTreeMap::new(),
        }
    }

    fn insert<T: DvcProcessor + 'static>(&mut self, channel: T) -> Option<DynamicVirtualChannel> {
        let name = channel.channel_name().to_owned();
        self.type_id_to_name.insert(TypeId::of::<T>(), name.clone());
        self.channels.insert(name, DynamicVirtualChannel::new(channel))
    }

    fn attach_channel_id(&mut self, name: DynamicChannelName, id: DynamicChannelId) -> Option<DynamicChannelId> {
        self.channel_id_to_name.insert(id, name.clone());
        self.name_to_channel_id.insert(name.clone(), id);
        let dvc = self.get_by_channel_name_mut(&name)?;
        let old_id = dvc.channel_id;
        dvc.channel_id = Some(id);
        old_id
    }

    fn get_by_type_id(&self, type_id: TypeId) -> Option<&DynamicVirtualChannel> {
        self.type_id_to_name
            .get(&type_id)
            .and_then(|name| self.channels.get(name))
    }

    fn get_by_channel_name(&self, name: &DynamicChannelName) -> Option<&DynamicVirtualChannel> {
        self.channels.get(name)
    }

    fn get_by_channel_name_mut(&mut self, name: &DynamicChannelName) -> Option<&mut DynamicVirtualChannel> {
        self.channels.get_mut(name)
    }

    fn get_by_channel_id(&self, id: DynamicChannelId) -> Option<&DynamicVirtualChannel> {
        self.channel_id_to_name
            .get(&id)
            .and_then(|name| self.channels.get(name))
    }

    fn get_by_channel_id_mut(&mut self, id: DynamicChannelId) -> Option<&mut DynamicVirtualChannel> {
        self.channel_id_to_name
            .get(&id)
            .and_then(|name| self.channels.get_mut(name))
    }

    fn remove_by_channel_id(&mut self, id: DynamicChannelId) -> Option<DynamicChannelId> {
        if let Some(name) = self.channel_id_to_name.remove(&id) {
            return self.name_to_channel_id.remove(&name);
            // Channels are retained in the `self.channels` and `self.type_id_to_name` map to allow potential
            // dynamic re-addition by the server.
        }
        None
    }

    #[inline]
    fn values(&self) -> impl Iterator<Item = &DynamicVirtualChannel> {
        self.channels.values()
    }
}
impl SvcClientProcessor for DrdynvcClient {}

fn decode_dvc_message(user_data: &[u8]) -> DecodeResult<DrdynvcServerPdu> {
    DrdynvcServerPdu::decode(&mut ReadCursor::new(user_data))
}
