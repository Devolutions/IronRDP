use core::time::Duration;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError, Weak};

use anyhow::{Context as _, anyhow};
use ironrdp_core::{Encode, EncodeResult, WriteCursor, impl_as_any};
use ironrdp_dvc::{DvcEncode, DvcMessage, DvcMessageBatch, DvcProcessor, DvcServerProcessor, encode_dvc_messages};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_server::{Acceptor, ServerEvent, StaticChannelFactory};
use ironrdp_svc::{CHANNEL_CHUNK_LENGTH, ChannelFlags, SvcMessage, SvcProcessor, SvcServerProcessor};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, trace, warn};
use windows::Win32::Foundation::ERROR_BROKEN_PIPE;

use crate::native_pipe::NativePipe;
use crate::windows_main::generate_capture_helper_token;

#[path = "native_camera.rs"]
mod camera;
#[path = "native_rail.rs"]
mod rail;

pub(crate) use rail::remote_app_enabled;

const NOW_CHANNEL: &str = "Devolutions::Now::Agent";
const WEBAUTHN_CHANNEL: &str = "WebAuthN_Channel";
const CAMERA_ENUMERATOR: &str = "RDCamera_Device_Enumerator";
const DYNAMIC_CHANNELS: [DynamicEndpoint; 3] = [
    // The NOW host keeps one channel for its lifetime and reopens it after loss.
    DynamicEndpoint {
        name: NOW_CHANNEL,
        instances: 1,
    },
    // webauthn.dll opens a channel for each API call and closes it after the response.
    DynamicEndpoint {
        name: WEBAUTHN_CHANNEL,
        instances: 4,
    },
    DynamicEndpoint {
        name: CAMERA_ENUMERATOR,
        instances: 1,
    },
];
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
const MAX_CAMERA_MESSAGE_SIZE: usize =
    1 /* Version */ + 1 /* MessageId */ + 1 /* StreamIndex */ + ironrdp_rdpecam::pdu::MAX_SAMPLE_SIZE;
// A static channel carries whole messages of its protocol, such as the read completions for the 1 MiB reads of the drive redirector.
// The agent also accepts print writes of up to 16 MiB, and the extra 64 KiB covers message headers and paths.
const MAX_STATIC_MESSAGE_SIZE: usize = 16 * 1024 * 1024 + 64 * 1024;
// The agent accepts requests up to 16 MiB, and a whole request can arrive faster than the pump drains it.
const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
// The pipe proxy delivers about 4 KiB per message, so the byte budget binds first unless messages average under 1 KiB.
const MAX_PENDING_MESSAGES: usize = MAX_PENDING_BYTES / 1024;
const OPEN_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
// TermService opens these by their upper-case names, while clients advertise them in lower case.
const STATIC_CHANNELS: [ChannelName; 3] = [
    ChannelName::from_static(b"rdpdr\0\0\0"),
    ChannelName::from_static(b"rdpsnd\0\0"),
    ChannelName::from_static(b"rail\0\0\0\0"),
];

/// A dynamic channel that the native bridge serves.
struct DynamicEndpoint {
    name: &'static str,
    /// How many channels of this endpoint one connection may have open or opening at once.
    instances: usize,
}

pub(crate) struct NativeChannels {
    remote_app: rail::RemoteAppState,
    create_pipe: fn(&str) -> anyhow::Result<NativePipe>,
    sender: StdMutex<Option<mpsc::UnboundedSender<ServerEvent>>>,
    session: watch::Sender<Option<u32>>,
    // One semaphore per entry of `DYNAMIC_CHANNELS`, in the same order, with a permit per instance.
    dynamic: [Arc<Semaphore>; DYNAMIC_CHANNELS.len()],
    camera: StdMutex<Weak<camera::Enumeration>>,
    camera_instances: Arc<Semaphore>,
    // One slot per entry of `STATIC_CHANNELS`, in the same order.
    statics: [Arc<StaticSlot>; STATIC_CHANNELS.len()],
}

impl Default for NativeChannels {
    fn default() -> Self {
        Self {
            remote_app: rail::RemoteAppState::default(),
            create_pipe: NativePipe::create,
            sender: StdMutex::default(),
            session: watch::Sender::default(),
            dynamic: DYNAMIC_CHANNELS.map(|endpoint| Arc::new(Semaphore::new(endpoint.instances))),
            camera: StdMutex::default(),
            camera_instances: Arc::new(Semaphore::new(4)),
            statics: Default::default(),
        }
    }
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "RemoteApp methods live beside their connection state and native window-order framing"
)]
impl NativeChannels {
    pub(crate) fn attach(
        self: &Arc<Self>,
        sender: mpsc::UnboundedSender<ServerEvent>,
    ) -> anyhow::Result<ConnectionGuard> {
        *self
            .sender
            .lock()
            .map_err(|_| anyhow!("native channel sender lock poisoned"))? = Some(sender);
        Ok(ConnectionGuard(Arc::clone(self)))
    }

    pub(crate) fn set_session(&self, session_id: u32) {
        self.session.send_if_modified(|current| {
            if *current == Some(session_id) {
                false
            } else {
                self.close_camera();
                *current = Some(session_id);
                true
            }
        });
    }

    fn camera(&self) -> MutexGuard<'_, Weak<camera::Enumeration>> {
        self.camera.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close_camera(&self) {
        if let Some(camera) = self.camera().upgrade() {
            camera.close();
        }
    }

    fn bind(
        &self,
        session_id: u32,
    ) -> anyhow::Result<(watch::Receiver<Option<u32>>, mpsc::UnboundedSender<ServerEvent>)> {
        let mut binding = self.session.subscribe();
        anyhow::ensure!(
            session_id != 0 && session_id != u32::MAX && *binding.borrow_and_update() == Some(session_id),
            "native channel session does not match the connection"
        );
        let sender = self
            .sender
            .lock()
            .map_err(|_| anyhow!("native channel sender lock poisoned"))?
            .clone()
            .context("native channel transport is unavailable")?;
        Ok((binding, sender))
    }

    pub(crate) async fn open(&self, session_id: u32, endpoint_name: &str) -> anyhow::Result<String> {
        if endpoint_name == "railenc" {
            return self.open_window_orders(session_id).await;
        }
        let (mut binding, sender) = self.bind(session_id)?;
        let fixed = DYNAMIC_CHANNELS
            .iter()
            .zip(&self.dynamic)
            .find(|(endpoint, _)| endpoint.name == endpoint_name);
        let mut removed = None;
        let instances = if let Some((_, instances)) = fixed {
            instances
        } else {
            let camera = self
                .camera()
                .upgrade()
                .context("native channel endpoint is not enabled")?;
            anyhow::ensure!(
                camera.session_id == session_id,
                "camera session does not match the connection"
            );
            removed = Some(camera.device(endpoint_name)?);
            &self.camera_instances
        };
        // The channel task holds the permit until the channel closes, so an abandoned open releases it too.
        let instance = Arc::clone(instances)
            .try_acquire_owned()
            .context("native channel endpoint has no free instance")?;
        let camera = (endpoint_name == CAMERA_ENUMERATOR).then(|| Arc::new(camera::Enumeration::new(session_id)));
        let camera_lease = camera.as_ref().map(|camera| {
            *self.camera() = Arc::downgrade(camera);
            camera::Lease(Arc::clone(camera))
        });

        let (opened, opened_rx) = oneshot::channel();
        let (closed, closed_rx) = oneshot::channel();
        let maximum_message_size = if fixed.is_some() {
            MAX_MESSAGE_SIZE
        } else {
            MAX_CAMERA_MESSAGE_SIZE
        };
        let (mut processor, incoming_rx) =
            NativeDvc::new(endpoint_name, maximum_message_size, Some(opened), Some(closed));
        processor.camera = camera.clone();
        let (created, created_rx) = oneshot::channel();
        let channel_sender = sender.clone();
        sender
            .send(ServerEvent::CreateDynamicChannel(Box::new(move |id| {
                let _ = created.send(ChannelGuard {
                    id,
                    sender: channel_sender,
                });
                Box::new(processor)
            })))
            .map_err(|_| anyhow!("native channel event queue closed"))?;
        let pending = async {
            let channel = created_rx.await.context("native channel creation was rejected")?;
            opened_rx.await.context("native channel peer declined creation")?;
            let pipe_name = format!(r"\\.\pipe\IronRdpNativeDvc-{}", generate_capture_helper_token()?);
            let pipe = (self.create_pipe)(&pipe_name)?;
            anyhow::Ok((channel, pipe_name, pipe))
        };
        let (channel, pipe_name, pipe) = tokio::select! {
            biased;
            _ = binding.changed() => anyhow::bail!("native channel session changed during creation"),
            _ = camera_removed(&mut removed) => anyhow::bail!("native camera was removed during creation"),
            result = timeout(OPEN_TIMEOUT, pending) => result.context("native channel creation timed out")??,
        };
        let channel_name = endpoint_name.to_owned();
        drop(tokio::spawn(async move {
            let channel_id = channel.id;
            let endpoint = match camera {
                Some(camera) => Endpoint::CameraEnumeration(channel, camera),
                None => Endpoint::Dynamic(channel),
            };
            let result = tokio::select! {
                biased;
                _ = binding.changed() => Err(anyhow!("native channel session binding changed")),
                _ = closed_rx => Ok(()),
                _ = camera_removed(&mut removed) => Ok(()),
                result = pump(&endpoint, &pipe, incoming_rx) => result,
            };
            drop(camera_lease);
            // Freeing the instance before the endpoint sends its close lets a reopen follow the close at once.
            drop(instance);
            if let Err(error) = result {
                warn!(session_id, channel_id, channel = channel_name, %error, "Native dynamic channel stopped");
            } else {
                debug!(
                    session_id,
                    channel_id,
                    channel = channel_name,
                    "Native dynamic channel closed"
                );
            }
        }));
        Ok(pipe_name)
    }

