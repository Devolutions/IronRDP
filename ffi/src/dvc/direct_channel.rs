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

static NEXT_DIRECT_DVC_ENDPOINT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectDvcEventKindInner {
    Opened,
    Data,
    Closed,
}

pub struct DirectDvcEventInner {
    kind: DirectDvcEventKindInner,
    channel_name: String,
    channel_id: u32,
    endpoint_id: u64,
    data: Vec<u8>,
}

#[derive(Clone)]
pub struct DirectDvcEventSinkInner {
    tx: SyncSender<DirectDvcEventInner>,
    overflowed: Arc<AtomicBool>,
}

impl DirectDvcEventSinkInner {
    fn send(&self, event: DirectDvcEventInner) -> PduResult<()> {
        match self.tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.overflowed.store(true, Ordering::Release);
                Err(pdu_other_err!("direct DVC event queue is full"))
            }
            Err(TrySendError::Disconnected(_)) => Err(pdu_other_err!("direct DVC event queue is closed")),
        }
    }

    fn record_close(&self, event: DirectDvcEventInner) {
        if matches!(self.tx.try_send(event), Err(TrySendError::Full(_))) {
            self.overflowed.store(true, Ordering::Release);
        }
    }

    fn next_endpoint_id() -> PduResult<u64> {
        NEXT_DIRECT_DVC_ENDPOINT_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| pdu_other_err!("direct DVC endpoint ID space exhausted"))
    }
}

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
        match self.rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) if self.overflowed.swap(false, Ordering::AcqRel) => {
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

    use super::{DirectDvcEventInner, DirectDvcEventKindInner, DirectDvcEventQueueInner, DirectDvcEventSinkInner};
    use crate::error::ffi::IronRdpError;
    use crate::utils::ffi::VecU8;

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct DirectDvcEventQueue(pub(super) DirectDvcEventQueueInner, pub(super) DirectDvcEventSink);

    impl DirectDvcEventQueue {
        pub fn new(queue_size: u32) -> Result<Box<Self>, Box<IronRdpError>> {
            let queue_size = usize::try_from(queue_size).map_err(|_| "invalid direct DVC event queue size")?;
            if queue_size == 0 {
                return Err("direct DVC event queue size must be greater than zero".into());
            }
            let (queue, sink) = DirectDvcEventQueueInner::new(queue_size);
            Ok(Box::new(Self(queue, DirectDvcEventSink(sink))))
        }

        pub fn next_event(&mut self) -> Result<Option<Box<DirectDvcEvent>>, Box<IronRdpError>> {
            Ok(self.0.next_event()?.map(DirectDvcEvent).map(Box::new))
        }

        #[diplomat::attr(auto, getter = "sink")]
        pub fn get_sink(&self) -> Box<DirectDvcEventSink> {
            Box::new(self.1.clone())
        }
    }

    #[diplomat::opaque]
    #[derive(Clone)]
    pub struct DirectDvcEventSink(pub(super) DirectDvcEventSinkInner);

    #[diplomat::opaque_mut]
    #[derive(Clone)]
    pub struct DirectDvcConfig {
        pub event_sink: DirectDvcEventSink,
        pub channel_names: Vec<String>,
    }

    impl DirectDvcConfig {
        pub fn new(event_sink: &DirectDvcEventSink) -> Box<Self> {
            Box::new(Self {
                event_sink: event_sink.clone(),
                channel_names: Vec::new(),
            })
        }

        pub fn add_channel(&mut self, channel_name: &str) {
            self.channel_names.push(channel_name.to_owned());
        }
    }

    pub enum DirectDvcEventKind {
        Opened,
        Data,
        Closed,
    }

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct DirectDvcEvent(pub(super) DirectDvcEventInner);

    impl DirectDvcEvent {
        #[diplomat::attr(auto, getter = "kind")]
        pub fn get_kind(&self) -> DirectDvcEventKind {
            match self.0.kind {
                DirectDvcEventKindInner::Opened => DirectDvcEventKind::Opened,
                DirectDvcEventKindInner::Data => DirectDvcEventKind::Data,
                DirectDvcEventKindInner::Closed => DirectDvcEventKind::Closed,
            }
        }

        #[diplomat::attr(auto, getter = "channel_id")]
        pub fn get_channel_id(&self) -> u32 {
            self.0.channel_id
        }

        #[diplomat::attr(auto, getter = "channel_name")]
        pub fn get_channel_name(&self, writeable: &mut DiplomatWrite) -> Result<(), Box<IronRdpError>> {
            write!(writeable, "{}", self.0.channel_name)?;
            Ok(())
        }

        #[diplomat::attr(auto, getter = "endpoint_id")]
        pub fn get_endpoint_id(&self) -> u64 {
            self.0.endpoint_id
        }

        pub fn take_data(&mut self) -> Box<VecU8> {
            Box::new(VecU8(core::mem::take(&mut self.0.data)))
        }
    }
}

