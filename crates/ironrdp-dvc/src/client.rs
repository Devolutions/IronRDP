use alloc::boxed::Box;
use alloc::collections::BTreeSet;
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
    DrdynvcDataPdu, DrdynvcServerPdu, SoftSyncResponsePdu, SoftSyncTunnelType,
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
    fn create(&mut self, channel_id: DynamicChannelId) -> Option<Box<dyn DvcClientProcessor>>;

    /// Returns whether this listener can still create a channel.
    fn is_available(&self) -> bool {
        true
    }
}

pub type DynamicChannelListener = Box<dyn DvcChannelListener>;

/// For pre-registered DVC
struct OnceListener {
    inner: Option<Box<dyn DvcClientProcessor>>,
}

impl OnceListener {
    fn new(dvc_processor: impl DvcClientProcessor + 'static) -> Self {
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

    fn create(&mut self, _channel_id: DynamicChannelId) -> Option<Box<dyn DvcClientProcessor>> {
        self.inner.take()
    }

    fn is_available(&self) -> bool {
        self.inner.is_some()
    }
}

struct DynamicVirtualChannel {
    channel_processor: Box<dyn DvcClientProcessor + Send>,
    complete_data: CompleteData,
    /// The channel ID assigned by the server.
    ///
    /// `Some` only after [`DynamicVirtualChannel::start`] has succeeded.
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
    fn from_boxed(processor: Box<dyn DvcClientProcessor + Send>) -> Self {
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
    available_tunnels: BTreeSet<SoftSyncTunnelType>,
    tunnel_channels: BTreeMap<DynamicChannelId, SoftSyncTunnelType>,
    soft_sync_complete: bool,
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
            available_tunnels: BTreeSet::new(),
            tunnel_channels: BTreeMap::new(),
            soft_sync_complete: false,
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
        T: DvcClientProcessor + 'static,
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
        T: DvcClientProcessor + 'static,
    {
        self.dynamic_channels.register_once(channel);
    }

    pub fn attach_established_dynamic_channel<T>(&mut self, channel_id: DynamicChannelId, channel: T) -> PduResult<()>
    where
        T: DvcClientProcessor + 'static,
    {
        self.dynamic_channels
            .attach_established_channel(channel_id, Box::new(channel))
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

    /// Returns whether a dynamic channel of type `T` was pre-registered with this client.
    pub fn has_registered_dvc<T>(&self) -> bool
    where
        T: DvcClientProcessor,
    {
        self.dynamic_channels.has_listener_by_type_id(TypeId::of::<T>())
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

    /// Returns a mutable typed accessor for a pre-registered client DVC.
    ///
    /// The mutable counterpart of [`DrdynvcClient::get_dvc`]. Type lookup is available
    /// only for channels registered with [`DrdynvcClient::with_dynamic_channel`] or
    /// [`DrdynvcClient::attach_dynamic_channel`]. Returns `None` until the server has
    /// created the channel and the processor has started.
    pub fn get_dvc_mut<T>(&mut self) -> Option<DynamicChannelMut<'_, T>>
    where
        T: DvcClientProcessor,
    {
        let dvc_channel = self.dynamic_channels.get_by_type_id_mut(TypeId::of::<T>())?;
        let channel_id = dvc_channel.channel_id?;
        dvc_channel
            .channel_processor
            .as_any_mut()
            .downcast_mut()
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
        self.tunnel_channels.remove(&channel_id);
        Some(SvcMessage::from(DrdynvcClientPdu::Close(ClosePdu::new(channel_id))))
    }

    /// Marks a multitransport tunnel as ready for a future Soft-Sync request.
    ///
    /// Call this only after the tunnel's Initiate Multitransport Response has
    /// been sent successfully.
    pub fn enable_soft_sync_tunnel(&mut self, tunnel_type: SoftSyncTunnelType) {
        self.available_tunnels.insert(tunnel_type);
    }

    /// Returns whether the client has produced its Soft-Sync response and activated
    /// its local routing state.
    pub const fn soft_sync_complete(&self) -> bool {
        self.soft_sync_complete
    }

    /// Returns the tunnel selected for client-to-server messages on `channel_id`.
    pub fn tunnel_for_channel(&self, channel_id: DynamicChannelId) -> Option<SoftSyncTunnelType> {
        self.tunnel_channels.get(&channel_id).copied()
    }

    /// Processes raw DRDYNVC data received through an established multitransport tunnel.
    pub fn process_tunnel(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode_dvc_message(payload).map_err(|e| decode_err!(e))?;
        let DrdynvcServerPdu::Data(data) = pdu else {
            return Err(pdu_other_err!("only DVC data is permitted on a multitransport tunnel"));
        };
        let channel_id = data.channel_id();
        if !self.tunnel_channels.contains_key(&channel_id) {
            return Err(pdu_other_err!(
                "received tunneled data for a channel not selected by Soft-Sync"
            ));
        }
        self.process_data(data)
    }

    fn process_data(&mut self, data: DrdynvcDataPdu) -> PduResult<Vec<SvcMessage>> {
        let channel_id = data.channel_id();
        let messages = self
            .dynamic_channels
            .get_by_channel_id_mut(channel_id)
            .ok_or_else(|| pdu_other_err!("access to non existing DVC channel"))?
            .process(data)?;

        encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|e| encode_err!(e))
    }

    fn process_soft_sync_request(&mut self, request: crate::pdu::SoftSyncRequestPdu) -> PduResult<SvcMessage> {
        if self.soft_sync_complete {
            return Err(pdu_other_err!("received duplicate Soft-Sync request"));
        }

        let mut tunnel_channels = BTreeMap::new();
        let mut tunnels_to_switch = Vec::new();
        for list in request.channel_lists() {
            if !self.available_tunnels.contains(&list.tunnel_type()) {
                return Err(pdu_other_err!("soft-sync request selected an unavailable tunnel"));
            }

            let mut selected_channels = Vec::new();
            for channel_id in list.channel_ids() {
                if self.dynamic_channels.get_by_channel_id(*channel_id).is_none() {
                    selected_channels.clear();
                    break;
                }
                selected_channels.push(*channel_id);
            }
            if selected_channels.is_empty() && !list.channel_ids().is_empty() {
                continue;
            }
            for channel_id in selected_channels {
                tunnel_channels.insert(channel_id, list.tunnel_type());
            }
            tunnels_to_switch.push(list.tunnel_type());
        }

        let response = SvcMessage::from(DrdynvcClientPdu::SoftSyncResponse(SoftSyncResponsePdu::new(
            tunnels_to_switch,
        )));
        self.tunnel_channels = tunnel_channels;
        self.soft_sync_complete = true;
        Ok(response)
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
                if self.tunnel_channels.contains_key(&data.channel_id()) {
                    return Err(pdu_other_err!("received TCP data for a channel selected by Soft-Sync"));
                }
                responses.extend(self.process_data(data)?);
            }
            DrdynvcServerPdu::SoftSyncRequest(request) => {
                debug!("Got DVC Soft-Sync Request PDU: {request:?}");
                responses.push(self.process_soft_sync_request(request)?);
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

    fn register_once<T: DvcClientProcessor + 'static>(&mut self, channel: T) {
        let name = channel.channel_name().to_owned();
        self.listeners.insert(
            name,
            ListenerEntry {
                listener: Box::new(OnceListener::new(channel)),
                type_id: Some(TypeId::of::<T>()),
            },
        );
    }

    fn attach_established_channel(
        &mut self,
        channel_id: DynamicChannelId,
        channel: Box<dyn DvcClientProcessor>,
    ) -> PduResult<()> {
        if self.active_channels.contains_key(&channel_id) {
            return Err(pdu_other_err!("dynamic channel ID is already attached"));
        }

        let mut channel = DynamicVirtualChannel::from_boxed(channel);
        let _messages = channel.start(channel_id)?;
        self.active_channels.insert(channel_id, channel);
        Ok(())
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

    fn has_listener_by_type_id(&self, type_id: TypeId) -> bool {
        self.listeners
            .values()
            .any(|entry| entry.type_id == Some(type_id) && entry.listener.is_available())
    }

    fn get_by_type_id_mut(&mut self, type_id: TypeId) -> Option<&mut DynamicVirtualChannel> {
        let channel_id = *self.type_id_to_channel_id.get(&type_id)?;
        self.active_channels.get_mut(&channel_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDvc;

    impl_as_any!(TestDvc);

    impl DvcProcessor for TestDvc {
        fn channel_name(&self) -> &str {
            "test"
        }

        fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
            Ok(Vec::new())
        }

        fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
            Ok(Vec::new())
        }
    }

    impl DvcClientProcessor for TestDvc {}

    #[test]
    fn consumed_typed_listener_is_not_registered() {
        let mut channels = DynamicChannelSet::new();
        channels.register_once(TestDvc);

        assert!(channels.has_listener_by_type_id(TypeId::of::<TestDvc>()));
        assert!(channels.try_create_channel(&"test".to_owned(), 1).is_some());
        assert!(!channels.has_listener_by_type_id(TypeId::of::<TestDvc>()));
    }

    fn add_active_channel(client: &mut DrdynvcClient, channel_id: DynamicChannelId) {
        client
            .dynamic_channels
            .active_channels
            .insert(channel_id, DynamicVirtualChannel::from_boxed(Box::new(TestDvc)));
    }

    #[test]
    fn soft_sync_rejects_tunnel_data_until_a_response_is_generated() {
        let mut client = DrdynvcClient::new();
        add_active_channel(&mut client, 1);
        client.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);

        let tunnel_data = ironrdp_core::encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(
            crate::pdu::DataPdu::new(1, Vec::new()),
        )))
        .unwrap();
        assert!(client.process_tunnel(&tunnel_data).is_err());

        let request = crate::pdu::SoftSyncRequestPdu::new(alloc::vec![crate::pdu::SoftSyncChannelList::new(
            SoftSyncTunnelType::RELIABLE_UDP,
            alloc::vec![1],
        )]);
        client.process_soft_sync_request(request).unwrap();

        assert!(client.soft_sync_complete());
        assert_eq!(client.tunnel_for_channel(1), Some(SoftSyncTunnelType::RELIABLE_UDP));
        assert!(client.process_tunnel(&tunnel_data).is_ok());

        assert!(client.process(&tunnel_data).is_err());

        client.close_channel(1).unwrap();
        assert_eq!(client.tunnel_for_channel(1), None);
    }

    #[test]
    fn soft_sync_rejects_an_unavailable_tunnel() {
        let mut client = DrdynvcClient::new();
        add_active_channel(&mut client, 1);

        let request = crate::pdu::SoftSyncRequestPdu::new(alloc::vec![crate::pdu::SoftSyncChannelList::new(
            SoftSyncTunnelType::RELIABLE_UDP,
            alloc::vec![1],
        )]);

        assert!(client.process_soft_sync_request(request).is_err());
        assert!(!client.soft_sync_complete());
    }
}
