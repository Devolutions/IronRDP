use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;
use core::any::TypeId;
use core::fmt;

use ironrdp_core::{Decode as _, DecodeResult, NonEmpty, ReadCursor, impl_as_any};
use ironrdp_pdu::{self as pdu, decode_err, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::PduResult;
use pdu::gcc::ChannelName;
use tracing::debug;

use crate::alloc::borrow::ToOwned as _;
use crate::cardinality::{CardinalityKind, ChannelIds, DvcChannelCardinality, Multi, Singleton, kind_of};
use crate::pdu::{
    CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcClientPdu,
    DrdynvcServerPdu,
};
use crate::{DvcProcessor, DynamicChannelId, DynamicChannelName, DynamicVirtualChannel, encode_dvc_messages};

pub trait DvcClientProcessor: DvcProcessor {}

/// How a channel produced by a [`DvcChannelListener`] should be tracked for type-based lookup.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Produced {
    /// The produced processor is not discoverable by type (e.g. a bespoke listener).
    Untracked,
    /// The produced processor is a singleton (at most one channel for its type).
    Singleton(TypeId),
    /// The produced processor is multi-instance (several channels may share its type).
    Multi(TypeId),
}

impl Produced {
    /// The tracking that matches processor type `T`'s [`cardinality`](DvcChannelCardinality).
    fn for_type<T>() -> Self
    where
        T: DvcChannelCardinality + 'static,
    {
        match kind_of::<T::Cardinality>() {
            CardinalityKind::Singleton => Self::Singleton(TypeId::of::<T>()),
            CardinalityKind::Multi => Self::Multi(TypeId::of::<T>()),
        }
    }
}

/// A dynamic virtual channel processor produced by a [`DvcChannelListener`], together with the
/// information needed to make it discoverable by type.
///
/// Build one with [`CreatedChannel::new`] (which records the processor's cardinality so it can
/// be looked up via [`DrdynvcClient::get_dvc_by_type_id`] /
/// [`DrdynvcClient::get_dvcs_by_type_id`]) or [`CreatedChannel::untracked`] for a processor
/// that should not be discoverable by type.
pub struct CreatedChannel {
    processor: Box<dyn DvcProcessor>,
    tracking: Produced,
}

impl CreatedChannel {
    /// Wraps `processor`, recording its type and cardinality for type-based lookup.
    #[must_use]
    pub fn new<T>(processor: T) -> Self
    where
        T: DvcChannelCardinality + 'static,
    {
        Self {
            processor: Box::new(processor),
            tracking: Produced::for_type::<T>(),
        }
    }

    /// Wraps an already-boxed `processor` that should not be discoverable by type.
    #[must_use]
    pub fn untracked(processor: Box<dyn DvcProcessor>) -> Self {
        Self {
            processor,
            tracking: Produced::Untracked,
        }
    }

    /// Returns a shared reference to the wrapped processor.
    #[must_use]
    pub fn processor(&self) -> &dyn DvcProcessor {
        self.processor.as_ref()
    }

    /// Returns a mutable reference to the wrapped processor.
    #[must_use]
    pub fn processor_mut(&mut self) -> &mut dyn DvcProcessor {
        self.processor.as_mut()
    }
}

pub trait DvcChannelListener: Send {
    fn channel_name(&self) -> &str;

    /// Called for each incoming DYNVC_CREATE_REQ matching this name.
    /// Return `None` to reject (NO_LISTENER).
    fn create(&mut self, channel_id: DynamicChannelId) -> Option<CreatedChannel>;
}

pub type DynamicChannelListener = Box<dyn DvcChannelListener>;

/// For pre-registered DVC
struct OnceListener {
    inner: Option<CreatedChannel>,
}

impl OnceListener {
    fn new<T>(dvc_processor: T) -> Self
    where
        T: DvcChannelCardinality + 'static,
    {
        Self {
            inner: Some(CreatedChannel::new(dvc_processor)),
        }
    }
}

impl DvcChannelListener for OnceListener {
    fn channel_name(&self) -> &str {
        self.inner
            .as_ref()
            .expect("channel name called after created")
            .processor
            .channel_name()
    }