pub struct DirectDvcChannelProcessor {
    channel_name: String,
    event_sink: DirectDvcEventSinkInner,
    active_endpoint_id: Option<u64>,
}

impl DirectDvcChannelProcessor {
    pub fn new(channel_name: &str, event_sink: &ffi::DirectDvcEventSink) -> Self {
        Self {
            channel_name: channel_name.to_owned(),
            event_sink: event_sink.0.clone(),
            active_endpoint_id: None,
        }
    }

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
            kind: DirectDvcEventKindInner::Opened,
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
            kind: DirectDvcEventKindInner::Data,
            channel_name: self.channel_name.clone(),
            channel_id,
            endpoint_id,
            data: payload.to_vec(),
        })?;
        Ok(Vec::new())
    }

    fn close(&mut self, channel_id: u32) {
        if let Some(endpoint_id) = self.active_endpoint_id.take() {
            self.event_sink.record_close(DirectDvcEventInner {
                kind: DirectDvcEventKindInner::Closed,
                channel_name: self.channel_name.clone(),
                channel_id,
                endpoint_id,
                data: Vec::new(),
            });
        }
    }
}

impl DvcClientProcessor for DirectDvcChannelProcessor {}

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

#[cfg(test)]
mod tests {
    use super::ffi::{DirectDvcEvent, DirectDvcEventKind, DirectDvcEventQueue};
    use super::{DirectDvcChannelProcessor, DvcProcessor as _};

    fn new_queue(queue_size: u32) -> Box<DirectDvcEventQueue> {
        match DirectDvcEventQueue::new(queue_size) {
            Ok(queue) => queue,
            Err(_) => panic!("a positive queue size is valid"),
        }
    }

    fn next_event(queue: &mut DirectDvcEventQueue) -> Box<DirectDvcEvent> {
        match queue.next_event() {
            Ok(Some(event)) => event,
            _ => panic!("the expected direct DVC event should be queued"),
        }
    }

    // oracle: mutant preserve payload ownership and endpoint identity across channel reopen
    #[test]
    fn events_preserve_payload_and_use_a_new_endpoint_after_reopen() {
        let mut queue = new_queue(4);
        let sink = queue.get_sink();
        let mut processor = DirectDvcChannelProcessor::new("TestChannel", &sink);

        processor.start(7).expect("the channel should start");
        let opened = next_event(&mut queue);
        assert!(matches!(opened.get_kind(), DirectDvcEventKind::Opened));
        assert_eq!(opened.0.channel_name, "TestChannel");
        assert_eq!(opened.get_channel_id(), 7);
        let first_endpoint_id = opened.get_endpoint_id();
        assert!(processor.is_active_endpoint(first_endpoint_id));

        let mut payload = vec![1, 2, 3];
        processor.process(7, &payload).expect("the channel should accept data");
        payload.fill(0);
        let mut data = next_event(&mut queue);
        assert!(matches!(data.get_kind(), DirectDvcEventKind::Data));
        assert_eq!(data.get_endpoint_id(), first_endpoint_id);
        let bytes = data.take_data();
        let mut received = vec![0; bytes.get_size()];
        assert!(bytes.fill(&mut received).is_ok());
        assert_eq!(received, [1, 2, 3]);

        processor.close(7);
        assert!(!processor.is_active_endpoint(first_endpoint_id));
        let closed = next_event(&mut queue);
        assert!(matches!(closed.get_kind(), DirectDvcEventKind::Closed));
        assert_eq!(closed.get_endpoint_id(), first_endpoint_id);

        processor.start(7).expect("the channel should reopen");
        let reopened = next_event(&mut queue);
        assert!(matches!(reopened.get_kind(), DirectDvcEventKind::Opened));
        assert_ne!(reopened.get_endpoint_id(), first_endpoint_id);
        assert!(!processor.is_active_endpoint(first_endpoint_id));
        assert!(processor.is_active_endpoint(reopened.get_endpoint_id()));
    }

    // oracle: mutant report explicit errors after direct DVC event loss
    #[test]
    fn event_queue_reports_overflow_after_queued_events_are_drained() {
        let mut queue = new_queue(1);
        let sink = queue.get_sink();
        let mut processor = DirectDvcChannelProcessor::new("TestChannel", &sink);

        processor.start(9).expect("the open event should fit");
        assert!(processor.process(9, &[1]).is_err());
        assert!(matches!(queue.next_event(), Ok(Some(_))));
        assert!(queue.next_event().is_err());
    }
}
