use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{
    Arc,
    mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};

use ironrdp::core::{Encode, EncodeResult, WriteCursor, ensure_size, impl_as_any};
use ironrdp::dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor};
use ironrdp::pdu::PduResult;
use ironrdp::pdu::pdu_other_err;

use crate::error::ffi::IronRdpError;

/// INVARIANT: Each successful update assigns a monotonically increasing ID that is never reused.
static NEXT_DIRECT_DVC_ENDPOINT_ID: AtomicU64 = AtomicU64::new(1);

/// An owned event describing one direct-DVC channel opening.
pub struct DirectDvcEventInner {
    kind: ffi::DirectDvcEventKind,
    channel_name: String,
    channel_id: u32,
    endpoint_id: u64,
    data: Vec<u8>,
}

/// A producer handle for the bounded direct-DVC event queue.
#[derive(Clone)]
pub struct DirectDvcEventSinkInner {
    tx: SyncSender<DirectDvcEventInner>,
    /// INVARIANT: This latch is set after event loss and never cleared.
    /// A poll that reads a Release store with Acquire observes the terminal latch, but concurrent sends and polls are not serialized.
    overflowed: Arc<AtomicBool>,
}

impl DirectDvcEventSinkInner {
    fn send(&self, event: DirectDvcEventInner) -> PduResult<()> {
        if self.overflowed.load(Ordering::Acquire) {
            return Err(pdu_other_err!("direct DVC event queue overflowed"));
        }

        match self.tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.overflowed.store(true, Ordering::Release);
                Err(pdu_other_err!("direct DVC event queue is full"))
            }
            Err(TrySendError::Disconnected(_)) => Err(pdu_other_err!("direct DVC event queue is closed")),
        }
    }

    fn next_endpoint_id() -> PduResult<u64> {
        // Relaxed ordering is sufficient because this counter only assigns unique IDs and publishes no other state.
        NEXT_DIRECT_DVC_ENDPOINT_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| pdu_other_err!("direct DVC endpoint ID space exhausted"))
    }
}

/// The consumer side of a bounded direct-DVC event queue.
pub struct DirectDvcEventQueueInner {
    rx: Receiver<DirectDvcEventInner>,
    overflowed: Arc<AtomicBool>,
}

impl DirectDvcEventQueueInner {
    fn new(queue_size: usize) -> (Self, DirectDvcEventSinkInner) {
        let (tx, rx) = mpsc::sync_channel(queue_size);
        let overflowed = Arc::new(AtomicBool::new(false));
        let sink = DirectDvcEventSinkInner {
            tx,
            overflowed: Arc::clone(&overflowed),
        };
        (Self { rx, overflowed }, sink)
    }

    fn next_event(&mut self) -> Result<Option<DirectDvcEventInner>, Box<IronRdpError>> {
        // Check before dequeue so a latched loss stops delivery even while events remain queued.
        if self.overflowed.load(Ordering::Acquire) {
            return Err("direct DVC event queue overflowed".into());
        }

        match self.rx.try_recv() {
            // Recheck after dequeue to catch an overflow published before this load.
            Ok(_) if self.overflowed.load(Ordering::Acquire) => Err("direct DVC event queue overflowed".into()),
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) if self.overflowed.load(Ordering::Acquire) => {
                Err("direct DVC event queue overflowed".into())
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("direct DVC event queue is closed".into()),
        }
    }
}

#[diplomat::bridge]
pub mod ffi {
    use core::fmt::Write as _;

    use diplomat_runtime::DiplomatWrite;

    use super::{DirectDvcEventInner, DirectDvcEventQueueInner, DirectDvcEventSinkInner};
    use crate::error::ffi::IronRdpError;
    use crate::utils::ffi::VecU8;

    /// Receives owned lifecycle and payload events from configured dynamic virtual channels ([MS-RDPEDYC]).
    ///
    /// Capacity is measured in events, not bytes.
    /// Overflow drops an application-level event without corrupting RDP framing or DVC reassembly.
    /// Channels configured with the same sink share this queue, so overflow stops delivery for every such channel.
    /// Every `next_event` call fails after overflow.
    ///
    /// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/
    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct DirectDvcEventQueue(pub(super) DirectDvcEventQueueInner, pub(super) DirectDvcEventSink);

    impl DirectDvcEventQueue {
        /// Creates a queue with a positive event capacity.
        pub fn new(queue_size: u32) -> Result<Box<Self>, Box<IronRdpError>> {
            let queue_size = usize::try_from(queue_size).map_err(|_| "invalid direct DVC event queue size")?;
            if queue_size == 0 {
                return Err("direct DVC event queue size must be greater than zero".into());
            }
            let (queue, sink) = DirectDvcEventQueueInner::new(queue_size);
            Ok(Box::new(Self(queue, DirectDvcEventSink(sink))))
        }

        /// Returns the next event, or `None` when the queue is empty.
        ///
        /// Returns an error after overflow or when the queue is closed.
        pub fn next_event(&mut self) -> Result<Option<Box<DirectDvcEvent>>, Box<IronRdpError>> {
            Ok(self.0.next_event()?.map(DirectDvcEvent).map(Box::new))
        }

        /// Returns a clone of the queue's producer handle.
        #[diplomat::attr(auto, getter = "sink")]
        pub fn get_sink(&self) -> Box<DirectDvcEventSink> {
            Box::new(self.1.clone())
        }
    }

    /// A cloneable producer handle for a [`DirectDvcEventQueue`].
    #[diplomat::opaque]
    #[derive(Clone)]
    pub struct DirectDvcEventSink(pub(super) DirectDvcEventSinkInner);

