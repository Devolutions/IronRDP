//! Contact lifetime state machine shared by touch and pen producers.
//!
//! Front-ends sample their local digitizer and feed each sample to [`transition`], which returns the
//! `contactFlags` to put on the wire and the state to remember for the next sample.
//! This keeps every emitted flag set inside the legal combinations of [MS-RDPEI] 2.2.3.3.1.1 and
//! 2.2.3.7.1.1, following the figure "Lifetime of a touch or pen contact" in [MS-RDPEI] 3.1.1.1.
//!
//! [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d

use crate::pdu::{PenContactFlags, TouchContact, TouchContactFlags, TouchEventPdu, TouchFrame};

/// State of a contact the server knows about.
///
/// A contact that is out of range is not tracked, so it has no variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactState {
    /// In range of the digitizer, not touching it.
    Hovering,
    /// Touching the digitizer.
    Engaged,
}

/// One local sample of a contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactSample {
    /// The digitizer still detects the contact.
    pub in_range: bool,
    /// The contact touches the digitizer.
    ///
    /// Ignored when `in_range` is false.
    pub in_contact: bool,
    /// The local system canceled the contact, for example because it became a palm or a gesture.
    pub canceled: bool,
}

/// The wire flags for one sample and the state that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactTransition {
    /// Touch `contactFlags` for this sample.
    pub flags: TouchContactFlags,
    /// State after this sample, or `None` when the contact left the range and is no longer tracked.
    pub next: Option<ContactState>,
}

impl ContactTransition {
    /// Pen `contactFlags` for this sample.
    ///
    /// Pen and touch contacts share the same flag values and legal combinations.
    #[must_use]
    pub fn pen_flags(self) -> PenContactFlags {
        PenContactFlags::from_bits_truncate(self.flags.bits())
    }

    /// Whether this sample takes the contact out of the engaged state.
    ///
    /// Such a contact is sent at its last engaged position, because the position may change only after
    /// the transition ([MS-RDPEI] 3.1.1.1 "Touch Contact State Transitions").
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    #[must_use]
    pub fn keeps_position(self) -> bool {
        self.flags.contains(TouchContactFlags::UP)
    }
}

/// Whether a contact with these flags stays in its state, so that dropping it loses only movement.
///
/// These are the two legal flag sets that keep a contact hovering or engaged ([MS-RDPEI] 2.2.3.3.1.1
/// "RDPINPUT_CONTACT_DATA").
///
/// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
#[must_use]
pub fn is_steady(flags: TouchContactFlags) -> bool {
    flags == TouchContactFlags::UPDATE | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT
        || flags == TouchContactFlags::UPDATE | TouchContactFlags::INRANGE
}

/// [`is_steady`] for pen contacts, which share the touch flag values and legal combinations.
#[must_use]
pub fn is_steady_pen(flags: PenContactFlags) -> bool {
    is_steady(TouchContactFlags::from_bits_truncate(flags.bits()))
}

/// Advances a contact by one local sample, following the figure "Lifetime of a touch or pen contact" in
/// [MS-RDPEI] 3.1.1.1 "Touch Contact State Transitions".
///
/// `previous` is the state returned by the last transition of this contact, or `None` for a contact the
/// server does not know about.
/// Returns `None` when the sample produces no wire contact: a contact that stays out of range, or a cancel
/// for a contact that was never sent.
///
/// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
#[must_use]
pub fn transition(previous: Option<ContactState>, sample: ContactSample) -> Option<ContactTransition> {
    use ContactState::{Engaged, Hovering};

    let transition = |flags, next| Some(ContactTransition { flags, next });

    if sample.canceled {
        return match previous {
            Some(Hovering) => transition(TouchContactFlags::UPDATE | TouchContactFlags::CANCELED, None),
            Some(Engaged) => transition(TouchContactFlags::UP | TouchContactFlags::CANCELED, None),
            None => None,
        };
    }

    let in_contact = sample.in_contact && sample.in_range;
    match (previous, sample.in_range, in_contact) {
        (None | Some(Hovering), true, true) => transition(
            TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
            Some(Engaged),
        ),
        (Some(Engaged), true, true) => transition(
            TouchContactFlags::UPDATE | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
            Some(Engaged),
        ),
        (Some(Engaged), true, false) => transition(TouchContactFlags::UP | TouchContactFlags::INRANGE, Some(Hovering)),
        (Some(Engaged), false, _) => transition(TouchContactFlags::UP, None),
        (None | Some(Hovering), true, false) => {
            transition(TouchContactFlags::UPDATE | TouchContactFlags::INRANGE, Some(Hovering))
        }
        (Some(Hovering), false, _) => transition(TouchContactFlags::UPDATE, None),
        (None, false, _) => None,
    }
}

