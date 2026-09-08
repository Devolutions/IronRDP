//! Output-event channel with per-event drop policy.
//!
//! See <https://github.com/Devolutions/IronRDP/issues/1330> for the design rationale.
//!
//! `RdpOutputEvent` mixes rare, correctness-sensitive control events (a failed
//! connection, a completed RAIL launch) with high-frequency display state
//! (framebuffer images, pointer position) that a slow consumer can safely fall
//! behind on as long as it eventually sees the *latest* value. [`DropPolicy`]
//! classifies each variant; [`OutputEventSender`]/[`OutputEventReceiver`] realize
//! the classification without the consumer needing to know which underlying
//! channel a given event arrived on.

use core::num::NonZeroU16;
use std::sync::{Arc, Mutex};

use ironrdp_graphics::pointer::DecodedPointer;
use tokio::sync::{Notify, mpsc, watch};

use crate::rdp::RdpOutputEvent;

/// How an [`RdpOutputEvent`] should be queued when the consumer can't keep up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropPolicy {
    /// Never drop. The sender backpressures until the consumer catches up (or
    /// the session closes).
    MustDeliver,
    /// Only the latest value matters. Sending never blocks; a new value always
    /// replaces whatever was pending.
    LatestOnly,
}

/// The three states a cursor's appearance can be in: showing the platform
/// default, hidden, or showing a specific bitmap. Mutually exclusive, so kept
/// on one [`LatestSlot`] (see [`LatestPayload::PointerAppearance`]) rather than
/// three independent ones: a later appearance always supersedes an earlier one
/// this way regardless of which specific variant either was. Three independent
/// slots would let an older appearance sent to one slot outlive a newer
/// appearance sent to a different slot, since delivery order across
/// independent slots doesn't track send order between them.
enum PointerAppearance {
    Default,
    Hidden,
    Bitmap(Arc<DecodedPointer>),
}

impl From<PointerAppearance> for RdpOutputEvent {
    fn from(appearance: PointerAppearance) -> Self {
        match appearance {
            PointerAppearance::Default => RdpOutputEvent::PointerDefault,
            PointerAppearance::Hidden => RdpOutputEvent::PointerHidden,
            PointerAppearance::Bitmap(pointer) => RdpOutputEvent::PointerBitmap(pointer),
        }
    }
}

/// The payload carried by each [`DropPolicy::LatestOnly`] variant, not the full
/// [`RdpOutputEvent`]: the enum as a whole isn't `Clone` (`AutoReconnecting`
/// carries a `oneshot::Sender`). [`LatestSlot`] takes ownership on delivery
/// rather than cloning out of a borrow, so this type doesn't need `Clone`
/// either. Reconstructed into the matching `RdpOutputEvent` variant on receive.
enum LatestPayload {
    Image {
        buffer: Vec<u32>,
        width: NonZeroU16,
        height: NonZeroU16,
    },
    PointerAppearance(PointerAppearance),
    PointerPosition {
        x: u16,
        y: u16,
    },
}

impl From<LatestPayload> for RdpOutputEvent {
    fn from(payload: LatestPayload) -> Self {
        match payload {
            LatestPayload::Image { buffer, width, height } => RdpOutputEvent::Image { buffer, width, height },
            LatestPayload::PointerAppearance(appearance) => appearance.into(),
            LatestPayload::PointerPosition { x, y } => RdpOutputEvent::PointerPosition { x, y },
        }
    }
}

/// A single-slot "latest value wins" cell. Sending always replaces whatever is
/// currently pending, dropping it; receiving takes ownership of the pending
/// value directly, leaving the slot empty. This is the property
/// [`tokio::sync::watch`] doesn't have: its receiver clones out of a borrowed
/// reference and its sender keeps its own copy after delivery. For a payload
/// as large as a decoded framebuffer, that's a full copy on every delivery and
/// double the steady-state memory for no reason, since the sender's copy is
/// never read again once superseded.
struct LatestSlot<T> {
    value: Mutex<Option<T>>,
    notify: Notify,
}

