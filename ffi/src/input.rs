/// A pen the server knows about.
struct TrackedPen {
    external_id: u64,
    state: ironrdp::rdpei::contact::ContactState,
    /// Last contact sent for this pen.
    last: ironrdp::rdpei::pdu::PenContact,
}

/// A pen frame no event has carried yet.
struct PendingPenFrame {
    timestamp_us: u64,
    frame: ironrdp::rdpei::pdu::PenFrame,
}

#[diplomat::bridge]
pub mod ffi {
    use std::collections::VecDeque;

    use crate::error::ffi::IronRdpError;
    use crate::pdu::ffi::FastPathInputEventIterator;

    #[diplomat::opaque_mut]
    pub struct InputDatabase(pub ironrdp::input::Database);

    impl InputDatabase {
        pub fn new() -> Box<InputDatabase> {
            Box::new(InputDatabase(ironrdp::input::Database::new()))
        }

        pub fn apply(&mut self, operation: &Operation) -> Box<FastPathInputEventIterator> {
            let res = self.0.apply(core::iter::once(operation.0.clone()));
            Box::new(res.to_vec().into())
        }
    }

    #[diplomat::opaque]
    pub struct Operation(pub ironrdp::input::Operation);

    pub enum OperationType {
        MouseButtonPressed,
        MouseButtonReleased,
        MouseMove,
        WheelRotations,
        KeyPressed,
        KeyReleased,
        UnicodeKeyPressed,
        UnicodeKeyReleased,
    }

    #[diplomat::opaque]
    pub struct MousePosition(pub ironrdp::input::MousePosition);

    impl MousePosition {
        pub fn new(x: u16, y: u16) -> Box<MousePosition> {
            Box::new(MousePosition(ironrdp::input::MousePosition { x, y }))
        }

        pub fn as_move_operation(&self) -> Box<Operation> {
            Box::new(Operation(ironrdp::input::Operation::MouseMove(self.0)))
        }
    }

    #[diplomat::opaque]
    pub struct MouseButton(pub ironrdp::input::MouseButton);

    #[diplomat::enum_convert(ironrdp::input::MouseButton)]
    pub enum MouseButtonType {
        Left = 0,
        Middle = 1,
        Right = 2,
        X1 = 3,
        X2 = 4,
    }

    impl MouseButton {
        pub fn new(button: MouseButtonType) -> Box<MouseButton> {
            Box::new(MouseButton(button.into()))
        }

        pub fn as_operation_mouse_button_pressed(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::MouseButtonPressed(self.0);
            Box::new(Operation(operation))
        }

        pub fn as_operation_mouse_button_released(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::MouseButtonReleased(self.0);
            Box::new(Operation(operation))
        }
    }

    #[diplomat::opaque]
    pub struct WheelRotations(pub ironrdp::input::WheelRotations);

    impl WheelRotations {
        pub fn new(is_vertical: bool, rotation_units: i16) -> Box<WheelRotations> {
            Box::new(WheelRotations(ironrdp::input::WheelRotations {
                is_vertical,
                rotation_units,
            }))
        }

        pub fn as_operation(&self) -> Box<Operation> {
            Box::new(Operation(ironrdp::input::Operation::WheelRotations(self.0)))
        }
    }

    #[diplomat::opaque]
    pub struct Scancode(pub ironrdp::input::Scancode);

    impl Scancode {
        pub fn from_u8(extended: bool, code: u8) -> Box<Scancode> {
            Box::new(Scancode(ironrdp::input::Scancode::from_u8(extended, code)))
        }

        pub fn from_u16(code: u16) -> Box<Scancode> {
            Box::new(Scancode(ironrdp::input::Scancode::from_u16(code)))
        }

        pub fn as_operation_key_pressed(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::KeyPressed(self.0);
            Box::new(Operation(operation))
        }

        pub fn as_operation_key_released(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::KeyReleased(self.0);
            Box::new(Operation(operation))
        }
    }

    #[diplomat::opaque]
    pub struct Char(pub char);