/// Wire flags that take a tracked contact out of range, for example when the window loses focus.
///
/// A hovering contact leaves with `UPDATE` and an engaged one with `UP` ([MS-RDPEI] 3.1.1.1 "Touch Contact
/// State Transitions").
///
/// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
#[must_use]
pub fn release_flags(state: ContactState) -> TouchContactFlags {
    match state {
        ContactState::Hovering => TouchContactFlags::UPDATE,
        ContactState::Engaged => TouchContactFlags::UP,
    }
}

/// Phase of a touch point as reported by windowing toolkits that deliver one event per contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    /// The finger touched the surface.
    Started,
    /// The finger moved while touching the surface.
    Moved,
    /// The finger left the surface.
    Ended,
    /// The system canceled the touch.
    Canceled,
}

#[derive(Debug, Clone)]
struct TrackedTouch {
    external_id: u64,
    contact_id: u8,
    x: i32,
    y: i32,
    pressure: Option<u32>,
}

/// Groups per-contact touch events into RDPEI touch frames.
///
/// Toolkits such as winit report each contact separately, while every RDPEI frame lists all contacts that
/// touch the surface.
/// Events pushed between two [`TouchFrameBuilder::take_event`] calls form one frame, and a contact that
/// changes twice starts a new frame.
/// Every frame restates the contacts that did not change in it, at their last position.
#[derive(Debug)]
pub struct TouchFrameBuilder {
    max_contacts: usize,
    tracked: Vec<TrackedTouch>,
    frames: Vec<TouchFrame>,
    /// Contacts changed in the frame being built.
    current: Vec<TouchContact>,
    current_timestamp_us: Option<u64>,
    oldest_timestamp_us: Option<u64>,
    last_frame_timestamp_us: Option<u64>,
}

impl TouchFrameBuilder {
    /// Creates a builder that tracks at most `max_contacts` contacts, the `maxTouchContacts` sent in `CS_READY`.
    #[must_use]
    pub fn new(max_contacts: u16) -> Self {
        Self {
            max_contacts: usize::from(max_contacts),
            tracked: Vec::new(),
            frames: Vec::new(),
            current: Vec::new(),
            current_timestamp_us: None,
            oldest_timestamp_us: None,
            last_frame_timestamp_us: None,
        }
    }

    /// Whether frames wait for [`TouchFrameBuilder::take_event`].
    #[must_use]
    pub fn has_pending_frames(&self) -> bool {
        self.current_timestamp_us.is_some() || !self.frames.is_empty()
    }