impl<T> LatestSlot<T> {
    fn new() -> Self {
        Self {
            value: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    /// Replaces whatever's pending. Never blocks.
    fn send(&self, value: T) {
        *self.value.lock().expect("not poisoned") = Some(value);
        self.notify.notify_one();
    }

    /// Takes the pending value without waiting, if there is one.
    fn try_take(&self) -> Option<T> {
        self.value.lock().expect("not poisoned").take()
    }

    /// Waits for a value to be pending, then takes it, leaving the slot empty.
    async fn recv(&self) -> T {
        loop {
            if let Some(value) = self.try_take() {
                return value;
            }
            self.notify.notified().await;
        }
    }
}

/// The [`DropPolicy::LatestOnly`] variants, each on its own [`LatestSlot`] so an
/// update to one never clobbers a pending update to another.
struct LatestSlots {
    image: LatestSlot<LatestPayload>,
    pointer_appearance: LatestSlot<LatestPayload>,
    pointer_position: LatestSlot<LatestPayload>,
}

/// Sending half of the output-event channel. Dispatches each event by
/// [`RdpOutputEvent::drop_policy`].
#[derive(Clone)]
pub struct OutputEventSender {
    must_deliver: mpsc::Sender<RdpOutputEvent>,
    latest: Arc<LatestSlots>,
}

/// Receiving half of the output-event channel.
pub struct OutputEventReceiver {
    must_deliver: mpsc::Receiver<RdpOutputEvent>,
    latest: Arc<LatestSlots>,
}

/// Creates a bounded output-event channel. `capacity` bounds only the
/// [`DropPolicy::MustDeliver`] side; [`DropPolicy::LatestOnly`] events never
/// queue, so no capacity applies to them.
pub fn output_channel(capacity: usize) -> (OutputEventSender, OutputEventReceiver) {
    let (must_deliver_tx, must_deliver_rx) = mpsc::channel(capacity);
    let latest = Arc::new(LatestSlots {
        image: LatestSlot::new(),
        pointer_appearance: LatestSlot::new(),
        pointer_position: LatestSlot::new(),
    });

    let sender = OutputEventSender {
        must_deliver: must_deliver_tx,
        latest: Arc::clone(&latest),
    };
    let receiver = OutputEventReceiver {
        must_deliver: must_deliver_rx,
        latest,
    };
    (sender, receiver)
}

impl OutputEventSender {
    /// Sends `event`, blocking only if its policy is [`DropPolicy::MustDeliver`]
    /// and the channel is full.
    pub async fn send(&self, event: RdpOutputEvent) -> Result<(), mpsc::error::SendError<RdpOutputEvent>> {
        match Self::split(event) {
            Ok(must_deliver) => self.must_deliver.send(must_deliver).await,
            Err(latest) => {
                self.send_latest(latest);
                Ok(())
            }
        }
    }

    /// Like [`Self::send`], but yields `Ok(false)` instead of blocking forever
    /// if `close_receiver` fires first. [`DropPolicy::LatestOnly`] events never
    /// block, so this only matters for [`DropPolicy::MustDeliver`].
    pub async fn send_cancellable(
        &self,
        event: RdpOutputEvent,
        close_receiver: &mut watch::Receiver<bool>,
    ) -> Result<bool, mpsc::error::SendError<RdpOutputEvent>> {
        match Self::split(event) {
            Ok(must_deliver) => {
                tokio::select! {
                    result = self.must_deliver.send(must_deliver) => result.map(|()| true),
                    _ = close_receiver.changed() => Ok(false),
                }
            }
            Err(latest) => {
                self.send_latest(latest);
                Ok(true)
            }
        }
    }

    /// Best-effort send used for out-of-band producers (e.g. a Hyper-V
    /// framebuffer callback) that cannot `.await`. [`DropPolicy::MustDeliver`]
    /// events use `try_send` and are dropped, matching a plain channel's
    /// existing full-queue behavior; [`DropPolicy::LatestOnly`] events always
    /// succeed.
    pub fn try_send(&self, event: RdpOutputEvent) -> Result<(), mpsc::error::TrySendError<RdpOutputEvent>> {
        match Self::split(event) {
            Ok(must_deliver) => self.must_deliver.try_send(must_deliver),
            Err(latest) => {
                self.send_latest(latest);
                Ok(())
            }
        }
    }

    /// Classifies `event` per [`RdpOutputEvent::drop_policy`], returning it
    /// unchanged for [`DropPolicy::MustDeliver`] (`Ok`) or as its
    /// [`LatestPayload`] for [`DropPolicy::LatestOnly`] (`Err`, chosen only so
    /// callers can use `?`-free `match`/`Result` combinators; not a failure).
    fn split(event: RdpOutputEvent) -> Result<RdpOutputEvent, LatestPayload> {
        if event.drop_policy() != DropPolicy::LatestOnly {
            return Ok(event);
        }
        Err(match event {
            RdpOutputEvent::Image { buffer, width, height } => LatestPayload::Image { buffer, width, height },
            RdpOutputEvent::PointerDefault => LatestPayload::PointerAppearance(PointerAppearance::Default),
            RdpOutputEvent::PointerHidden => LatestPayload::PointerAppearance(PointerAppearance::Hidden),
            RdpOutputEvent::PointerPosition { x, y } => LatestPayload::PointerPosition { x, y },
            RdpOutputEvent::PointerBitmap(pointer) => {
                LatestPayload::PointerAppearance(PointerAppearance::Bitmap(pointer))
            }
            other => unreachable!("drop_policy() said LatestOnly for a variant split() doesn't handle: {other:?}"),
        })
    }

    fn send_latest(&self, payload: LatestPayload) {
        let slot = match &payload {
            LatestPayload::Image { .. } => &self.latest.image,
            LatestPayload::PointerAppearance(_) => &self.latest.pointer_appearance,
            LatestPayload::PointerPosition { .. } => &self.latest.pointer_position,
        };
        slot.send(payload);
    }
}

impl OutputEventReceiver {
    /// Waits for the next event, whichever policy produced it.
    ///
    /// Checks [`DropPolicy::MustDeliver`] first, unconditionally, before
    /// waiting on anything else. A [`DropPolicy::LatestOnly`] slot fed by
    /// continuous traffic (a busy frame stream) can be ready on every single
    /// poll; selecting among branches without this check first would let that
    /// traffic starve correctness-sensitive events (`Connected`, a failure,
    /// termination) indefinitely, since a `LatestOnly` branch could win every
    /// single poll for as long as the traffic keeps up. Checking
    /// `must_deliver` unconditionally at the start of every call bounds the
    /// wait to at most the one `LatestOnly` delivery already in flight when a
    /// `MustDeliver` event arrives, never indefinite.
    pub async fn recv(&mut self) -> Option<RdpOutputEvent> {
        match self.must_deliver.try_recv() {
            Ok(event) => return Some(event),
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => return self.drain_latest_or_close(),
        }

        tokio::select! {
            payload = self.latest.image.recv() => Some(payload.into()),
            payload = self.latest.pointer_appearance.recv() => Some(payload.into()),
            payload = self.latest.pointer_position.recv() => Some(payload.into()),
            event = self.must_deliver.recv() => match event {
                Some(event) => Some(event),
                None => self.drain_latest_or_close(),
            },
        }
    }

    /// The `must_deliver` side is closed and empty: no more `MustDeliver`
    /// events will ever arrive. Still deliver one pending `LatestOnly` value if
    /// there is one (e.g. the last frame) before reporting the channel closed,
    /// rather than dropping it silently. Called again on each subsequent
    /// `recv()` while `must_deliver` stays closed, so this drains every
    /// slot that had a pending value, one per call, the same way a bounded
    /// channel drains multiple queued items over multiple `recv()` calls.
    fn drain_latest_or_close(&self) -> Option<RdpOutputEvent> {
        for slot in [
            &self.latest.image,
            &self.latest.pointer_appearance,
            &self.latest.pointer_position,
        ] {
            if let Some(payload) = slot.try_take() {
                return Some(payload.into());
            }
        }
        None
    }
}