    fn create(&mut self, _channel_id: DynamicChannelId) -> Option<CreatedChannel> {
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
        T: DvcProcessor + DvcChannelCardinality + 'static,
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
        T: DvcProcessor + DvcChannelCardinality + 'static,
    {
        self.dynamic_channels.register_once(channel);
    }

    /// Bind a listener.
    ///
    /// # Note
    ///
    /// * Doesn't support [TypeId] lookup via [DrdynvcClient::get_dvc_by_type_id].
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
    /// * Doesn't support [TypeId] lookup via [DrdynvcClient::get_dvc_by_type_id].
    /// * If a listener or a pre-registered channel with the same name already exists,
    ///   it will be silently overwritten.
    pub fn attach_listener<T>(&mut self, listener: T)
    where
        T: DvcChannelListener + 'static,
    {
        self.dynamic_channels.register_listener(listener);
    }

    /// Returns the dynamic virtual channel backed by the singleton processor type `T`, or
    /// `None` if no such channel is currently active.
    ///
    /// This accessor is only available for [`Singleton`](crate::Singleton) channel types;
    /// calling it on a [`Multi`](crate::Multi) type is a compile error, use
    /// [`get_dvcs_by_type_id`](Self::get_dvcs_by_type_id) instead.
    pub fn get_dvc_by_type_id<T>(&self) -> Option<&DynamicVirtualChannel>
    where
        T: DvcChannelCardinality<Cardinality = Singleton> + 'static,
    {
        self.dynamic_channels.get_by_type_id(TypeId::of::<T>())
    }

    /// Returns every active dynamic virtual channel backed by the multi-instance processor
    /// type `T`, in creation order, or `None` if no such channel is currently active.
    ///
    /// This accessor is only available for [`Multi`](crate::Multi) channel types; calling it
    /// on a [`Singleton`](crate::Singleton) type is a compile error, use
    /// [`get_dvc_by_type_id`](Self::get_dvc_by_type_id) instead.
    pub fn get_dvcs_by_type_id<T>(&self) -> Option<NonEmpty<&DynamicVirtualChannel>>
    where
        T: DvcChannelCardinality<Cardinality = Multi> + 'static,
    {
        self.dynamic_channels.get_all_by_type_id(TypeId::of::<T>())
    }

    pub fn get_dvc_by_channel_id(&self, channel_id: u32) -> Option<&DynamicVirtualChannel> {
        self.dynamic_channels.get_by_channel_id(channel_id)
    }

    pub fn get_dvc_by_channel_id_mut(&mut self, channel_id: u32) -> Option<&mut DynamicVirtualChannel> {
        self.dynamic_channels.get_by_channel_id_mut(channel_id)
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
}

struct DynamicChannelSet {
    listeners: BTreeMap<DynamicChannelName, ListenerEntry>,
    active_channels: BTreeMap<DynamicChannelId, DynamicVirtualChannel>,
    type_id_to_channel_id: BTreeMap<TypeId, ChannelIds>,
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
            },
        );
    }

    fn register_once<T: DvcProcessor + DvcChannelCardinality + 'static>(&mut self, channel: T) {
        let name = channel.channel_name().to_owned();
        self.listeners.insert(
            name,
            ListenerEntry {
                listener: Box::new(OnceListener::new(channel)),
            },
        );
    }

    fn try_create_channel(
        &mut self,
        name: &DynamicChannelName,
        channel_id: DynamicChannelId,
    ) -> Option<&mut DynamicVirtualChannel> {
        let entry = self.listeners.get_mut(name)?;
        let created = entry.listener.create(channel_id)?;

        match created.tracking {
            Produced::Untracked => {}
            Produced::Singleton(type_id) => {
                ChannelIds::insert_into(
                    &mut self.type_id_to_channel_id,
                    CardinalityKind::Singleton,
                    type_id,
                    channel_id,
                );
            }
            Produced::Multi(type_id) => {
                ChannelIds::insert_into(
                    &mut self.type_id_to_channel_id,
                    CardinalityKind::Multi,
                    type_id,
                    channel_id,
                );
            }
        }

        let dvc = DynamicVirtualChannel::from_boxed(created.processor);
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
        match self.type_id_to_channel_id.get(&type_id) {
            Some(ChannelIds::Singleton(id)) => self.active_channels.get(id),
            // A given type is always stored with the variant matching its cardinality, so a
            // singleton type never maps to a `Multi` entry.
            Some(ChannelIds::Multi(_)) => unreachable!("singleton type stored as Multi"),
            None => None,
        }
    }

    fn get_all_by_type_id(&self, type_id: TypeId) -> Option<NonEmpty<&DynamicVirtualChannel>> {
        let ids = match self.type_id_to_channel_id.get(&type_id) {
            Some(ChannelIds::Multi(ids)) => ids,
            // A given type is always stored with the variant matching its cardinality, so a
            // multi type never maps to a `Singleton` entry.
            Some(ChannelIds::Singleton(_)) => unreachable!("multi type stored as Singleton"),
            None => return None,
        };

        // The type mapping and `active_channels` are kept in sync, so every registered id
        // resolves to an active channel and the result is non-empty.
        let mut channels = ids.iter().filter_map(|id| self.active_channels.get(id));
        let mut result = NonEmpty::new(channels.next()?);
        for channel in channels {
            result.push(channel);
        }
        Some(result)
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

            // Drop only the id being removed from the type mapping, keeping any sibling
            // channels of the same type discoverable. The map entry is removed only when no
            // channel of that type remains.
            if let alloc::collections::btree_map::Entry::Occupied(entry) = self.type_id_to_channel_id.entry(type_id) {
                let channel_ids = entry.remove();
                if let Some(remaining) = channel_ids.without(id) {
                    self.type_id_to_channel_id.insert(type_id, remaining);
                }
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