    /// Adds one toolkit event, in desktop coordinates.
    ///
    /// `external_id` identifies the touch point for the toolkit, `pressure` is normalized to 0..=1024, and
    /// `timestamp_us` is a monotonic time in microseconds.
    /// Moves of unknown contacts are ignored, which drops hovering pens that toolkits report as touches.
    /// Contacts beyond `max_contacts` are ignored.
    pub fn push(
        &mut self,
        external_id: u64,
        phase: TouchPhase,
        x: i32,
        y: i32,
        pressure: Option<u32>,
        timestamp_us: u64,
    ) {
        let index = self.tracked.iter().position(|touch| touch.external_id == external_id);
        let (in_range, canceled) = match phase {
            TouchPhase::Started => (true, false),
            TouchPhase::Moved if index.is_some() => (true, false),
            TouchPhase::Moved => return,
            TouchPhase::Ended => (false, false),
            TouchPhase::Canceled => (false, true),
        };
        let sample = ContactSample {
            in_range,
            in_contact: in_range,
            canceled,
        };
        let Some(transition) = transition(index.map(|_| ContactState::Engaged), sample) else {
            return;
        };

        let contact_id = match index {
            Some(index) => self.tracked[index].contact_id,
            None => match self.allocate_contact_id() {
                Some(contact_id) => contact_id,
                None => return,
            },
        };
        if self.current.iter().any(|contact| contact.contact_id == contact_id) {
            self.finish_frame();
        }
        self.current_timestamp_us.get_or_insert(timestamp_us);

        let (x, y) = match index {
            Some(index) if transition.keeps_position() => (self.tracked[index].x, self.tracked[index].y),
            _ => (x, y),
        };
        let pressure = pressure.map(|pressure| pressure.min(1024));
        let mut contact = TouchContact::new(contact_id, x, y, transition.flags);
        if let Some(pressure) = pressure {
            contact = contact.with_pressure(pressure);
        }
        self.current.push(contact);

        match (transition.next, index) {
            (Some(_), Some(index)) => {
                let touch = &mut self.tracked[index];
                (touch.x, touch.y, touch.pressure) = (x, y, pressure);
            }
            (Some(_), None) => self.tracked.push(TrackedTouch {
                external_id,
                contact_id,
                x,
                y,
                pressure,
            }),
            (None, Some(index)) => {
                self.tracked.remove(index);
            }
            (None, None) => {}
        }
    }

    /// Takes the frames built since the last call as one touch event.
    ///
    /// `now_us` is the encoding time, on the clock of the pushed timestamps.
    pub fn take_event(&mut self, now_us: u64) -> Option<TouchEventPdu> {
        self.finish_frame();
        if self.frames.is_empty() {
            return None;
        }
        let oldest_us = self.oldest_timestamp_us.take().unwrap_or(now_us);
        let encode_time = u32::try_from(now_us.saturating_sub(oldest_us) / 1000).unwrap_or(u32::MAX);
        Some(TouchEventPdu::new(encode_time, core::mem::take(&mut self.frames)))
    }

    /// Cancels every contact, for example when the window loses focus.
    ///
    /// The cancellations wait with the other pending frames for [`TouchFrameBuilder::take_event`], so a full
    /// input queue does not lose them.
    pub fn cancel_all(&mut self, now_us: u64) {
        let touches: Vec<(u64, i32, i32)> = self
            .tracked
            .iter()
            .map(|touch| (touch.external_id, touch.x, touch.y))
            .collect();
        for (external_id, x, y) in touches {
            self.push(external_id, TouchPhase::Canceled, x, y, None, now_us);
        }
    }

    /// Forgets every contact and pending frame without sending anything, for when the channel closed.
    pub fn reset(&mut self) {
        self.tracked.clear();
        self.frames.clear();
        self.current.clear();
        self.current_timestamp_us = None;
        self.oldest_timestamp_us = None;
        self.last_frame_timestamp_us = None;
    }

    fn finish_frame(&mut self) {
        let Some(timestamp_us) = self.current_timestamp_us.take() else {
            return;
        };
        let mut contacts = core::mem::take(&mut self.current);
        let unchanged: Vec<TouchContact> = self
            .tracked
            .iter()
            .filter(|touch| contacts.iter().all(|contact| contact.contact_id != touch.contact_id))
            .map(|touch| {
                let contact = TouchContact::new(
                    touch.contact_id,
                    touch.x,
                    touch.y,
                    TouchContactFlags::UPDATE | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
                );
                match touch.pressure {
                    Some(pressure) => contact.with_pressure(pressure),
                    None => contact,
                }
            })
            .collect();
        contacts.extend(unchanged);

        let frame_offset = self
            .last_frame_timestamp_us
            .map_or(0, |last| timestamp_us.saturating_sub(last));
        // The transaction ends once no contact is tracked, and the next one starts at offset 0.
        self.last_frame_timestamp_us = (!self.tracked.is_empty()).then_some(timestamp_us);
        self.oldest_timestamp_us.get_or_insert(timestamp_us);
        self.frames.push(TouchFrame::new(frame_offset, contacts));
    }

    fn allocate_contact_id(&self) -> Option<u8> {
        if self.max_contacts <= self.tracked.len() {
            return None;
        }
        (0..=u8::MAX).find(|id| self.tracked.iter().all(|touch| touch.contact_id != *id))
    }
}