    pub(crate) async fn open_static(&self, session_id: u32, endpoint_name: &str) -> anyhow::Result<String> {
        if endpoint_name.eq_ignore_ascii_case("rail") {
            anyhow::ensure!(
                self.remote_app_settings()?.is_some_and(|settings| settings.enabled),
                "RemoteApp was not requested or is disabled"
            );
        }
        let (index, name) = STATIC_CHANNELS
            .iter()
            .enumerate()
            .find(|(_, name)| {
                name.as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(endpoint_name))
            })
            .context("native static channel endpoint is not enabled")?;
        let (mut binding, sender) = self.bind(session_id)?;
        let (reply, joined) = oneshot::channel();
        sender
            .send(ServerEvent::StaticChannelJoined(name.clone(), reply))
            .map_err(|_| anyhow!("native channel event queue closed"))?;
        let joined = tokio::select! {
            biased;
            _ = binding.changed() => anyhow::bail!("native channel session changed during creation"),
            result = timeout(OPEN_TIMEOUT, joined) => result
                .context("native static channel join query timed out")?
                .context("native static channel join query was dropped")?,
        };
        anyhow::ensure!(joined, "native static channel was not offered by the client");
        let pipe_name = format!(r"\\.\pipe\IronRdpNativeDvc-{}", generate_capture_helper_token()?);
        let pipe = NativePipe::create(&pipe_name)?;
        drop(start_static(
            session_id,
            name,
            &self.statics[index],
            sender,
            binding,
            pipe,
        ));
        Ok(pipe_name)
    }

    fn processors(&self) -> impl Iterator<Item = NativeStatic> + '_ {
        STATIC_CHANNELS
            .iter()
            .zip(&self.statics)
            .filter(|(name, _)| name.as_str() != Some("rail") || self.remote_app_capabilities().is_some())
            .map(|(name, slot)| NativeStatic {
                name: name.clone(),
                slot: Arc::clone(slot),
            })
    }

    pub(crate) fn static_channels(self: &Arc<Self>) -> Box<dyn StaticChannelFactory> {
        Box::new(NativeStaticChannels(Arc::clone(self)))
    }
}

async fn camera_removed(removed: &mut Option<watch::Receiver<()>>) {
    match removed {
        Some(removed) => {
            let _ = removed.changed().await;
        }
        None => core::future::pending().await,
    }
}

struct NativeStaticChannels(Arc<NativeChannels>);

impl StaticChannelFactory for NativeStaticChannels {
    fn attach(&self, acceptor: &mut Acceptor) {
        for processor in self.0.processors() {
            let name = processor.name.clone();
            if !acceptor.attach_dynamic_static_channel(processor) {
                warn!(
                    channel = name.as_str().unwrap_or_default(),
                    "Native static channel was not attached"
                );
            }
        }
    }
}

pub(crate) struct ConnectionGuard(Arc<NativeChannels>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.close_camera();
        self.0.session.send_replace(None);
        match self.0.sender.lock() {
            Ok(mut sender) => *sender = None,
            Err(error) => warn!(%error, "Native channel sender lock poisoned during disconnect"),
        }
    }
}

struct ChannelGuard {
    id: u32,
    sender: mpsc::UnboundedSender<ServerEvent>,
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        if self.sender.send(ServerEvent::CloseDynamicChannel(self.id)).is_err() {
            debug!(channel_id = self.id, "Native channel transport already disconnected");
        }
    }
}

struct NativeDvc {
    name: String,
    camera: Option<Arc<camera::Enumeration>>,
    inbox: Inbox,
    opened: Option<oneshot::Sender<()>>,
    closed: Option<oneshot::Sender<()>>,
    failed: bool,
}

struct IncomingMessage {
    payload: Vec<u8>,
    _budget: OwnedSemaphorePermit,
}

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    MessageTooLarge,
    PendingBytes,
    PendingMessages,
    ConsumerGone,
}

/// Bounded queue of client messages waiting for the native consumer to read them.
#[derive(Debug)]
struct Inbox {
    incoming: mpsc::Sender<IncomingMessage>,
    budget: Arc<Semaphore>,
    maximum_message_size: usize,
}

impl Inbox {
    fn new(maximum_message_size: usize) -> (Self, mpsc::Receiver<IncomingMessage>) {
        let (incoming, queue) = mpsc::channel(MAX_PENDING_MESSAGES);
        let inbox = Self {
            incoming,
            budget: Arc::new(Semaphore::new(MAX_PENDING_BYTES.max(maximum_message_size))),
            maximum_message_size,
        };
        (inbox, queue)
    }

    fn admit(&self, payload: &[u8]) -> Result<(), Refusal> {
        let size = u32::try_from(payload.len().max(1))
            .ok()
            .filter(|_| payload.len() <= self.maximum_message_size)
            .ok_or(Refusal::MessageTooLarge)?;
        let budget = Arc::clone(&self.budget)
            .try_acquire_many_owned(size)
            .map_err(|_| Refusal::PendingBytes)?;
        self.incoming
            .try_send(IncomingMessage {
                payload: payload.to_vec(),
                _budget: budget,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Refusal::PendingMessages,
                mpsc::error::TrySendError::Closed(_) => Refusal::ConsumerGone,
            })
    }

    fn queued_messages(&self) -> usize {
        self.incoming.max_capacity() - self.incoming.capacity()
    }

    fn queued_bytes(&self) -> usize {
        MAX_PENDING_BYTES.max(self.maximum_message_size) - self.budget.available_permits()
    }
}

impl NativeDvc {
    fn new(
        name: &str,
        maximum_message_size: usize,
        opened: Option<oneshot::Sender<()>>,
        closed: Option<oneshot::Sender<()>>,
    ) -> (Self, mpsc::Receiver<IncomingMessage>) {
        let (inbox, queue) = Inbox::new(maximum_message_size);
        let processor = Self {
            name: name.to_owned(),
            camera: None,
            inbox,
            opened,
            closed,
            failed: false,
        };
        (processor, queue)
    }
}

impl_as_any!(NativeDvc);