    /// Configures event-driven listeners for dynamic virtual channels ([MS-RDPEDYC]).
    ///
    /// Channel names must be unique and must not match an existing dynamic-channel listener.
    /// Each registered name can have multiple concurrent openings, each with a distinct endpoint ID.
    ///
    /// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/
    #[diplomat::opaque_mut]
    #[derive(Clone)]
    pub struct DirectDvcConfig {
        pub event_sink: DirectDvcEventSink,
        pub channel_names: Vec<String>,
    }

    impl DirectDvcConfig {
        /// Creates a configuration that sends events to `event_sink`.
        pub fn new(event_sink: &DirectDvcEventSink) -> Box<Self> {
            Box::new(Self {
                event_sink: event_sink.clone(),
                channel_names: Vec::new(),
            })
        }

        /// Adds a dynamic-channel name to this configuration.
        pub fn add_channel(&mut self, channel_name: &str) {
            self.channel_names.push(channel_name.to_owned());
        }
    }

    /// The kind of lifecycle or payload event observed for a direct DVC ([MS-RDPEDYC]).
    ///
    /// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/
    pub enum DirectDvcEventKind {
        /// A dynamic channel opened.
        Opened,
        /// Data arrived on a dynamic channel.
        Data,
        /// A dynamic channel closed.
        Closed,
    }

    /// An owned open, data, or close event for one dynamic-channel opening ([MS-RDPEDYC]).
    ///
    /// Data bytes remain owned by the event until transferred by `take_data`.
    ///
    /// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/
    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct DirectDvcEvent(pub(super) DirectDvcEventInner);

    impl DirectDvcEvent {
        /// Returns the kind of event.
        #[diplomat::attr(auto, getter = "kind")]
        pub fn get_kind(&self) -> DirectDvcEventKind {
            self.0.kind
        }

        /// Returns the server-assigned dynamic-channel ID.
        #[diplomat::attr(auto, getter = "channel_id")]
        pub fn get_channel_id(&self) -> u32 {
            self.0.channel_id
        }

        /// Returns the configured dynamic-channel name.
        #[diplomat::attr(auto, getter = "channel_name")]
        pub fn get_channel_name(&self, writeable: &mut DiplomatWrite) -> Result<(), Box<IronRdpError>> {
            write!(writeable, "{}", self.0.channel_name)?;
            Ok(())
        }

        /// Returns the identifier for this channel opening.
        #[diplomat::attr(auto, getter = "endpoint_id")]
        pub fn get_endpoint_id(&self) -> u64 {
            self.0.endpoint_id
        }

        /// Transfers the event's owned payload bytes into a `VecU8`.
        pub fn take_data(&mut self) -> Box<VecU8> {
            Box::new(VecU8(core::mem::take(&mut self.0.data)))
        }
    }
}

/// Bridges one registered listener to the direct-DVC event queue ([MS-RDPEDYC]).
///
/// Each successful opening receives a distinct endpoint ID.
/// The listener emits owned payload events.
///
/// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/
pub struct DirectDvcChannelProcessor {
    channel_name: String,
    event_sink: DirectDvcEventSinkInner,
    /// INVARIANT: This is `Some` only for the current channel opening and is cleared by `close`.
    active_endpoint_id: Option<u64>,
}

impl DirectDvcChannelProcessor {
    /// Creates a listener for `channel_name` that writes events to `event_sink`.
    pub fn new(event_sink: &ffi::DirectDvcEventSink, channel_name: &str) -> Self {
        Self {
            channel_name: channel_name.to_owned(),
            event_sink: event_sink.0.clone(),
            active_endpoint_id: None,
        }
    }

    /// Returns whether `endpoint_id` identifies the current channel opening.
    pub fn is_active_endpoint(&self, endpoint_id: u64) -> bool {
        self.active_endpoint_id == Some(endpoint_id)
    }
}

impl_as_any!(DirectDvcChannelProcessor);

impl DvcProcessor for DirectDvcChannelProcessor {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let endpoint_id = DirectDvcEventSinkInner::next_endpoint_id()?;
        self.event_sink.send(DirectDvcEventInner {
            kind: ffi::DirectDvcEventKind::Opened,
            channel_name: self.channel_name.clone(),
            channel_id,
            endpoint_id,
            data: Vec::new(),
        })?;
        self.active_endpoint_id = Some(endpoint_id);
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let endpoint_id = self
            .active_endpoint_id
            .ok_or_else(|| pdu_other_err!("direct DVC channel is not open"))?;
        self.event_sink.send(DirectDvcEventInner {
            kind: ffi::DirectDvcEventKind::Data,
            channel_name: self.channel_name.clone(),
            channel_id,
            endpoint_id,
            data: payload.to_vec(),
        })?;
        Ok(Vec::new())
    }

    fn close(&mut self, channel_id: u32) {
        if let Some(endpoint_id) = self.active_endpoint_id.take() {
            let _ = self.event_sink.send(DirectDvcEventInner {
                kind: ffi::DirectDvcEventKind::Closed,
                channel_name: self.channel_name.clone(),
                channel_id,
                endpoint_id,
                data: Vec::new(),
            });
        }
    }
}

impl DvcClientProcessor for DirectDvcChannelProcessor {}

/// The payload encoded in a DYNVC_DATA PDU ([MS-RDPEDYC] section 2.2.3.2, "DVC Data (DYNVC_DATA)").
///
/// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/11402a8d-7ad8-49a7-a7d4-1458b7ac3fea
pub struct DirectDvcMessage(pub Vec<u8>);

impl Encode for DirectDvcMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DirectDvcMessage"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for DirectDvcMessage {}