    impl Char {
        pub fn new(c: u32) -> Result<Box<Char>, Box<IronRdpError>> {
            char::from_u32(c)
                .map(|c| Box::new(Char(c)))
                .ok_or_else(|| "Invalid unicode character".into())
        }

        pub fn as_operation_unicode_key_pressed(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::UnicodeKeyPressed(self.0);
            Box::new(Operation(operation))
        }

        pub fn as_operation_unicode_key_released(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::UnicodeKeyReleased(self.0);
            Box::new(Operation(operation))
        }
    }

    /// Phase of a touch point, as windowing toolkits report one event per contact.
    #[diplomat::enum_convert(ironrdp::rdpei::contact::TouchPhase)]
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

    /// Groups per-contact touch events into MS-RDPEI touch frames.
    ///
    /// Push every toolkit event, then hand the builder to `ActiveStage::encode_rdpei_touch`, which
    /// takes the oldest pending frames, at most `MAX_FRAMES_PER_EVENT` per call.
    /// Contact ids and `contactFlags` are derived here, so every frame stays within the legal
    /// combinations of [MS-RDPEI] 2.2.3.3.1.1 "RDPINPUT_CONTACT_DATA".
    /// Frame timing comes from a monotonic clock read at each push.
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    #[diplomat::opaque_mut]
    pub struct TouchFrameBuilder {
        inner: ironrdp::rdpei::contact::TouchFrameBuilder,
        /// Frames taken from `inner` that no event has carried yet, oldest first.
        ///
        /// INVARIANT: `pending_oldest_us` is the builder-clock time of `pending[0]` when `pending` is
        /// not empty, so that each event's `encodeTime` is the age of its oldest frame.
        pending: VecDeque<ironrdp::rdpei::pdu::TouchFrame>,
        pending_oldest_us: u64,
        /// Builder-clock time of the oldest frame of the last event `take_event` returned.
        taken_oldest_us: u64,
        epoch: std::time::Instant,
    }

    /// Frames one `RDPINPUT_TOUCH_EVENT_PDU` can carry: `frameCount` is a TWO_BYTE_UNSIGNED_INTEGER
    /// ([MS-RDPEI] 2.2.3.3 "RDPINPUT_TOUCH_EVENT_PDU", 2.2.2.1 "TWO_BYTE_UNSIGNED_INTEGER").
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    pub(crate) const MAX_FRAMES_PER_EVENT: usize = 0x7FFF;

    impl TouchFrameBuilder {
        /// Tracks at most `max_contacts` contacts, the `max_touch_contacts` given to
        /// `ClientConnector::with_dynamic_channel_rdpei`.
        pub fn new(max_contacts: u16) -> Box<TouchFrameBuilder> {
            Box::new(TouchFrameBuilder {
                inner: ironrdp::rdpei::contact::TouchFrameBuilder::new(max_contacts),
                pending: VecDeque::new(),
                pending_oldest_us: 0,
                taken_oldest_us: 0,
                epoch: std::time::Instant::now(),
            })
        }

        /// Adds one toolkit event for the touch point `external_id`, in desktop coordinates.
        ///
        /// A `Moved` or `Ended` event for an unknown touch point is ignored, as is a `Started` event
        /// beyond `max_contacts`.
        pub fn push(&mut self, external_id: u64, phase: TouchPhase, x: i32, y: i32) {
            let now_us = self.now_us();
            self.inner.push(external_id, phase.into(), x, y, None, now_us);
        }

        /// Like `push`, with the contact pressure normalized to `0..=1024`.
        pub fn push_with_pressure(&mut self, external_id: u64, phase: TouchPhase, x: i32, y: i32, pressure: u32) {
            let now_us = self.now_us();
            self.inner.push(external_id, phase.into(), x, y, Some(pressure), now_us);
        }

        /// Cancels every tracked contact, for example when the window loses focus.
        ///
        /// The cancellations wait with the other pending frames for the next encode.
        pub fn cancel_all(&mut self) {
            let now_us = self.now_us();
            self.inner.cancel_all(now_us);
        }

