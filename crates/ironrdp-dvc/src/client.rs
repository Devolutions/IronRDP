use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;
use core::any::TypeId;
use core::fmt;

use crate::alloc::borrow::ToOwned as _;
use crate::complete_data::CompleteData;
use ironrdp_core::{Decode as _, DecodeResult, ReadCursor, impl_as_any};
use ironrdp_pdu::{self as pdu, decode_err, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::PduResult;
use pdu::gcc::ChannelName;
use tracing::debug;

use crate::pdu::{
    CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcClientPdu,
    DrdynvcDataPdu, DrdynvcServerPdu,
};
use crate::{
    DvcMessage, DvcProcessor, DynamicChannelId, DynamicChannelMut, DynamicChannelName, DynamicChannelRef,
    encode_dvc_messages,
};

pub trait DvcClientProcessor: DvcProcessor {}

pub trait DvcChannelListener: Send {
    fn channel_name(&self) -> &str;

    /// Called for each incoming DYNVC_CREATE_REQ matching this name.
    /// Return `None` to reject (NO_LISTENER).
    fn create(&mut self, channel_id: DynamicChannelId) -> Option<Box<dyn DvcProcessor>>;
}

pub type DynamicChannelListener = Box<dyn DvcChannelListener>;

/// For pre-registered DVC
struct OnceListener {
    inner: Option<Box<dyn DvcProcessor>>,
}

impl OnceListener {
    fn new(dvc_processor: impl DvcProcessor + 'static) -> Self {
        Self {
            inner: Some(Box::new(dvc_processor)),
        }
    }
}

impl DvcChannelListener for OnceListener {
    fn channel_name(&self) -> &str {
        self.inner
            .as_ref()
            .expect("channel name called after created")
            .channel_name()
    }

    fn create(&mut self, _channel_id: DynamicChannelId) -> Option<Box<dyn DvcProcessor>> {
        self.inner.take()
    }
}

struct DynamicVirtualChannel {
    channel_processor: Box<dyn DvcProcessor + Send>,
    complete_data: CompleteData,
    /// The channel ID assigned by the server.
    ///
    /// `Some` only after [`DynamicVirtualChannel::start`] has succeeded. This invariant
    channel_id: Option<DynamicChannelId>,
}

impl Drop for DynamicVirtualChannel {
    fn drop(&mut self) {
        if let Some(id) = self.channel_id {
            self.channel_processor.close(id);
        }
    }
}

impl DynamicVirtualChannel {
    fn from_boxed(processor: Box<dyn DvcProcessor + Send>) -> Self {
        Self {
            channel_processor: processor,
            complete_data: CompleteData::new(),
            channel_id: None,
        }
    }

    fn processor_type_id(&self) -> TypeId {
        self.channel_processor.as_any().type_id()
    }

    fn start(&mut self, channel_id: DynamicChannelId) -> PduResult<Vec<DvcMessage>> {
        let messages = self.channel_processor.start(channel_id)?;
        self.channel_id = Some(channel_id);
        Ok(messages)
    }

    fn process(&mut self, pdu: DrdynvcDataPdu) -> PduResult<Vec<DvcMessage>> {
        let channel_id = pdu.channel_id();
        let complete_data = self.complete_data.process_data(pdu).map_err(|e| decode_err!(e))?;
        if let Some(complete_data) = complete_data {
            self.channel_processor.process(channel_id, &complete_data)
        } else {
            Ok(Vec::new())
        }
    }

    fn channel_name(&self) -> &str {
        self.channel_processor.channel_name()
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

    /// Registers a pre-initialized dynamic virtual channel with the [`DrdynvcClient`],
    /// making it available for immediate use when the session starts.
    ///
    /// # Note
    ///
    /// If a listener or a pre-registered channel with the same name already exists,
    /// it will be silently overwritten.
    #[must_use]
    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DvcProcessor + 'static,
    {
        self.dynamic_channels.register_once(channel);
        self
    }

    /// Attaches a pre-initialized dynamic virtual channel with the [`DrdynvcClient`],
    /// making it available for immediate use when the session starts.
    ///
    /// # Note
    ///
    /// If a listener or a pre-registered channel with the same name already exists,
    /// it will be silently overwritten.
    pub fn attach_dynamic_channel<T>(&mut self, channel: T)
    where
        T: DvcProcessor + 'static,
    {
        self.dynamic_channels.register_once(channel);
    }

    /// Bind a listener.
    ///
    /// # Note
    ///
    /// * Doesn't support [TypeId] lookup via [DrdynvcClient::get_dvc].
    /// * If a listener or a pre-registered channel with the same name already exists,
    ///   it will be silently overwritten.
    #[must_use]
    pub fn with_listener<T>(mut self, listener: T) -> Self
    where
        T: DvcChannelListener + 'static,
    {
        self.dynamic_channels.register_listener(listener);
        self
    }

    /// Attaches a listener.
    ///
    /// # Note
    ///
    /// * Doesn't support [TypeId] lookup via [DrdynvcClient::get_dvc].
    /// * If a listener or a pre-registered channel with the same name already exists,
    ///   it will be silently overwritten.
    pub fn attach_listener<T>(&mut self, listener: T)
    where
        T: DvcChannelListener + 'static,
    {
        self.dynamic_channels.register_listener(listener);
    }

    /// Returns a typed accessor for a pre-registered client DVC.
    ///
    /// Type lookup is available only for channels registered with
    /// [`DrdynvcClient::with_dynamic_channel`] or [`DrdynvcClient::attach_dynamic_channel`].
    /// Listener-created channels can be retrieved with [`DrdynvcClient::get_dvc_by_channel_id`].
    ///
    /// Returns `None` until the server has created the channel and the processor has started.
    pub fn get_dvc<T>(&self) -> Option<DynamicChannelRef<'_, T>>
    where
        T: DvcClientProcessor,
    {
        let dvc_channel = self.dynamic_channels.get_by_type_id(TypeId::of::<T>())?;
        let channel_id = dvc_channel.channel_id?;
        dvc_channel
            .channel_processor
            .as_any()
            .downcast_ref()
            .map(|p| DynamicChannelRef::new(channel_id, p))
    }

    /// Returns a typed accessor for an active client DVC by channel ID.
    ///
    /// Returns `None` when the channel ID is unknown or the processor has a different type.
    pub fn get_dvc_by_channel_id<T>(&self, channel_id: u32) -> Option<DynamicChannelRef<'_, T>>
    where
        T: DvcClientProcessor,
    {
        self.dynamic_channels
            .get_by_channel_id(channel_id)
            .and_then(|dvc| dvc.channel_processor.as_any().downcast_ref())
            .map(|p| DynamicChannelRef::new(channel_id, p))
    }

    /// Returns a mutable typed accessor for an active client DVC by channel ID.
    ///
    /// Returns `None` when the channel ID is unknown or the processor has a different type.
    pub fn get_dvc_by_channel_id_mut<T>(&mut self, channel_id: u32) -> Option<DynamicChannelMut<'_, T>>
    where
        T: DvcClientProcessor,
    {
        self.dynamic_channels
            .get_by_channel_id_mut(channel_id)
            .and_then(|dvc| dvc.channel_processor.as_any_mut().downcast_mut())
            .map(|p| DynamicChannelMut::new(channel_id, p))
    }

    fn create_capabilities_response(&mut self, server_version: CapsVersion) -> SvcMessage {
        let caps_response = DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(server_version));
        debug!("Send DVC Capabilities Response PDU: {caps_response:?}");
        self.cap_handshake_done = true;
        SvcMessage::from(caps_response)
    }

    pub fn close_channel(&mut self, channel_id: u32) -> Option<SvcMessage> {
        self.dynamic_channels.remove_by_channel_id(channel_id)?;
        Some(SvcMessage::from(DrdynvcClientPdu::Close(ClosePdu::new(channel_id))))
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

                let (creation_status, start_messages) =
                    if let Some(dvc) = self.dynamic_channels.try_create_channel(&channel_name, channel_id) {
                        match dvc.start(channel_id) {
                            Ok(messages) => (CreationStatus::OK, messages),
                            Err(e) => {
                                debug!(
                                    ?channel_id, error = %e,
                                    "DVC start failed; removing channel and reporting NO_LISTENER"
                                );
                                self.dynamic_channels.remove_by_channel_id(channel_id);
                                (CreationStatus::NO_LISTENER, Vec::new())
                            }
                        }
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
            DrdynvcServerPdu::Close(close) => {
                debug!("Got DVC Close PDU: {close:?}");
                let channel_id = close.channel_id();
                if self.dynamic_channels.remove_by_channel_id(channel_id).is_some() {
                    let close_response = DrdynvcClientPdu::Close(ClosePdu::new(channel_id));
                    debug!("Send DVC Close Response PDU: {close_response:?}");
                    responses.push(SvcMessage::from(close_response));
                }
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

struct ListenerEntry {
    listener: DynamicChannelListener,
    /// `Some` only for channels registered via `with_dynamic_channel<T>()`.
    type_id: Option<TypeId>,
}

struct DynamicChannelSet {
    listeners: BTreeMap<DynamicChannelName, ListenerEntry>,
    active_channels: BTreeMap<DynamicChannelId, DynamicVirtualChannel>,
    type_id_to_channel_id: BTreeMap<TypeId, DynamicChannelId>,
}

impl DynamicChannelSet {
    #[inline]
    fn new() -> Self {
        Self {
            listeners: BTreeMap::new(),
            active_channels: BTreeMap::new(),
            type_id_to_channel_id: BTreeMap::new(),
        }
    }

    fn register_listener<T: DvcChannelListener + 'static>(&mut self, listener: T) {
        let name = listener.channel_name().to_owned();
        self.listeners.insert(
            name,
            ListenerEntry {
                listener: Box::new(listener),
                type_id: None,
            },
        );
    }

    fn register_once<T: DvcProcessor + 'static>(&mut self, channel: T) {
        let name = channel.channel_name().to_owned();
        self.listeners.insert(
            name,
            ListenerEntry {
                listener: Box::new(OnceListener::new(channel)),
                type_id: Some(TypeId::of::<T>()),
            },
        );
    }

    fn try_create_channel(
        &mut self,
        name: &DynamicChannelName,
        channel_id: DynamicChannelId,
    ) -> Option<&mut DynamicVirtualChannel> {
        let entry = self.listeners.get_mut(name)?;
        let processor = entry.listener.create(channel_id)?;

        if let Some(type_id) = entry.type_id {
            self.type_id_to_channel_id.insert(type_id, channel_id);
        }

        let dvc = DynamicVirtualChannel::from_boxed(processor);
        // `dvc.channel_id` stays `None` here — it is set by `DynamicVirtualChannel::start`
        // on success, so `Drop` only invokes `close` for channels that were actually opened.
        let dvc = match self.active_channels.entry(channel_id) {
            alloc::collections::btree_map::Entry::Occupied(mut e) => {
                e.insert(dvc);
                e.into_mut()
            }
            alloc::collections::btree_map::Entry::Vacant(e) => e.insert(dvc),
        };
        Some(dvc)
    }

    fn get_by_type_id(&self, type_id: TypeId) -> Option<&DynamicVirtualChannel> {
        self.type_id_to_channel_id
            .get(&type_id)
            .and_then(|id| self.active_channels.get(id))
    }

    fn get_by_channel_id(&self, id: DynamicChannelId) -> Option<&DynamicVirtualChannel> {
        self.active_channels.get(&id)
    }

    fn get_by_channel_id_mut(&mut self, id: DynamicChannelId) -> Option<&mut DynamicVirtualChannel> {
        self.active_channels.get_mut(&id)
    }

    fn remove_by_channel_id(&mut self, id: DynamicChannelId) -> Option<DynamicVirtualChannel> {
        self.active_channels.remove(&id).inspect(|dvc| {
            let type_id = dvc.processor_type_id();

            // Only matters for pre-registered channels
            if let alloc::collections::btree_map::Entry::Occupied(entry) = self.type_id_to_channel_id.entry(type_id)
                && entry.get() == &id
            {
                entry.remove();
            }
        })
    }

    #[inline]
    fn values(&self) -> impl Iterator<Item = &DynamicVirtualChannel> {
        self.active_channels.values()
    }
}
impl SvcClientProcessor for DrdynvcClient {}

fn decode_dvc_message(user_data: &[u8]) -> DecodeResult<DrdynvcServerPdu> {
    DrdynvcServerPdu::decode(&mut ReadCursor::new(user_data))
}