impl DvcProcessor for NativeDvc {
    fn channel_name(&self) -> &str {
        &self.name
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(self.inbox.maximum_message_size)
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if let Some(opened) = self.opened.take() {
            let _ = opened.send(());
        }
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if self.failed {
            return Ok(Vec::new());
        }
        if let Some(camera) = &self.camera {
            if let Err(error) = camera.observe_client(payload) {
                warn!(channel_id, %error, "Closing the invalid camera enumeration channel");
                camera.close();
                self.failed = true;
                self.closed.take();
                return Ok(Vec::new());
            }
        }
        if let Err(reason) = self.inbox.admit(payload) {
            warn!(
                channel_id,
                ?reason,
                queued_messages = self.inbox.queued_messages(),
                queued_bytes = self.inbox.queued_bytes(),
                "Closing the native channel after refusing an incoming message"
            );
            self.failed = true;
            self.closed.take();
            if let Some(camera) = &self.camera {
                camera.close();
            }
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for NativeDvc {}

/// The consumer registered for one static channel, if any.
#[derive(Debug)]
struct StaticSlot {
    state: StdMutex<SlotState>,
    outstanding_write: Arc<Semaphore>,
}

impl Default for StaticSlot {
    fn default() -> Self {
        Self {
            state: StdMutex::default(),
            outstanding_write: Arc::new(Semaphore::new(1)),
        }
    }
}

#[derive(Debug, Default)]
struct SlotState {
    generation: u64,
    consumer: Option<StaticConsumer>,
}

#[derive(Debug)]
struct StaticConsumer {
    generation: u64,
    inbox: Inbox,
    // Dropping the consumer drops this sender, which stops its pump.
    _closed: oneshot::Sender<()>,
}

/// Clears its registration when dropped, unless a newer consumer has replaced it.
struct SlotGuard {
    slot: Arc<StaticSlot>,
    generation: u64,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let mut state = self.slot.state();
        if state
            .consumer
            .as_ref()
            .is_some_and(|consumer| consumer.generation == self.generation)
        {
            state.consumer = None;
        }
    }
}

impl StaticSlot {
    fn state(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes a new consumer the receiver of client data, stopping the one it replaces.
    fn register(self: &Arc<Self>) -> (SlotGuard, mpsc::Receiver<IncomingMessage>, oneshot::Receiver<()>) {
        let (inbox, queue) = Inbox::new(MAX_STATIC_MESSAGE_SIZE);
        let (closed, closed_rx) = oneshot::channel();
        let mut state = self.state();
        state.generation += 1;
        let generation = state.generation;
        state.consumer = Some(StaticConsumer {
            generation,
            inbox,
            _closed: closed,
        });
        drop(state);
        let guard = SlotGuard {
            slot: Arc::clone(self),
            generation,
        };
        (guard, queue, closed_rx)
    }

    fn deliver(&self, name: &ChannelName, payload: &[u8]) {
        self.deliver_for(self.context(), name, payload);
    }

    fn context(&self) -> Option<u64> {
        let mut state = self.state();
        if state
            .consumer
            .as_ref()
            .is_some_and(|consumer| consumer.inbox.incoming.is_closed())
        {
            state.consumer = None;
        }
        state.consumer.as_ref().map(|consumer| consumer.generation)
    }

    fn deliver_for(&self, context: Option<u64>, name: &ChannelName, payload: &[u8]) {
        let channel = name.as_str().unwrap_or_default();
        let mut state = self.state();
        let Some(consumer) = state.consumer.as_ref() else {
            debug!(
                channel,
                length = payload.len(),
                "Dropping client data for a native static channel without a consumer"
            );
            return;
        };
        if context != Some(consumer.generation) {
            debug!(channel, "Discarding a native static message for a previous consumer");
            return;
        }
        match consumer.inbox.admit(payload) {
            Ok(()) => trace!(
                channel,
                length = payload.len(),
                "Queued client data for the native static channel"
            ),
            Err(Refusal::ConsumerGone) => {
                debug!(channel, "Native static channel consumer is gone");
                state.consumer = None;
            }
            Err(reason) => {
                warn!(
                    channel,
                    ?reason,
                    queued_messages = consumer.inbox.queued_messages(),
                    queued_bytes = consumer.inbox.queued_bytes(),
                    "Closing the native static channel after refusing an incoming message"
                );
                state.consumer = None;
            }
        }
    }
}

#[derive(Debug)]
struct NativeStatic {
    name: ChannelName,
    slot: Arc<StaticSlot>,
}

impl_as_any!(NativeStatic);

impl SvcProcessor for NativeStatic {
    fn channel_name(&self) -> ChannelName {
        self.name.clone()
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_STATIC_MESSAGE_SIZE)
    }

    fn message_context(&self) -> Option<u64> {
        self.slot.context()
    }

    fn process_with_context(&mut self, context: u64, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        self.slot.deliver_for(Some(context), &self.name, payload);
        Ok(Vec::new())
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        self.slot.deliver(&self.name, payload);
        Ok(Vec::new())
    }
}

impl SvcServerProcessor for NativeStatic {}

fn start_static(
    session_id: u32,
    name: &ChannelName,
    slot: &Arc<StaticSlot>,
    sender: mpsc::UnboundedSender<ServerEvent>,
    mut binding: watch::Receiver<Option<u32>>,
    pipe: NativePipe,
) -> JoinHandle<()> {
    let (guard, incoming, closed) = slot.register();
    let name = name.clone();
    let slot = Arc::clone(slot);
    tokio::spawn(async move {
        let endpoint = Endpoint::Static {
            name: name.clone(),
            sender,
            slot,
            generation: guard.generation,
        };
        let _guard = guard;
        let result = tokio::select! {
            biased;
            _ = binding.changed() => Err(anyhow!("native channel session binding changed")),
            _ = closed => Ok(()),
            result = pump(&endpoint, &pipe, incoming) => result,
        };
        let channel = name.as_str().unwrap_or_default();
        if let Err(error) = result {
            warn!(session_id, channel, %error, "Native static channel stopped");
        } else {
            debug!(session_id, channel, "Native static channel closed");
        }
    })
}

/// The side of the RDP connection that a native pipe is bridged to.
enum Endpoint {
    Dynamic(ChannelGuard),
    CameraEnumeration(ChannelGuard, Arc<camera::Enumeration>),
    Static {
        name: ChannelName,
        sender: mpsc::UnboundedSender<ServerEvent>,
        slot: Arc<StaticSlot>,
        generation: u64,
    },
}

impl Endpoint {
    /// The largest message the native consumer may send to the client in one piece.
    fn maximum_message_size(&self) -> usize {
        match self {
            Self::Dynamic(_) | Self::CameraEnumeration(..) => MAX_MESSAGE_SIZE,
            Self::Static { .. } => MAX_STATIC_MESSAGE_SIZE,
        }
    }

    /// Writes one client message to the pipe in the records the native consumer reads.
    async fn write_message(&self, pipe: &NativePipe, payload: &[u8]) -> anyhow::Result<()> {
        match self {
            Self::Dynamic(_) | Self::CameraEnumeration(..) => {
                write_framed(pipe, payload, ChannelControlFlags::empty()).await
            }
            Self::Static { name, .. } if name.as_str() == Some("rail") => {
                write_framed(pipe, payload, ChannelControlFlags::FLAG_SHOW_PROTOCOL).await
            }
            Self::Static { .. } => {
                for chunk in payload.chunks(CHANNEL_CHUNK_LENGTH) {
                    pipe.write(chunk.to_vec()).await?;
                }
                Ok(())
            }
        }
    }

    /// Sends one native message to the client and waits until the transport has written it.
    async fn forward(&self, payload: Vec<u8>) -> anyhow::Result<()> {
        if let Self::CameraEnumeration(_, camera) = self {
            camera.observe_native(&payload)?;
        }
        let (written, completion) = oneshot::channel();
        match self {
            Self::Dynamic(channel) | Self::CameraEnumeration(channel, _) => {
                let messages = encode_dvc_messages(
                    channel.id,
                    vec![Box::new(Payload(payload))],
                    ChannelFlags::SHOW_PROTOCOL,
                )?;
                let batch = DvcMessageBatch::try_new(channel.id, messages)?;
                channel
                    .sender
                    .send(ServerEvent::DynamicChannelWithCompletion(batch, written))
                    .map_err(|_| anyhow!("native dynamic channel transport closed"))?;
            }
            Self::Static {
                name,
                sender,
                slot,
                generation,
            } => {
                if payload.is_empty() {
                    return Ok(());
                }
                let permit = timeout(IO_TIMEOUT, Arc::clone(&slot.outstanding_write).acquire_owned())
                    .await
                    .context("native static channel outstanding write timed out")?
                    .context("native static channel write queue closed")?;
                trace!(
                    channel = name.as_str().unwrap_or_default(),
                    length = payload.len(),
                    "Forwarding native static channel data to the client"
                );
                let slot = Arc::clone(slot);
                let generation = *generation;
                sender
                    .send(ServerEvent::StaticChannelWithCompletion {
                        name: name.clone(),
                        payload,
                        completion: written,
                        permit,
                        is_current: Box::new(move || slot.context() == Some(generation)),
                    })
                    .map_err(|_| anyhow!("native static channel transport closed"))?;
            }
        }
        timeout(IO_TIMEOUT, completion)
            .await
            .context("native channel transport write timed out")?
            .context("native channel transport write failed")?;
        Ok(())
    }
}

/// Writes `payload` as chunks of at most `CHANNEL_CHUNK_LENGTH` bytes, each led by a `CHANNEL_PDU_HEADER`.
async fn write_framed(pipe: &NativePipe, payload: &[u8], extra_flags: ChannelControlFlags) -> anyhow::Result<()> {
    let length = u32::try_from(payload.len()).context("native channel payload length overflow")?;
    let mut offset = 0;
    loop {
        let end = (offset + CHANNEL_CHUNK_LENGTH).min(payload.len());
        let mut flags = extra_flags;
        if offset == 0 {
            flags |= ChannelControlFlags::FLAG_FIRST;
        }
        if end == payload.len() {
            flags |= ChannelControlFlags::FLAG_LAST;
        }
        let mut record = ironrdp_core::encode_vec(&ChannelPduHeader { length, flags })?;
        record.extend_from_slice(&payload[offset..end]);
        pipe.write(record).await?;
        offset = end;
        if end == payload.len() {
            return Ok(());
        }
    }
}

async fn pump(
    endpoint: &Endpoint,
    pipe: &NativePipe,
    mut incoming: mpsc::Receiver<IncomingMessage>,
) -> anyhow::Result<()> {
    timeout(OPEN_TIMEOUT, pipe.connect())
        .await
        .context("native channel pipe connection timed out")??;
    let receive = async {
        while let Some(message) = incoming.recv().await {
            endpoint.write_message(pipe, &message.payload).await?;
        }
        anyhow::Ok(())
    };
    let send = async {
        loop {
            let payload = match pipe
                .read(endpoint.maximum_message_size())
                .await
                .context("native pipe read worker failed")?
            {
                Ok(payload) => payload,
                // The consumer closed its handle, which is how a WTS caller closes the channel.
                Err(error)
                    if error
                        .downcast_ref::<windows::core::Error>()
                        .is_some_and(|error| error.code() == ERROR_BROKEN_PIPE.to_hresult()) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            endpoint.forward(payload).await?;
        }
    };
    tokio::select! {
        result = receive => result,
        result = send => result,
    }
}

struct Payload(Vec<u8>);

impl DvcEncode for Payload {}

impl Encode for Payload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.0.encode(dst)
    }

    fn name(&self) -> &'static str {
        "NativeChannelPayload"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::{Read as _, Write as _};
    use std::os::windows::io::AsRawHandle as _;

    use ironrdp_core::decode;
    use ironrdp_dvc::pdu::{DrdynvcDataPdu, DrdynvcServerPdu};
    use ironrdp_pdu::nego::SecurityProtocol;
    use ironrdp_server::DesktopSize;
    use ironrdp_svc::StaticVirtualChannel;
    use windows::Win32::Foundation::HANDLE;

    use super::*;

    // oracle: regression oversized static FIRST without LAST must be rejected before reassembly, even without a consumer.
    #[test]
    fn static_reassembly_rejects_an_oversized_initial_fragment_without_a_consumer() {
        let native = NativeChannels::default();
        let mut channel = StaticVirtualChannel::new(native.processors().next().unwrap());
        let fragment = static_fragment(1024 * 1024 * 1024, ChannelControlFlags::FLAG_FIRST, b"start");
        assert!(
            channel.process(&fragment).is_err(),
            "an oversized unterminated message entered reassembly"
        );
    }

    // oracle: regression a message begun for a replaced static consumer must never reach its successor.
    #[test]
    fn static_reassembly_discards_a_replaced_consumers_message() {
        let native = NativeChannels::default();
        let slot = &native.statics[0];
        let (_first, mut first_queue, _first_closed) = slot.register();
        let mut channel = StaticVirtualChannel::new(native.processors().next().unwrap());
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_FIRST, b"old"))
            .unwrap();
        let (_second, mut second_queue, _second_closed) = slot.register();
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_LAST, b"end"))
            .unwrap();
        assert!(first_queue.try_recv().is_err());
        assert!(
            matches!(second_queue.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "the successor received the previous consumer's partial message"
        );
        channel
            .process(&static_fragment(
                5,
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
                b"fresh",
            ))
            .unwrap();
        assert_eq!(second_queue.try_recv().unwrap().payload, b"fresh");
    }

    // oracle: regression closing a consumer during reassembly discards its remainder without poisoning a fresh message.
    #[test]
    fn static_reassembly_discards_a_closed_consumers_message() {
        let native = NativeChannels::default();
        let slot = &native.statics[0];
        let (first, _first_queue, _first_closed) = slot.register();
        let mut channel = StaticVirtualChannel::new(native.processors().next().unwrap());
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_FIRST, b"old"))
            .unwrap();
        drop(first);
        channel
            .process(&static_fragment(6, ChannelControlFlags::empty(), b"en"))
            .unwrap();
        let (_second, mut queue, _closed) = slot.register();
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_LAST, b"d"))
            .unwrap();
        assert!(matches!(queue.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        channel
            .process(&static_fragment(
                5,
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
                b"fresh",
            ))
            .unwrap();
        assert_eq!(queue.try_recv().unwrap().payload, b"fresh");
    }

    // oracle: regression a newly attached consumer must not receive a message begun before it registered.
    #[test]
    fn static_reassembly_does_not_replay_a_message_begun_without_a_consumer() {
        let native = NativeChannels::default();
        let mut channel = StaticVirtualChannel::new(native.processors().next().unwrap());
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_FIRST, b"old"))
            .unwrap();
        let (_guard, mut queue, _closed) = native.statics[0].register();
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_LAST, b"end"))
            .unwrap();
        assert!(matches!(queue.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // oracle: regression discarded fragments remain subject to protocol length validation.
    #[test]
    fn static_reassembly_validates_the_remainder_of_a_discarded_message() {
        let native = NativeChannels::default();
        let (_first, _queue, _closed) = native.statics[0].register();
        let mut channel = StaticVirtualChannel::new(native.processors().next().unwrap());
        channel
            .process(&static_fragment(6, ChannelControlFlags::FLAG_FIRST, b"old"))
            .unwrap();
        let _second = native.statics[0].register();
        assert!(
            channel
                .process(&static_fragment(6, ChannelControlFlags::FLAG_LAST, b"too long"))
                .is_err()
        );
    }

    // oracle: regression the native static boundary accepts a complete 16 MiB + 64 KiB message, independent of DVC limits.
    #[test]
    fn static_reassembly_accepts_the_native_limit_in_transport_sized_fragments() {
        const LENGTH: usize = 16 * 1024 * 1024 + 64 * 1024;
        let native = NativeChannels::default();
        let (_guard, mut queue, _closed) = native.statics[0].register();
        let mut channel = StaticVirtualChannel::new(native.processors().next().unwrap());
        let payload: Vec<u8> = (0..LENGTH).map(|index| u8::try_from(index % 251).unwrap()).collect();
        for (index, data) in payload.chunks(1600).enumerate() {
            let mut flags = ChannelControlFlags::empty();
            if index == 0 {
                flags |= ChannelControlFlags::FLAG_FIRST;
            }
            if index * 1600 + data.len() == LENGTH {
                flags |= ChannelControlFlags::FLAG_LAST;
            }
            channel
                .process(&static_fragment(u32::try_from(LENGTH).unwrap(), flags, data))
                .unwrap();
        }
        assert_eq!(queue.try_recv().unwrap().payload, payload);
    }

    fn static_fragment(length: u32, flags: ChannelControlFlags, payload: &[u8]) -> Vec<u8> {
        let mut fragment = ironrdp_core::encode_vec(&ChannelPduHeader { length, flags }).unwrap();
        fragment.extend_from_slice(payload);
        fragment
    }

    // oracle: spec WTSVirtualChannelRead remarks and CHANNEL_PDU_HEADER; independently chosen bytes.
    #[tokio::test]
    async fn native_reads_receive_chunk_headers_and_preserve_empty_messages() {
        let (_task, mut processor, _events, mut pipe) = pipe_fixture();
        processor.process(7, &vec![0xa5; 1601]).unwrap();
        processor.process(7, &[]).unwrap();
        let read = tokio::task::spawn_blocking(move || {
            let mut buffer = [0; 2048];
            let length = pipe.read(&mut buffer).unwrap();
            assert_eq!(length, 1608);
            assert_eq!(&buffer[..8], &[0x41, 0x06, 0, 0, 1, 0, 0, 0]);
            assert!(buffer[8..length].iter().all(|byte| *byte == 0xa5));
            let length = pipe.read(&mut buffer).unwrap();
            assert_eq!(&buffer[..length], &[0x41, 0x06, 0, 0, 2, 0, 0, 0, 0xa5]);
            let length = pipe.read(&mut buffer).unwrap();
            assert_eq!(&buffer[..length], &[0, 0, 0, 0, 3, 0, 0, 0]);
        });
        timeout(Duration::from_secs(5), read).await.unwrap().unwrap();
    }

    // oracle: mutant counting small transport chunks as full-sized messages rejects ordinary streamed stdin.
    #[tokio::test]
    async fn native_reads_accept_a_megabyte_split_into_small_messages() {
        let (_task, mut processor, _events, mut pipe) = pipe_fixture();
        for index in 0..256u16 {
            processor.process(7, &vec![index.to_le_bytes()[0]; 4096]).unwrap();
        }
        let read = tokio::task::spawn_blocking(move || {
            let mut record = [0; 1608];
            for index in 0..256u16 {
                let mut payload = Vec::new();
                while payload.len() < 4096 {
                    let count = pipe.read(&mut record).unwrap();
                    assert_eq!(&record[..4], &[0, 16, 0, 0]);
                    payload.extend_from_slice(&record[8..count]);
                }
                assert_eq!(payload, vec![index.to_le_bytes()[0]; 4096]);
            }
        });
        timeout(Duration::from_secs(5), read).await.unwrap().unwrap();
    }

    // oracle: mutant a receive bound below the 16 MiB the agent accepts in one request closes the channel mid-stream.
    #[tokio::test]
    async fn native_reads_deliver_the_largest_stdin_burst_intact() {
        assert_burst_delivered(1024 * 1024).await;
    }

    // oracle: mutant a message-count bound below a 16 MiB request in 4 KiB pieces closes the channel mid-stream.
    #[tokio::test]
    async fn native_reads_deliver_the_largest_stdin_burst_in_small_messages() {
        assert_burst_delivered(4096).await;
    }

    // oracle: mutant a blocked native reader also stalls the independent native-to-DVC direction.
    #[tokio::test]
    async fn native_output_progresses_while_native_input_is_blocked() {
        let (_task, mut processor, mut events, mut pipe) = pipe_fixture();
        processor.process(7, &vec![0xa5; 128 * 1024]).unwrap();
        let writer = tokio::task::spawn_blocking(move || {
            let mut record = [0; 1608];
            assert_eq!(pipe.read(&mut record).unwrap(), record.len());
            pipe.write_all(b"independent output").unwrap();
            pipe
        });
        let _pipe = timeout(Duration::from_secs(5), writer).await.unwrap().unwrap();
        let event = timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
        let ServerEvent::DynamicChannelWithCompletion(batch, completion) = event else {
            panic!("expected native output while the peer is not reading input");
        };
        let pdu: DrdynvcServerPdu = decode(&batch.messages()[0].encode_unframed_pdu().unwrap()).unwrap();
        let DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) = pdu else {
            panic!("expected an unfragmented output message");
        };
        assert_eq!(data.data(), b"independent output");
        completion.send(()).unwrap();
    }

    // oracle: mutant removing the receive bound allows arbitrarily large queued payloads.
    #[tokio::test]
    async fn excessive_pending_bytes_close_the_native_channel() {
        let (_native, _connection, mut processor, mut events) = open_fixture().await;
        for _ in 0..MAX_PENDING_BYTES / MAX_MESSAGE_SIZE {
            processor.process(7, &vec![0; MAX_MESSAGE_SIZE]).unwrap();
        }
        processor.process(7, &[1]).unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(7))
        ));
    }

    // oracle: mutant zero-length messages bypass the payload bound and grow the queue without limit.
    #[tokio::test]
    async fn excessive_pending_empty_messages_close_the_native_channel() {
        let (_native, _connection, mut processor, mut events) = open_fixture().await;
        for _ in 0..=MAX_PENDING_MESSAGES {
            processor.process(7, &[]).unwrap();
        }
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(7))
        ));
    }

    // oracle: mutant treating a zero-byte native message as EOF or repeating it instead of reading the next message.
    #[tokio::test]
    async fn native_writes_preserve_empty_messages_between_nonempty_messages() {
        let (_task, _processor, mut events, mut pipe) = pipe_fixture();
        let writes = tokio::task::spawn_blocking(move || {
            for payload in [b"first".as_slice(), b"", b"last"] {
                assert_eq!(pipe.write(payload).unwrap(), payload.len());
            }
            pipe
        });
        for expected in [b"first".as_slice(), b"", b"last"] {
            let event = timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
            let ServerEvent::DynamicChannelWithCompletion(batch, completion) = event else {
                panic!("expected a native data message");
            };
            assert_eq!(batch.channel_id(), 7);
            assert_eq!(batch.messages().len(), 1);
            let pdu: DrdynvcServerPdu = decode(&batch.messages()[0].encode_unframed_pdu().unwrap()).unwrap();
            let DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) = pdu else {
                panic!("expected a short data PDU");
            };
            assert_eq!(data.data(), expected);
            completion.send(()).unwrap();
        }
        let _pipe = writes.await.unwrap();
    }

    // oracle: mutant truncating a native write at an internal pipe buffer boundary.
    #[tokio::test]
    async fn a_large_native_write_remains_one_complete_dvc_message() {
        let (_task, _processor, mut events, mut pipe) = pipe_fixture();
        let writes = tokio::task::spawn_blocking(move || {
            assert_eq!(pipe.write(&vec![0x6b; 1024 * 1024]).unwrap(), 1024 * 1024);
            pipe
        });
        let event = timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
        let ServerEvent::DynamicChannelWithCompletion(batch, completion) = event else {
            panic!("expected a native data message");
        };
        let mut payload = Vec::new();
        for (index, message) in batch.messages().iter().enumerate() {
            let pdu: DrdynvcServerPdu = decode(&message.encode_unframed_pdu().unwrap()).unwrap();
            match pdu {
                DrdynvcServerPdu::Data(DrdynvcDataPdu::DataFirst(data)) => {
                    assert_eq!(index, 0);
                    assert_eq!(data.length(), 1024 * 1024);
                    payload.extend_from_slice(data.data());
                }
                DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) => payload.extend_from_slice(data.data()),
                _ => panic!("expected uncompressed data"),
            }
        }
        assert_eq!(payload, vec![0x6b; 1024 * 1024]);
        completion.send(()).unwrap();
        let _pipe = writes.await.unwrap();
    }

    // oracle: mutant reading dynamic consumer messages up to the static limit lets one DVC message exceed the 1 MiB bound.
    #[tokio::test]
    async fn a_dynamic_native_write_above_the_message_limit_ends_the_pump_without_forwarding() {
        let (server, mut pipe) = test_pipe();
        let (sender, mut events) = mpsc::unbounded_channel();
        let endpoint = Endpoint::Dynamic(ChannelGuard { id: 7, sender });
        let (_processor, receiver) = NativeDvc::new(NOW_CHANNEL, MAX_MESSAGE_SIZE, None, None);
        let write = tokio::task::spawn_blocking(move || {
            // The pump may close its end before the write completes.
            let _written = pipe.write(&vec![0x6b; MAX_MESSAGE_SIZE + 1]);
            pipe
        });
        let result = timeout(Duration::from_secs(5), pump(&endpoint, &server, receiver))
            .await
            .expect("the pump kept waiting for the client to take an oversized message");
        assert!(result.is_err());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        let _pipe = write.await.unwrap();
    }

    // oracle: mutant keeping a native process connected after reassignment to a different desktop.
    #[tokio::test]
    async fn rebinding_closes_the_native_channel_and_rejects_the_old_session() {
        let (native, _connection, _processor, mut events) = open_fixture().await;
        native.set_session(8);
        let event = timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
        assert!(matches!(event, ServerEvent::CloseDynamicChannel(7)));
        assert!(native.open(5, NOW_CHANNEL).await.is_err());
    }

    // oracle: mutant retaining an allocated DVC after the control caller abandons its open request.
    #[tokio::test]
    async fn abandoned_open_releases_the_reserved_channel() {
        let native = Arc::new(NativeChannels::default());
        native.set_session(5);
        let (sender, mut events) = mpsc::unbounded_channel();
        let _connection = native.attach(sender).unwrap();
        let pending = tokio::spawn({
            let native = Arc::clone(&native);
            async move { native.open(5, NOW_CHANNEL).await }
        });
        let ServerEvent::CreateDynamicChannel(build) = events.recv().await.unwrap() else {
            panic!("expected a creation request");
        };
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        let _processor = build(7);
        assert!(matches!(events.recv().await, Some(ServerEvent::CloseDynamicChannel(7))));
    }

    // oracle: spec MS-RDPEWA §2.1 names the dynamic channel `WebAuthN_Channel`; regression webauthn.dll in the session was refused it while the NOW host held its channel.
    #[tokio::test]
    async fn a_webauthn_channel_opens_beside_the_now_channel() {
        let (native, _connection, _now, mut events) = open_fixture().await;
        let (opened, processor) = open_dynamic_answered(&native, &mut events, "WebAuthN_Channel", 8).await;
        opened.unwrap();
        assert_eq!(processor.channel_name(), "WebAuthN_Channel");
    }

    // oracle: spec MS-RDPECAM section 2.1 names the enumeration channel; the native log shows this open being refused.
    #[tokio::test]
    async fn a_camera_enumerator_opens_beside_the_now_channel() {
        let (native, _connection, _now, mut events) = open_fixture().await;
        let (answered, answer) = oneshot::channel();
        let _peer = AbortOnDrop(tokio::spawn(async move {
            if let Some(ServerEvent::CreateDynamicChannel(build)) = events.recv().await {
                let mut processor = build(8);
                processor.start(8).unwrap();
                let _ = answered.send((processor, events));
            }
        }));
        let result = native.open(5, "RDCamera_Device_Enumerator").await;
        assert!(result.is_ok(), "camera enumeration was refused: {result:?}");
        let (processor, _events) = timeout(Duration::from_secs(5), answer).await.unwrap().unwrap();
        assert_eq!(processor.channel_name(), "RDCamera_Device_Enumerator");
    }

    // oracle: spec MS-RDPECAM sections 2.1 and 3.3.5.3 use the announced name, not a fixed name prefix.
    #[tokio::test]
    async fn an_announced_camera_can_open_its_device_channel() {
        let (native, _connection, mut enumerator, _pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        let (answered, answer) = oneshot::channel();
        let _peer = AbortOnDrop(tokio::spawn(async move {
            if let Some(ServerEvent::CreateDynamicChannel(build)) = events.recv().await {
                let mut processor = build(31);
                processor.start(31).unwrap();
                let _ = answered.send((processor, events));
            }
        }));
        let result = native.open(5, "Camera_Device_0").await;
        assert!(result.is_ok(), "the announced camera was refused: {result:?}");
        let (processor, _events) = timeout(Duration::from_secs(5), answer).await.unwrap().unwrap();
        assert_eq!(processor.channel_name(), "Camera_Device_0");
    }

    // oracle: reference crates/ironrdp-rdpecam/src/pdu.rs admits 64 MiB samples; MS-RDPECAM section 2.2.3.14 adds Version, MessageId and StreamIndex.
    #[tokio::test]
    async fn a_camera_sample_at_the_client_limit_reaches_the_native_consumer_intact() {
        let (native, _connection, mut enumerator, _enumerator_pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        let (opened, mut device) = open_dynamic_answered(&native, &mut events, "Camera_Device_0", 31).await;
        const SAMPLE_SIZE: usize = 64 * 1024 * 1024;
        assert_eq!(device.max_message_size(), Some(SAMPLE_SIZE + 3));
        let mut pipe = open_test_pipe(&opened.unwrap());
        let mut message = vec![0xa5; SAMPLE_SIZE + 3];
        message[..3].copy_from_slice(&[2, 0x12, 0]);
        device.process(31, &message).unwrap();
        let read = tokio::task::spawn_blocking(move || {
            let mut position = 0;
            let mut record = [0; 1608];
            while position < SAMPLE_SIZE + 3 {
                let count = pipe.read(&mut record).unwrap();
                assert_eq!(&record[..4], &[3, 0, 0, 4]);
                for &byte in &record[8..count] {
                    let expected = match position {
                        0 => 2,
                        1 => 0x12,
                        2 => 0,
                        _ => 0xa5,
                    };
                    assert_eq!(byte, expected, "sample byte {position}");
                    position += 1;
                }
            }
            assert_eq!(position, SAMPLE_SIZE + 3);
        });
        timeout(Duration::from_secs(30), read).await.unwrap().unwrap();
    }

    // oracle: spec MS-RDPECAM section 1.3.3 requires the server to stop using a removed device channel.
    #[tokio::test]
    async fn removing_a_camera_cancels_its_open_before_the_peer_answers() {
        let (native, _connection, mut enumerator, _pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        let pending = tokio::spawn({
            let native = Arc::clone(&native);
            async move { native.open(5, "Camera_Device_0").await }
        });
        let Some(ServerEvent::CreateDynamicChannel(build)) =
            timeout(Duration::from_secs(5), events.recv()).await.unwrap()
        else {
            panic!("expected a device channel request");
        };
        enumerator.process(30, b"\x02\x06Camera_Device_0\x00").unwrap();
        let result = timeout(Duration::from_secs(1), pending)
            .await
            .expect("the open waited for the peer after the camera was removed")
            .unwrap();
        assert!(result.is_err());
        let _late_processor = build(31);
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(31))
        ));
    }

    // oracle: spec MS-RDPECAM section 1.3.3 requires all instances of the removed device channel to stop.
    #[tokio::test]
    async fn removing_a_camera_closes_its_open_channels_and_revokes_its_name() {
        let (native, _connection, mut enumerator, _pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        let (first, _first) = open_dynamic_answered(&native, &mut events, "Camera_Device_0", 31).await;
        first.unwrap();
        let (second, _second) = open_dynamic_answered(&native, &mut events, "Camera_Device_0", 32).await;
        second.unwrap();
        enumerator.process(30, b"\x02\x06Camera_Device_0\x00").unwrap();
        let mut closed = Vec::new();
        for _ in 0..2 {
            let Some(ServerEvent::CloseDynamicChannel(id)) =
                timeout(Duration::from_secs(1), events.recv()).await.unwrap()
            else {
                panic!("expected a removed camera channel to close");
            };
            closed.push(id);
        }
        closed.sort_unstable();
        assert_eq!(closed, [31, 32]);
        assert!(native.open(5, "Camera_Device_0").await.is_err());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // oracle: mutant retaining camera authorization after its enumeration channel closes leaves stale device channels usable.
    #[tokio::test]
    async fn closing_camera_enumeration_closes_its_device_channels() {
        let (native, _connection, mut enumerator, pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        let (opened, _device) = open_dynamic_answered(&native, &mut events, "Camera_Device_0", 31).await;
        opened.unwrap();
        drop(pipe);
        let mut closed = Vec::new();
        for _ in 0..2 {
            let Some(ServerEvent::CloseDynamicChannel(id)) =
                timeout(Duration::from_secs(1), events.recv()).await.unwrap()
            else {
                panic!("expected the enumeration and camera device to close");
            };
            closed.push(id);
        }
        closed.sort_unstable();
        assert_eq!(closed, [30, 31]);
        assert!(native.open(5, "Camera_Device_0").await.is_err());
    }

    // oracle: mutant sharing camera authorization across names, connections or session bindings opens an unannounced endpoint.
    #[tokio::test]
    async fn camera_announcements_do_not_authorize_other_names_connections_or_bindings() {
        let (native, _connection, mut enumerator, _pipe, _events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        assert!(native.open(5, "Camera_Device_1").await.is_err());
        assert!(native.open(5, "AUDIO_PLAYBACK_DVC").await.is_err());
        assert!(native.open(6, "Camera_Device_0").await.is_err());
        let (other, _other_connection, mut other_events) = connected();
        assert!(other.open(5, "Camera_Device_0").await.is_err());
        assert!(matches!(other_events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        native.set_session(6);
        native.set_session(5);
        assert!(native.open(5, "Camera_Device_0").await.is_err());
    }

    // oracle: mutant accepting a reserved camera name replaces the authorization or limits of a fixed endpoint.
    #[tokio::test]
    async fn a_reserved_camera_name_closes_enumeration_without_closing_now() {
        let (native, _connection, mut enumerator, _pipe, mut events) = camera_fixture().await;
        let (now, _now) = open_dynamic_answered(&native, &mut events, NOW_CHANNEL, 7).await;
        let _now_pipe = open_test_pipe(&now.unwrap());
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Devolutions::Now::Agent\x00")
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(30))
        ));
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert!(
            native.open(5, NOW_CHANNEL).await.is_err(),
            "the original NOW instance was lost"
        );
    }

    // oracle: mutant authorizing railenc as a camera endpoint mixes device traffic with native graphics orders.
    #[tokio::test]
    async fn a_camera_cannot_reserve_the_native_window_encoder() {
        let (_native, _connection, mut enumerator, _pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00railenc\x00")
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(30))
        ));
    }

    // oracle: spec MS-RDPERP §2.2.1.3: an invalid window-order header cannot be forwarded.
    #[tokio::test]
    async fn malformed_native_window_headers_close_the_real_pipe_immediately() {
        use ironrdp_pdu::rdp::client_info::ClientInfoFlags;

        let (native, _connection, mut events) = connected();
        native.configure_remote_app(true);
        let mut handler = native.connection_handler();
        handler.on_client_info(ClientInfoFlags::RAIL).unwrap();
        assert!(
            native.open(5, "railenc").await.is_err(),
            "opened before capability negotiation"
        );
        handler
            .on_remote_app_capabilities(native.remote_app_capabilities().as_ref())
            .unwrap();
        assert!(native.open(6, "railenc").await.is_err(), "opened in another session");
        let name = native.open(5, "railenc").await.unwrap();
        assert!(native.open(5, "railenc").await.is_err(), "opened two native encoders");
        let mut pipe = open_test_pipe(&name);
        let read = tokio::task::spawn_blocking(move || {
            pipe.write_all(&[0x2e, 0, 0]).unwrap();
            pipe.read(&mut [0])
        });
        assert_eq!(
            timeout(Duration::from_secs(5), read).await.unwrap().unwrap().unwrap(),
            0
        );
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // oracle: spec MS-RDPERP §1.5: the native RAIL endpoint reads CHANNEL_PDU_HEADER, including SHOW_PROTOCOL.
    #[tokio::test]
    async fn native_rail_handshake_retains_the_channel_header() {
        let mut fixture = static_fixture(2);
        fixture.processor.process(&[5, 0, 8, 0, 0xb1, 0x1d, 0, 0]).unwrap();
        let mut pipe = fixture.pipe;
        let record = tokio::task::spawn_blocking(move || {
            let mut record = [0; 32];
            let read = pipe.read(&mut record).unwrap();
            record[..read].to_vec()
        });
        assert_eq!(
            timeout(Duration::from_secs(5), record).await.unwrap().unwrap(),
            [8, 0, 0, 0, 0x13, 0, 0, 0, 5, 0, 8, 0, 0xb1, 0x1d, 0, 0]
        );
    }

    // oracle: spec MS-RDPBCGR §2.2.6.1.1 and MS-RDPERP §1.5: each RAIL chunk carries total length and fragment flags.
    #[tokio::test]
    async fn native_rail_chunks_repeat_total_length_and_show_protocol() {
        let mut fixture = static_fixture(2);
        fixture.processor.process(&vec![0x55; 3201]).unwrap();
        let mut pipe = fixture.pipe;
        let records = tokio::task::spawn_blocking(move || {
            let mut records = Vec::new();
            for _ in 0..3 {
                let mut record = [0; 1608];
                let read = pipe.read(&mut record).unwrap();
                records.push(record[..read].to_vec());
            }
            records
        });
        let records = timeout(Duration::from_secs(5), records).await.unwrap().unwrap();
        assert_eq!(records.iter().map(Vec::len).collect::<Vec<_>>(), [1608, 1608, 9]);
        for (record, flags) in records.iter().zip([0x11, 0x10, 0x12]) {
            assert_eq!(&record[..8], &[0x81, 0x0c, 0, 0, flags, 0, 0, 0]);
            assert!(record[8..].iter().all(|value| *value == 0x55));
        }
    }

    // oracle: spec MS-RDPECAM section 1.3.1 requires version negotiation before device notifications.
    #[tokio::test]
    async fn an_announcement_before_version_negotiation_does_not_authorize_a_device() {
        let (native, _connection, mut events) = connected();
        let (opened, mut enumerator) =
            open_dynamic_answered(&native, &mut events, "RDCamera_Device_Enumerator", 30).await;
        let _pipe = open_test_pipe(&opened.unwrap());
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        assert!(native.open(5, "Camera_Device_0").await.is_err());
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(30))
        ));
    }

    // oracle: mutant omitting or leaking camera permits breaks the four-open resource bound or prevents reuse.
    #[tokio::test]
    async fn camera_opens_are_bounded_and_reusable() {
        let (native, _connection, mut enumerator, _pipe, mut events) = camera_fixture().await;
        enumerator
            .process(30, b"\x02\x05C\x00a\x00m\x00\x00\x00Camera_Device_0\x00")
            .unwrap();
        let mut devices = Vec::new();
        for id in 31..35 {
            let (opened, device) = open_dynamic_answered(&native, &mut events, "Camera_Device_0", id).await;
            opened.unwrap();
            devices.push(device);
        }
        assert!(native.open(5, "Camera_Device_0").await.is_err());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        drop(devices.pop());
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(34))
        ));
        let (opened, _device) = open_dynamic_answered(&native, &mut events, "Camera_Device_0", 35).await;
        opened.unwrap();
    }

    // oracle: regression webauthn.dll opens a new channel for each API call, so a closed channel that kept its instance fails later calls.
    #[tokio::test]
    async fn closed_webauthn_channels_free_their_instances() {
        let instances = DYNAMIC_CHANNELS
            .iter()
            .find(|endpoint| endpoint.name == "WebAuthN_Channel")
            .unwrap()
            .instances;
        let (native, _connection, mut events) = connected();
        for channel_id in 1..=u32::try_from(instances).unwrap() + 1 {
            let (opened, processor) = open_dynamic_answered(&native, &mut events, "WebAuthN_Channel", channel_id).await;
            opened.unwrap();
            drop(processor);
            let closed = timeout(Duration::from_secs(5), events.recv()).await.unwrap();
            assert!(matches!(closed, Some(ServerEvent::CloseDynamicChannel(id)) if id == channel_id));
        }
    }

    // oracle: mutant removing the instance limit lets any process in the session allocate client channels without bound.
    #[tokio::test]
    async fn opens_beyond_the_instance_limit_are_refused_without_asking_the_client() {
        for endpoint in DYNAMIC_CHANNELS {
            let (native, _connection, mut events) = connected();
            let mut processors = Vec::new();
            for channel_id in 1..=u32::try_from(endpoint.instances).unwrap() {
                let (opened, processor) = open_dynamic_answered(&native, &mut events, endpoint.name, channel_id).await;
                opened.unwrap();
                processors.push(processor);
            }
            assert!(native.open(5, endpoint.name).await.is_err(), "{}", endpoint.name);
            assert!(
                matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
                "{}",
                endpoint.name
            );
            // Dropping a processor closes its channel, so they stay alive until the refusal is checked.
            drop(processors);
        }
    }

    // oracle: regression TermService requests dynamic endpoints that it serves itself, such as AUDIO_PLAYBACK_DVC, during logon.
    #[tokio::test]
    async fn opening_an_unlisted_dynamic_channel_is_refused_without_asking_the_client() {
        let (native, _connection, mut events) = connected();
        assert!(native.open(5, "AUDIO_PLAYBACK_DVC").await.is_err());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // oracle: doc ReadFile fails with ERROR_BROKEN_PIPE once the client end of a pipe closes, which is how a WTS caller closes its channel.
    #[tokio::test]
    async fn a_consumer_closing_its_pipe_ends_the_pump_without_error() {
        let (server, mut pipe) = test_pipe();
        let (sender, mut events) = mpsc::unbounded_channel();
        let endpoint = Endpoint::Dynamic(ChannelGuard { id: 7, sender });
        let (_processor, receiver) = NativeDvc::new(NOW_CHANNEL, MAX_MESSAGE_SIZE, None, None);
        let pump = tokio::spawn(async move { pump(&endpoint, &server, receiver).await });
        // A forwarded message shows the pump has connected, so closing the pipe now cannot fail the connection instead.
        pipe.write_all(b"x").unwrap();
        let Some(ServerEvent::DynamicChannelWithCompletion(_, completion)) =
            timeout(Duration::from_secs(5), events.recv()).await.unwrap()
        else {
            panic!("expected a forwarded message");
        };
        completion.send(()).unwrap();
        drop(pipe);
        timeout(Duration::from_secs(5), pump).await.unwrap().unwrap().unwrap();
    }

    // oracle: mutant reporting a different limit than the one exceeded hides why the channel closed.
    #[test]
    fn an_oversized_message_is_refused_as_too_large() {
        let (inbox, _queue) = Inbox::new(MAX_MESSAGE_SIZE);
        assert_eq!(
            inbox.admit(&vec![0; MAX_MESSAGE_SIZE + 1]),
            Err(Refusal::MessageTooLarge)
        );
    }

    // oracle: mutant reporting a different limit than the one exceeded hides why the channel closed.
    #[test]
    fn bytes_beyond_the_budget_are_refused_as_pending_bytes() {
        let (inbox, _queue) = Inbox::new(MAX_MESSAGE_SIZE);
        for _ in 0..MAX_PENDING_BYTES / MAX_MESSAGE_SIZE {
            assert_eq!(inbox.admit(&vec![0; MAX_MESSAGE_SIZE]), Ok(()));
        }
        assert_eq!(inbox.admit(&[1]), Err(Refusal::PendingBytes));
    }

    // oracle: mutant reporting a different limit than the one exceeded hides why the channel closed.
    #[test]
    fn messages_beyond_the_queue_are_refused_as_pending_messages() {
        let (inbox, _queue) = Inbox::new(MAX_MESSAGE_SIZE);
        for _ in 0..MAX_PENDING_MESSAGES {
            assert_eq!(inbox.admit(&[]), Ok(()));
        }
        assert_eq!(inbox.admit(&[]), Err(Refusal::PendingMessages));
    }

    // oracle: mutant reporting a different limit than the one exceeded hides why the channel closed.
    #[test]
    fn a_message_for_a_departed_consumer_is_refused_as_consumer_gone() {
        let (inbox, queue) = Inbox::new(MAX_MESSAGE_SIZE);
        drop(queue);
        assert_eq!(inbox.admit(&[1]), Err(Refusal::ConsumerGone));
    }

    // oracle: mutant dropping client data that a registered consumer is waiting for stalls its protocol handshake.
    #[test]
    fn client_data_reaches_the_registered_static_consumer_in_order() {
        let slot = Arc::new(StaticSlot::default());
        let (_guard, mut queue, _closed) = slot.register();
        slot.deliver(&STATIC_CHANNELS[0], b"first");
        slot.deliver(&STATIC_CHANNELS[0], b"second");
        assert_eq!(queue.try_recv().unwrap().payload, b"first");
        assert_eq!(queue.try_recv().unwrap().payload, b"second");
    }

    // oracle: spec MS-RDPEFS 2.2.1.5.3, a read response is a 20-byte header and up to the requested Length of data, and the redirector requests 1 MiB.
    #[test]
    fn a_static_message_above_the_dynamic_limit_reaches_the_consumer() {
        let slot = Arc::new(StaticSlot::default());
        let (_guard, mut queue, mut closed) = slot.register();
        let response: Vec<u8> = (0..1024 * 1024 + 20)
            .map(|position| u8::try_from(position % 251).unwrap())
            .collect();
        slot.deliver(&STATIC_CHANNELS[0], &response);
        assert_eq!(queue.try_recv().unwrap().payload, response);
        assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    }

    // oracle: spec MS-RDPEFS 2.2.1.4.4, the 16 MiB print writes the agent accepts carry a 56-byte header; mutant refusing them, or admitting any size, breaks the consumer or the memory bound.
    #[test]
    fn a_static_message_is_admitted_up_to_the_static_limit_only() {
        const LARGEST_PRINT_WRITE: usize = 16 * 1024 * 1024 + 56;
        let slot = Arc::new(StaticSlot::default());
        let (_guard, mut queue, mut closed) = slot.register();
        slot.deliver(&STATIC_CHANNELS[0], &vec![0; LARGEST_PRINT_WRITE]);
        assert_eq!(queue.try_recv().unwrap().payload.len(), LARGEST_PRINT_WRITE);
        // oracle: mutant rejecting the inclusive limit introduces an off-by-one refusal.
        slot.deliver(&STATIC_CHANNELS[0], &vec![0; MAX_STATIC_MESSAGE_SIZE]);
        assert_eq!(queue.try_recv().unwrap().payload.len(), MAX_STATIC_MESSAGE_SIZE);
        slot.deliver(&STATIC_CHANNELS[0], &vec![0; MAX_STATIC_MESSAGE_SIZE + 1]);
        assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
    }

    // oracle: mutant replaying data from before the consumer attached feeds it half of a conversation it never began.
    #[test]
    fn client_data_without_a_static_consumer_is_dropped_not_replayed() {
        let slot = Arc::new(StaticSlot::default());
        slot.deliver(&STATIC_CHANNELS[0], b"early");
        let (_guard, mut queue, _closed) = slot.register();
        slot.deliver(&STATIC_CHANNELS[0], b"late");
        assert_eq!(queue.try_recv().unwrap().payload, b"late");
        assert!(queue.try_recv().is_err());
    }

    // oracle: mutant skipping a refused message hands the consumer a corrupted protocol stream instead of closing it.
    #[test]
    fn a_refused_static_message_ends_the_consumer_and_clears_the_slot() {
        let slot = Arc::new(StaticSlot::default());
        let (_guard, _queue, mut closed) = slot.register();
        for _ in 0..MAX_PENDING_BYTES / MAX_MESSAGE_SIZE {
            slot.deliver(&STATIC_CHANNELS[0], &vec![0; MAX_MESSAGE_SIZE]);
        }
        assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        slot.deliver(&STATIC_CHANNELS[0], &[1]);
        assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
        assert!(slot.state().consumer.is_none());
    }

    // oracle: mutant keeping a consumer registered after its queue is gone queues data that nobody can read.
    #[test]
    fn client_data_for_a_departed_static_consumer_unregisters_it() {
        let slot = Arc::new(StaticSlot::default());
        let (_guard, queue, _closed) = slot.register();
        drop(queue);
        slot.deliver(&STATIC_CHANNELS[0], b"nobody is listening");
        assert!(slot.state().consumer.is_none());
    }

    // oracle: mutant keeping a replaced consumer alive, or letting its teardown unregister its successor, strands the new one.
    #[test]
    fn a_new_static_consumer_replaces_the_previous_one() {
        let slot = Arc::new(StaticSlot::default());
        let (old_guard, mut old_queue, mut old_closed) = slot.register();
        let (_new_guard, mut new_queue, mut new_closed) = slot.register();
        assert!(matches!(
            old_closed.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        slot.deliver(&STATIC_CHANNELS[0], b"for the new consumer");
        assert!(old_queue.try_recv().is_err());
        assert_eq!(new_queue.try_recv().unwrap().payload, b"for the new consumer");
        drop(old_guard);
        slot.deliver(&STATIC_CHANNELS[0], b"after the old teardown");
        assert_eq!(new_queue.try_recv().unwrap().payload, b"after the old teardown");
        assert!(matches!(
            new_closed.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }

    // oracle: mutant leaving a finished consumer registered queues client data that nobody reads.
    #[test]
    fn dropping_a_static_guard_unregisters_its_consumer() {
        let slot = Arc::new(StaticSlot::default());
        let (guard, mut queue, mut closed) = slot.register();
        drop(guard);
        assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
        slot.deliver(&STATIC_CHANNELS[0], b"nobody is listening");
        assert!(matches!(queue.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)));
    }

    // oracle: spec WTSVirtualChannelRead remarks, a read returns at most one 1600-byte chunk of static channel data.
    #[tokio::test]
    async fn static_client_data_reaches_the_pipe_in_chunks_of_at_most_1600_bytes() {
        let mut fixture = static_fixture(0);
        // Position-dependent bytes show that no chunk is repeated, dropped or reordered.
        let mut expected = Vec::new();
        for length in [3201, 5, 1600] {
            let payload: Vec<u8> = (expected.len()..expected.len() + length)
                .map(|position| u8::try_from(position % 251).unwrap())
                .collect();
            fixture.processor.process(&payload).unwrap();
            expected.extend_from_slice(&payload);
        }
        let mut pipe = fixture.pipe;
        let total = expected.len();
        let read = tokio::task::spawn_blocking(move || {
            let mut received = Vec::new();
            let mut buffer = [0; 4096];
            while received.len() < total {
                // A closed pipe reads as zero bytes, which would otherwise loop forever.
                let length = pipe.read(&mut buffer).unwrap();
                assert!(length > 0, "the pipe closed before every chunk arrived");
                assert!(length <= 1600, "a chunk of {length} bytes exceeds the 1600-byte limit");
                received.extend_from_slice(&buffer[..length]);
            }
            received
        });
        let received = timeout(Duration::from_secs(5), read).await.unwrap().unwrap();
        assert_eq!(received, expected);
    }

    // oracle: mutant routing to the wrong client channel, forwarding an empty message, or merging or splitting messages.
    #[tokio::test]
    async fn static_consumer_messages_are_forwarded_to_the_client_channel_one_for_one() {
        let mut fixture = static_fixture(1);
        let mut pipe = fixture.pipe;
        let large = vec![0x6b; 100_000];
        let writes = tokio::task::spawn_blocking({
            let large = large.clone();
            move || {
                for payload in [b"first".as_slice(), b"", &large, b"last"] {
                    assert_eq!(pipe.write(payload).unwrap(), payload.len());
                }
                pipe
            }
        });
        for expected in [b"first".as_slice(), &large, b"last"] {
            let event = timeout(Duration::from_secs(5), fixture.events.recv())
                .await
                .unwrap()
                .unwrap();
            let ServerEvent::StaticChannelWithCompletion {
                name,
                payload,
                completion,
                ..
            } = event
            else {
                panic!("expected static channel data for the client");
            };
            assert_eq!(name.as_str(), Some("rdpsnd"));
            assert_eq!(payload, expected);
            completion.send(()).unwrap();
        }
        let _pipe = writes.await.unwrap();
    }

    // oracle: spec MS-RDPEFS 2.2.1.4.4, a write request is a 56-byte header and its data, and a message above the dynamic limit still crosses whole.
    #[tokio::test]
    async fn static_consumer_messages_above_the_dynamic_limit_reach_the_client_whole() {
        let mut fixture = static_fixture(0);
        let mut pipe = fixture.pipe;
        let request: Vec<u8> = (0..1024 * 1024 + 56)
            .map(|position| u8::try_from(position % 251).unwrap())
            .collect();
        let write = tokio::task::spawn_blocking({
            let request = request.clone();
            move || {
                assert_eq!(pipe.write(&request).unwrap(), request.len());
                pipe
            }
        });
        let event = timeout(Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        let ServerEvent::StaticChannelWithCompletion {
            payload, completion, ..
        } = event
        else {
            panic!("expected static channel data for the client");
        };
        assert_eq!(payload, request);
        completion.send(()).unwrap();
        let _pipe = write.await.unwrap();
    }

    // oracle: mutant ignoring a failed client write leaves the consumer sending into a channel that drops its data.
    #[tokio::test]
    async fn a_dropped_static_write_completion_closes_the_consumer_pipe() {
        let mut fixture = static_fixture(0);
        let mut pipe = fixture.pipe;
        let write = tokio::task::spawn_blocking(move || {
            pipe.write_all(b"lost").unwrap();
            pipe
        });
        let event = timeout(Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        let ServerEvent::StaticChannelWithCompletion { completion, .. } = event else {
            panic!("expected static channel data for the client");
        };
        drop(completion);
        let mut pipe = write.await.unwrap();
        let read = tokio::task::spawn_blocking(move || pipe.read(&mut [0; 16]));
        // The standard library reports a pipe closed by its peer as a read of zero bytes.
        let length = timeout(Duration::from_secs(5), read).await.unwrap().unwrap().unwrap();
        assert_eq!(length, 0);
    }

    // oracle: mutant delivering a zero-length record ends the consumer's read loop as if the channel had closed.
    #[tokio::test]
    async fn an_empty_static_client_message_is_not_delivered_as_a_zero_length_record() {
        let mut fixture = static_fixture(0);
        fixture.processor.process(&[]).unwrap();
        fixture.processor.process(b"after").unwrap();
        let mut pipe = fixture.pipe;
        let read = tokio::task::spawn_blocking(move || {
            let mut buffer = [0; 16];
            let length = pipe.read(&mut buffer).unwrap();
            buffer[..length].to_vec()
        });
        assert_eq!(timeout(Duration::from_secs(5), read).await.unwrap().unwrap(), b"after");
    }

    // oracle: mutant letting a replaced consumer keep waiting for its pipe holds a stale registration open for seconds.
    #[tokio::test]
    async fn a_replaced_static_consumer_stops_without_waiting_for_its_pipe() {
        let (server, _name) = unconnected_test_pipe();
        let (native, _connection, _events) = connected();
        let (binding, sender) = native.bind(5).unwrap();
        let processor = native.processors().next().unwrap();
        let first = start_static(5, &processor.name, &processor.slot, sender, binding, server);
        let _second = processor.slot.register();
        timeout(Duration::from_secs(1), first).await.unwrap().unwrap();
    }

    // oracle: regression replacing a consumer must not admit another queued write until its predecessor is discarded.
    #[tokio::test]
    async fn a_queued_static_write_keeps_its_permit_across_consumer_replacement() {
        let mut fixture = static_fixture(0);
        let mut first_pipe = fixture.pipe;
        let first_write = tokio::task::spawn_blocking(move || {
            first_pipe.write_all(b"old").unwrap();
            first_pipe
        });
        let first_event = timeout(Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        let _first_pipe = first_write.await.unwrap();

        let (server, mut second_pipe) = test_pipe();
        let (binding, sender) = fixture._connection.0.bind(5).unwrap();
        let _replacement = AbortOnDrop(start_static(
            5,
            &fixture.processor.name,
            &fixture.processor.slot,
            sender,
            binding,
            server,
        ));
        let ServerEvent::StaticChannelWithCompletion { is_current, .. } = &first_event else {
            panic!("expected the previous consumer's write");
        };
        assert!(
            !is_current(),
            "replacement must invalidate the queued event synchronously"
        );
        let second_write = tokio::task::spawn_blocking(move || {
            second_pipe.write_all(b"new").unwrap();
            second_pipe
        });
        let _second_pipe = second_write.await.unwrap();
        assert!(
            timeout(Duration::from_millis(100), fixture.events.recv())
                .await
                .is_err(),
            "replacement queued data while the canceled predecessor still occupied the event queue"
        );
        drop(first_event);
        let second_event = timeout(Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        let ServerEvent::StaticChannelWithCompletion {
            payload, completion, ..
        } = second_event
        else {
            panic!("expected the replacement consumer's write");
        };
        assert_eq!(payload, b"new");
        completion.send(()).unwrap();
    }

    // oracle: mutant registering the consumer for another static channel, or after returning, strands the opener's data.
    #[tokio::test]
    async fn opening_a_static_channel_registers_a_consumer_for_its_endpoint() {
        let (native, _connection, mut events) = connected();
        let (opened, queried) = open_static_answered(&native, &mut events, "RDPSND", true).await;
        let pipe_name = opened.unwrap();
        assert_eq!(queried.as_str(), Some("rdpsnd"));
        assert!(pipe_name.starts_with(r"\\.\pipe\IronRdpNativeDvc-"));
        assert!(native.statics[0].state().consumer.is_none());
        assert!(native.statics[1].state().consumer.is_some());
    }

    // oracle: mutant opening a channel outside the enabled set exposes the client's other channels to the host.
    #[tokio::test]
    async fn opening_an_unlisted_static_channel_is_refused_without_asking_the_client() {
        let (native, _connection, mut events) = connected();
        assert!(native.open_static(5, "CLIPRDR").await.is_err());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // oracle: mutant skipping the session check lets a stale caller attach a consumer to another desktop's channels.
    #[tokio::test]
    async fn opening_a_static_channel_for_another_session_is_refused_without_asking_the_client() {
        let (native, _connection, mut events) = connected();
        assert!(native.open_static(9, "RDPDR").await.is_err());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    // oracle: mutant registering a consumer for a channel the client never offered strands its handshake.
    #[tokio::test]
    async fn opening_a_static_channel_the_client_did_not_offer_is_refused() {
        let (native, _connection, mut events) = connected();
        let (opened, queried) = open_static_answered(&native, &mut events, "RDPDR", false).await;
        assert!(opened.is_err());
        assert_eq!(queried.as_str(), Some("rdpdr"));
        assert!(native.statics[0].state().consumer.is_none());
    }

    // oracle: mutant waiting out the join query after the session moved holds a stale open until it times out.
    #[tokio::test]
    async fn a_session_change_abandons_a_pending_static_channel_open() {
        let (native, _connection, mut events) = connected();
        let pending = tokio::spawn({
            let native = Arc::clone(&native);
            async move { native.open_static(5, "RDPDR").await }
        });
        let ServerEvent::StaticChannelJoined(_, _reply) =
            timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap()
        else {
            panic!("expected a join query");
        };
        native.set_session(8);
        let opened = timeout(Duration::from_secs(1), pending).await.unwrap().unwrap();
        assert!(opened.is_err());
    }

    // oracle: mutant leaving the consumer registered after a rebind keeps another desktop's consumer on the channel.
    #[tokio::test]
    async fn rebinding_ends_the_static_consumer_and_clears_its_slot() {
        let (native, _connection, mut events) = connected();
        let (opened, _) = open_static_answered(&native, &mut events, "RDPDR", true).await;
        opened.unwrap();
        assert!(native.statics[0].state().consumer.is_some());
        native.set_session(8);
        // The pump also ends when its pipe connection times out, so the bound is shorter than that timeout.
        timeout(Duration::from_secs(1), async {
            while native.statics[0].state().consumer.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    // oracle: mutant attaching a different set of channels offers the client names that the bridge cannot serve.
    #[test]
    fn the_factory_attaches_exactly_the_native_static_channels() {
        let native = Arc::new(NativeChannels::default());
        let size = DesktopSize {
            width: 1024,
            height: 768,
        };
        let mut acceptor = Acceptor::new(SecurityProtocol::empty(), size, Vec::new(), None);
        native.static_channels().attach(&mut acceptor);
        let mut attach = |name: &'static [u8; 8]| {
            acceptor.attach_dynamic_static_channel(NativeStatic {
                name: ChannelName::from_static(name),
                slot: Arc::default(),
            })
        };
        assert!(!attach(b"rdpdr\0\0\0"));
        assert!(!attach(b"rdpsnd\0\0"));
        assert!(attach(b"cliprdr\0"));
        assert!(attach(b"rail\0\0\0\0"));
    }

    // oracle: spec MS-RDPERP §2.1: RAIL uses the rail static virtual channel.
    #[test]
    fn remote_app_opt_in_attaches_the_native_rail_static_channel() {
        let native = Arc::new(NativeChannels::default());
        native.configure_remote_app(true);
        let mut acceptor = Acceptor::new(
            SecurityProtocol::empty(),
            DesktopSize {
                width: 1024,
                height: 768,
            },
            Vec::new(),
            None,
        );
        native.static_channels().attach(&mut acceptor);
        assert!(!acceptor.attach_dynamic_static_channel(NativeStatic {
            name: ChannelName::from_static(b"rail\0\0\0\0"),
            slot: Arc::default(),
        }));
    }

    async fn assert_burst_delivered(message: usize) {
        // The agent rejects IPC requests above 16 MiB, so no initial stdin can exceed it.
        const BURST: usize = 16 * 1024 * 1024;
        // Distinct fill bytes show each message arrives whole and in order.
        let fill = |index: usize| u8::try_from(index % 251).unwrap();
        let (_task, mut processor, _events, mut pipe) = pipe_fixture();
        // The current-thread runtime cannot run the pump while this loop queues the whole burst.
        for index in 0..BURST / message {
            processor.process(7, &vec![fill(index); message]).unwrap();
        }
        let read = tokio::task::spawn_blocking(move || {
            let mut record = [0; 1608];
            let mut received = 0;
            while received < BURST {
                let count = pipe.read(&mut record).unwrap();
                if count < 8 {
                    break;
                }
                let expected = fill(received / message);
                assert!(
                    record[8..count].iter().all(|byte| *byte == expected),
                    "the record after {received} bytes carries unexpected data"
                );
                received += count - 8;
            }
            received
        });
        let received = timeout(Duration::from_secs(10), read)
            .await
            .expect("the burst was not fully delivered")
            .unwrap();
        assert_eq!(received, BURST);
    }

    async fn open_fixture() -> (
        Arc<NativeChannels>,
        ConnectionGuard,
        Box<dyn DvcServerProcessor>,
        mpsc::UnboundedReceiver<ServerEvent>,
    ) {
        let (native, connection, mut events) = connected();
        let (opened, processor) = open_dynamic_answered(&native, &mut events, NOW_CHANNEL, 7).await;
        opened.unwrap();
        (native, connection, processor, events)
    }

    async fn camera_fixture() -> (
        Arc<NativeChannels>,
        ConnectionGuard,
        Box<dyn DvcServerProcessor>,
        File,
        mpsc::UnboundedReceiver<ServerEvent>,
    ) {
        let (native, connection, mut events) = connected();
        let (opened, mut enumerator) =
            open_dynamic_answered(&native, &mut events, "RDCamera_Device_Enumerator", 30).await;
        let mut pipe = open_test_pipe(&opened.unwrap());
        enumerator.process(30, &[2, 3]).unwrap();
        let pipe = tokio::task::spawn_blocking(move || {
            let mut request = [0; 16];
            let count = pipe.read(&mut request).unwrap();
            assert_eq!(&request[..count], &[2, 0, 0, 0, 3, 0, 0, 0, 2, 3]);
            pipe.write_all(&[2, 4]).unwrap();
            pipe
        });
        let Some(ServerEvent::DynamicChannelWithCompletion(_, completion)) =
            timeout(Duration::from_secs(5), events.recv()).await.unwrap()
        else {
            panic!("expected the camera version response");
        };
        completion.send(()).unwrap();
        (native, connection, enumerator, pipe.await.unwrap(), events)
    }

    /// Opens the dynamic `endpoint` on session 5 while the client creates it as `channel_id`.
    /// Returns the result of the open and the started processor.
    async fn open_dynamic_answered(
        native: &Arc<NativeChannels>,
        events: &mut mpsc::UnboundedReceiver<ServerEvent>,
        endpoint: &'static str,
        channel_id: u32,
    ) -> (anyhow::Result<String>, Box<dyn DvcServerProcessor>) {
        let pending = tokio::spawn({
            let native = Arc::clone(native);
            async move { native.open(5, endpoint).await }
        });
        let ServerEvent::CreateDynamicChannel(build) =
            timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap()
        else {
            panic!("expected a creation request");
        };
        let mut processor = build(channel_id);
        processor.start(channel_id).unwrap();
        (pending.await.unwrap(), processor)
    }

    fn pipe_fixture() -> (AbortOnDrop, NativeDvc, mpsc::UnboundedReceiver<ServerEvent>, File) {
        let (server, pipe) = test_pipe();
        let (sender, events) = mpsc::unbounded_channel();
        let endpoint = Endpoint::Dynamic(ChannelGuard { id: 7, sender });
        let (processor, receiver) = NativeDvc::new(NOW_CHANNEL, MAX_MESSAGE_SIZE, None, None);
        let task = tokio::spawn(async move {
            pump(&endpoint, &server, receiver).await.unwrap();
        });
        (AbortOnDrop(task), processor, events, pipe)
    }

    /// A connection bound to session 5 whose server events the test receives.
    fn connected() -> (
        Arc<NativeChannels>,
        ConnectionGuard,
        mpsc::UnboundedReceiver<ServerEvent>,
    ) {
        let native = Arc::new(NativeChannels {
            create_pipe: NativePipe::for_test,
            ..NativeChannels::default()
        });
        native.set_session(5);
        let (sender, events) = mpsc::unbounded_channel();
        let connection = native.attach(sender).unwrap();
        (native, connection, events)
    }

    /// Opens `endpoint` on session 5 while the client answers the join query with `joined`.
    /// Returns the result of the open and the channel name that the query carried.
    async fn open_static_answered(
        native: &Arc<NativeChannels>,
        events: &mut mpsc::UnboundedReceiver<ServerEvent>,
        endpoint: &'static str,
        joined: bool,
    ) -> (anyhow::Result<String>, ChannelName) {
        let pending = tokio::spawn({
            let native = Arc::clone(native);
            async move { native.open_static(5, endpoint).await }
        });
        let ServerEvent::StaticChannelJoined(name, reply) =
            timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap()
        else {
            panic!("expected a join query");
        };
        reply.send(joined).unwrap();
        (pending.await.unwrap(), name)
    }

    /// A pump for the static channel at `index` in `STATIC_CHANNELS`, bound to session 5, with the consumer's end of the pipe.
    fn static_fixture(index: usize) -> StaticFixture {
        let (server, pipe) = test_pipe();
        let native = Arc::new(NativeChannels::default());
        native.configure_remote_app(true);
        native.set_session(5);
        let (sender, events) = mpsc::unbounded_channel();
        let connection = native.attach(sender.clone()).unwrap();
        let (binding, _) = native.bind(5).unwrap();
        let processor = native.processors().nth(index).unwrap();
        let task = start_static(5, &processor.name, &processor.slot, sender, binding, server);
        StaticFixture {
            _task: AbortOnDrop(task),
            _connection: connection,
            processor,
            events,
            pipe,
        }
    }

    struct StaticFixture {
        _task: AbortOnDrop,
        _connection: ConnectionGuard,
        processor: NativeStatic,
        events: mpsc::UnboundedReceiver<ServerEvent>,
        pipe: File,
    }

    /// A test pipe server end that no consumer has opened yet, with its name.
    fn unconnected_test_pipe() -> (NativePipe, String) {
        let name = format!(
            r"\\.\pipe\IronRdpNativeTest-{}",
            generate_capture_helper_token().unwrap()
        );
        (NativePipe::for_test(&name).unwrap(), name)
    }

    /// A message-mode test pipe: the server end for a pump and the consumer's client end.
    fn test_pipe() -> (NativePipe, File) {
        let (server, name) = unconnected_test_pipe();
        (server, open_test_pipe(&name))
    }

    fn open_test_pipe(name: &str) -> File {
        let pipe = OpenOptions::new().read(true).write(true).open(name).unwrap();
        // SAFETY: This is a live message-pipe handle, exclusively owned by the test.
        unsafe {
            windows::Win32::System::Pipes::SetNamedPipeHandleState(
                HANDLE(pipe.as_raw_handle()),
                Some(&windows::Win32::System::Pipes::PIPE_READMODE_MESSAGE),
                None,
                None,
            )
        }
        .unwrap();
        pipe
    }

    struct AbortOnDrop(JoinHandle<()>);

    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
}