        /// Forgets every contact and pending frame without sending anything, for when the channel closed.
        pub fn reset(&mut self) {
            self.inner.reset();
            self.pending.clear();
        }

        /// Whether frames wait for the next encode.
        ///
        /// More than `MAX_FRAMES_PER_EVENT` frames, as after a long suspension, take several encodes.
        #[diplomat::attr(auto, getter = "has_pending_frames")]
        pub fn get_has_pending_frames(&self) -> bool {
            self.inner.has_pending_frames() || !self.pending.is_empty()
        }

        /// Takes the oldest pending frames, at most `MAX_FRAMES_PER_EVENT`, as one touch event.
        ///
        /// The frames built since the last call queue behind the frames an earlier call left pending,
        /// so frames leave in the order they were built.
        pub(crate) fn take_event(&mut self) -> Option<ironrdp::rdpei::pdu::TouchEventPdu> {
            let now_us = self.now_us();
            if let Some(event) = self.inner.take_event(now_us) {
                if self.pending.is_empty() {
                    self.pending_oldest_us = now_us.saturating_sub(u64::from(event.encode_time) * 1000);
                }
                self.pending.extend(event.frames);
            }
            if self.pending.is_empty() {
                return None;
            }

            let count = self.pending.len().min(MAX_FRAMES_PER_EVENT);
            let frames: Vec<_> = self.pending.drain(..count).collect();
            self.taken_oldest_us = self.pending_oldest_us;
            // Each frame's offset is relative to the frame before it, so the time of the next pending
            // frame is the time of the taken frames plus the offsets after the first.
            // The offset is 0 at the start of a transaction, which can only age the remainder.
            let taken_span_us: u64 = frames.iter().skip(1).map(|frame| frame.frame_offset).sum();
            if let Some(next) = self.pending.front() {
                self.pending_oldest_us = self
                    .pending_oldest_us
                    .saturating_add(taken_span_us)
                    .saturating_add(next.frame_offset);
            }

            let encode_time = u32::try_from(now_us.saturating_sub(self.taken_oldest_us) / 1000).unwrap_or(u32::MAX);
            Some(ironrdp::rdpei::pdu::TouchEventPdu::new(encode_time, frames))
        }

        /// Puts back the event the last `take_event` returned, for when it could not be encoded.
        pub(crate) fn restore(&mut self, event: ironrdp::rdpei::pdu::TouchEventPdu) {
            for frame in event.frames.into_iter().rev() {
                self.pending.push_front(frame);
            }
            self.pending_oldest_us = self.taken_oldest_us;
        }

        fn now_us(&self) -> u64 {
            u64::try_from(self.epoch.elapsed().as_micros()).unwrap_or(u64::MAX)
        }
    }

    /// How a pen relates to the digitizer in one toolkit sample.
    pub enum PenPhase {
        /// In range of the digitizer without touching it.
        Hovering,
        /// Touching the digitizer.
        Touching,
        /// Out of range of the digitizer.
        OutOfRange,
        /// The system canceled the pen input.
        Canceled,
    }

    /// Optional attributes of a pen sample ([MS-RDPEI] 2.2.3.7.1.1 "RDPINPUT_PEN_CONTACT").
    ///
    /// Each value travels only when its `has_` flag is set, except the button and inversion flags,
    /// which always travel.
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    pub struct PenAttributes {
        pub barrel_pressed: bool,
        pub eraser_pressed: bool,
        pub inverted: bool,
        pub has_pressure: bool,
        /// Normalized to `0..=1024`.
        pub pressure: u32,
        pub has_rotation: bool,
        /// Clockwise degrees, `0..=359`.
        pub rotation: u16,
        pub has_tilt_x: bool,
        /// Degrees, `-90..=90`.
        pub tilt_x: i16,
        pub has_tilt_y: bool,
        /// Degrees, `-90..=90`.
        pub tilt_y: i16,
    }

