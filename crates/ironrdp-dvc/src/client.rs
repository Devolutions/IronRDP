use alloc::boxed::Box;
use alloc::collections::BTreeSet;
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
    DrdynvcServerPdu, SoftSyncResponsePdu, SoftSyncTunnelType,
};
use crate::{DvcProcessor, DynamicChannelId, DynamicChannelName, DynamicVirtualChannel, encode_dvc_messages};

pub trait DvcClientProcessor: DvcProcessor {}

pub trait DvcChannelListener: Send {
    fn channel_name(&self) -> &str;

    /// Called for each incoming DYNVC_CREATE_REQ matching this name.
    /// Return `None` to reject (NO_LISTENER).
    fn create(&mut self, channel_id: DynamicChannelId) -> Option<Box<dyn DvcProcessor>>;

    /// Returns whether this listener can still create a channel.
    fn is_available(&self) -> bool {
        true
    }
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

    fn is_available(&self) -> bool {
        self.inner.is_some()
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
    pending_tunnel_channels: Option<BTreeMap<DynamicChannelId, SoftSyncTunnelType>>,
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
            pending_tunnel_channels: None,
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

    pub fn get_dvc_by_type_id<T>(&self) -> Option<&DynamicVirtualChannel>
    where
        T: DvcProcessor,
    {
        self.dynamic_channels.get_by_type_id(TypeId::of::<T>())
    }

    /// Returns whether a dynamic channel of type `T` was pre-registered with this client.
    pub fn has_registered_dvc<T>(&self) -> bool
    where
        T: DvcProcessor,
    {
        self.dynamic_channels.has_listener_by_type_id(TypeId::of::<T>())
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
        self.tunnel_channels.remove(&channel_id);
        if let Some(channels) = self.pending_tunnel_channels.as_mut() {
            channels.remove(&channel_id);
        }
        Some(SvcMessage::from(DrdynvcClientPdu::Close(ClosePdu::new(channel_id))))
    }

    /// Enables a multitransport tunnel for a future Soft-Sync request.
    pub fn enable_soft_sync_tunnel(&mut self, tunnel_type: SoftSyncTunnelType) {
        self.available_tunnels.insert(tunnel_type);
    }

    /// Returns whether a Soft-Sync response has been encoded but not yet sent on TCP.
    pub fn has_pending_soft_sync_response(&self) -> bool {
        self.pending_tunnel_channels.is_some()
    }

    /// Activates the routing selected by the most recent Soft-Sync response.
    ///
    /// Call this only after the response has been successfully written over the
    /// DRDYNVC static virtual channel.
    pub fn complete_soft_sync_response(&mut self) -> PduResult<()> {
        let pending = self
            .pending_tunnel_channels
            .take()
            .ok_or_else(|| pdu_other_err!("no Soft-Sync response is pending"))?;
        self.tunnel_channels = pending;
        self.soft_sync_complete = true;
        Ok(())
    }

    /// Returns whether the Soft-Sync response has been sent over TCP and
    /// multitransport DVC data can be exchanged.
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

    fn process_data(&mut self, data: crate::pdu::DrdynvcDataPdu) -> PduResult<Vec<SvcMessage>> {
        let channel_id = data.channel_id();
        let messages = self
            .dynamic_channels
            .get_by_channel_id_mut(channel_id)
            .ok_or_else(|| pdu_other_err!("access to non existing DVC channel"))?
            .process(data)?;

        encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|e| encode_err!(e))
    }

    fn process_soft_sync_request(&mut self, request: crate::pdu::SoftSyncRequestPdu) -> PduResult<SvcMessage> {
        if self.pending_tunnel_channels.is_some() || self.soft_sync_complete {
            return Err(pdu_other_err!("received duplicate Soft-Sync request"));
        }

        let mut tunnel_channels = BTreeMap::new();
        let mut tunnels_to_switch = Vec::new();
        for list in request.channel_lists() {
            if !self.available_tunnels.contains(&list.tunnel_type()) {
                continue;
            }
            for channel_id in list.channel_ids() {
                if self.dynamic_channels.get_by_channel_id(*channel_id).is_none() {
                    return Err(pdu_other_err!("Soft-Sync request contains an unknown dynamic channel"));
                }
                tunnel_channels.insert(*channel_id, list.tunnel_type());
            }
            tunnels_to_switch.push(list.tunnel_type());
        }

        self.pending_tunnel_channels = Some(tunnel_channels);
        Ok(SvcMessage::from(DrdynvcClientPdu::SoftSyncResponse(
            SoftSyncResponsePdu::new(tunnels_to_switch),
        )))
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

    fn has_listener_by_type_id(&self, type_id: TypeId) -> bool {
        self.listeners
            .values()
            .any(|entry| entry.type_id == Some(type_id) && entry.listener.is_available())
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

        fn start(&mut self, _channel_id: u32) -> PduResult<Vec<crate::DvcMessage>> {
            Ok(Vec::new())
        }

        fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<crate::DvcMessage>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn consumed_typed_listener_is_not_registered() {
        let mut channels = DynamicChannelSet::new();
        channels.register_once(TestDvc);

        assert!(channels.has_listener_by_type_id(TypeId::of::<TestDvc>()));
        assert!(channels.try_create_channel(&"test".to_owned(), 1).is_some());
        assert!(!channels.has_listener_by_type_id(TypeId::of::<TestDvc>()));
    }
}