    /// Turns pen samples into MS-RDPEI pen frames, one frame per sample.
    ///
    /// Push every toolkit sample, then hand the builder to `ActiveStage::encode_rdpei_pen`, which takes
    /// the oldest pending frames, at most `MAX_FRAMES_PER_EVENT` per call.
    /// `contactFlags` follow the contact lifetime of [MS-RDPEI] 3.1.1.1 "Touch Contact State Transitions".
    /// One pen is tracked at a time with `deviceId` 0, because multipen injection is not requested;
    /// samples of another pen are ignored until the tracked pen leaves the range.
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    #[diplomat::opaque_mut]
    pub struct PenFrameBuilder {
        tracked: Option<super::TrackedPen>,
        /// Frames no event has carried yet, oldest first.
        pending: VecDeque<super::PendingPenFrame>,
        /// Timestamps of the frames of the last event `take_event` returned, for `restore`.
        taken_timestamps_us: Vec<u64>,
        /// Time of the last frame, for `frameOffset`; `None` between transactions.
        last_frame_us: Option<u64>,
        epoch: std::time::Instant,
    }

    impl PenFrameBuilder {
        pub fn new() -> Box<PenFrameBuilder> {
            Box::new(PenFrameBuilder {
                tracked: None,
                pending: VecDeque::new(),
                taken_timestamps_us: Vec::new(),
                last_frame_us: None,
                epoch: std::time::Instant::now(),
            })
        }

        /// Adds one toolkit sample of the pen `external_id`, in desktop coordinates.
        ///
        /// A sample that produces no contact transition is ignored: a pen out of range that stays so,
        /// or a cancel for a pen the server never saw.
        pub fn push(&mut self, external_id: u64, phase: PenPhase, x: i32, y: i32) {
            self.push_sample(external_id, phase, x, y, None);
        }

        /// Like `push`, with the pen buttons, pressure, rotation and tilt of the sample.
        pub fn push_with_attributes(
            &mut self,
            external_id: u64,
            phase: PenPhase,
            x: i32,
            y: i32,
            attributes: PenAttributes,
        ) {
            self.push_sample(external_id, phase, x, y, Some(attributes));
        }

        /// Cancels the tracked pen, for example when the window loses focus.
        ///
        /// The cancellation waits with the other pending frames for the next encode.
        pub fn cancel_all(&mut self) {
            if let Some(pen) = &self.tracked {
                let (external_id, x, y) = (pen.external_id, pen.last.x, pen.last.y);
                self.push_sample(external_id, PenPhase::Canceled, x, y, None);
            }
        }

        /// Forgets the pen and every pending frame without sending anything, for when the channel closed.
        pub fn reset(&mut self) {
            self.tracked = None;
            self.pending.clear();
            self.taken_timestamps_us.clear();
            self.last_frame_us = None;
        }

        /// Whether frames wait for the next encode.
        ///
        /// More than `MAX_FRAMES_PER_EVENT` frames, as after a long suspension, take several encodes.
        #[diplomat::attr(auto, getter = "has_pending_frames")]
        pub fn get_has_pending_frames(&self) -> bool {
            !self.pending.is_empty()
        }

        /// Takes the oldest pending frames, at most `MAX_FRAMES_PER_EVENT`, as one pen event.
        pub(crate) fn take_event(&mut self) -> Option<ironrdp::rdpei::pdu::PenEventPdu> {
            let now_us = self.now_us();
            let count = self.pending.len().min(MAX_FRAMES_PER_EVENT);
            let oldest_us = self.pending.front()?.timestamp_us;
            self.taken_timestamps_us.clear();
            let frames: Vec<ironrdp::rdpei::pdu::PenFrame> = self
                .pending
                .drain(..count)
                .map(|pending| {
                    self.taken_timestamps_us.push(pending.timestamp_us);
                    pending.frame
                })
                .collect();
            let encode_time = u32::try_from(now_us.saturating_sub(oldest_us) / 1000).unwrap_or(u32::MAX);
            Some(ironrdp::rdpei::pdu::PenEventPdu::new(encode_time, frames))
        }

        /// Puts back the event the last `take_event` returned, for when it could not be encoded.
        pub(crate) fn restore(&mut self, event: ironrdp::rdpei::pdu::PenEventPdu) {
            let timestamps = core::mem::take(&mut self.taken_timestamps_us);
            for (timestamp_us, frame) in timestamps.into_iter().zip(event.frames).rev() {
                self.pending.push_front(super::PendingPenFrame { timestamp_us, frame });
            }
        }

        fn push_sample(
            &mut self,
            external_id: u64,
            phase: PenPhase,
            x: i32,
            y: i32,
            attributes: Option<PenAttributes>,
        ) {
            let tracked = match &self.tracked {
                Some(pen) if pen.external_id != external_id => return,
                tracked => tracked.as_ref(),
            };
            let sample = match phase {
                PenPhase::Hovering => ironrdp::rdpei::contact::ContactSample {
                    in_range: true,
                    in_contact: false,
                    canceled: false,
                },
                PenPhase::Touching => ironrdp::rdpei::contact::ContactSample {
                    in_range: true,
                    in_contact: true,
                    canceled: false,
                },
                PenPhase::OutOfRange => ironrdp::rdpei::contact::ContactSample {
                    in_range: false,
                    in_contact: false,
                    canceled: false,
                },
                PenPhase::Canceled => ironrdp::rdpei::contact::ContactSample {
                    in_range: false,
                    in_contact: false,
                    canceled: true,
                },
            };
            let Some(transition) = ironrdp::rdpei::contact::transition(tracked.map(|pen| pen.state), sample) else {
                return;
            };

            // A lift or cancel is sent where the pen last touched; it moves only after the transition.
            let (x, y) = match tracked {
                Some(pen) if transition.keeps_position() => (pen.last.x, pen.last.y),
                _ => (x, y),
            };
            let mut contact = ironrdp::rdpei::pdu::PenContact::new(0, x, y, transition.pen_flags());
            if let Some(attributes) = attributes {
                let mut pen_flags = ironrdp::rdpei::pdu::PenFlags::empty();
                pen_flags.set(ironrdp::rdpei::pdu::PenFlags::BARREL_PRESSED, attributes.barrel_pressed);
                pen_flags.set(ironrdp::rdpei::pdu::PenFlags::ERASER_PRESSED, attributes.eraser_pressed);
                pen_flags.set(ironrdp::rdpei::pdu::PenFlags::INVERTED, attributes.inverted);
                contact = contact.with_pen_flags(pen_flags);
                if attributes.has_pressure {
                    contact = contact.with_pressure(attributes.pressure.min(1024));
                }
                if attributes.has_rotation {
                    contact = contact.with_rotation(attributes.rotation % 360);
                }
                // Digitizers may report one tilt axis only, so each axis has its own field flag.
                if attributes.has_tilt_x {
                    contact
                        .fields_present
                        .insert(ironrdp::rdpei::pdu::PenContactDataFlags::TILTX_PRESENT);
                    contact.fields.tilt_x = Some(attributes.tilt_x.clamp(-90, 90));
                }
                if attributes.has_tilt_y {
                    contact
                        .fields_present
                        .insert(ironrdp::rdpei::pdu::PenContactDataFlags::TILTY_PRESENT);
                    contact.fields.tilt_y = Some(attributes.tilt_y.clamp(-90, 90));
                }
            }

            let now_us = self.now_us();
            let frame_offset = self.last_frame_us.map_or(0, |last| now_us.saturating_sub(last));
            // The transaction ends once the pen is out of range, and the next one starts at offset 0.
            self.last_frame_us = transition.next.map(|_| now_us);
            self.pending.push_back(super::PendingPenFrame {
                timestamp_us: now_us,
                frame: ironrdp::rdpei::pdu::PenFrame::new(frame_offset, vec![contact.clone()]),
            });
            self.tracked = transition.next.map(|state| super::TrackedPen {
                external_id,
                state,
                last: contact,
            });
        }

        fn now_us(&self) -> u64 {
            u64::try_from(self.epoch.elapsed().as_micros()).unwrap_or(u64::MAX)
        }
    }
}
