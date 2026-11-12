/// A pen the server knows about.
struct TrackedPen {
    external_id: u64,
    state: ironrdp::rdpei::contact::ContactState,
    /// Last contact sent for this pen.
    last: ironrdp::rdpei::pdu::PenContact,
}

/// Frames one `RDPINPUT_TOUCH_EVENT_PDU` or `RDPINPUT_PEN_EVENT_PDU` can carry: `frameCount` is a
/// TWO_BYTE_UNSIGNED_INTEGER ([MS-RDPEI] 2.2.3.3 "RDPINPUT_TOUCH_EVENT_PDU", 2.2.3.7
/// "RDPINPUT_PEN_EVENT_PDU", 2.2.2.1 "TWO_BYTE_UNSIGNED_INTEGER").
///
/// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
const MAX_FRAMES_PER_EVENT: usize = 0x7FFF;

/// Frames no event has carried yet, oldest first, each with the builder-clock time it was built at.
///
/// Keeping the absolute time of every frame, rather than deriving it from `frameOffset`, keeps each
/// event's `encodeTime` exact across batches: `frameOffset` is 0 at the start of a transaction, so it
/// does not carry the idle time between two gestures.
struct PendingFrames<F> {
    frames: std::collections::VecDeque<(u64, F)>,
    /// Times of the frames of the last `take`, for `restore`.
    taken_us: Vec<u64>,
}

impl<F> PendingFrames<F> {
    fn new() -> Self {
        Self {
            frames: std::collections::VecDeque::new(),
            taken_us: Vec::new(),
        }
    }

    fn push(&mut self, timestamp_us: u64, frame: F) {
        self.frames.push_back((timestamp_us, frame));
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.taken_us.clear();
    }

    /// Takes the oldest frames, at most `MAX_FRAMES_PER_EVENT`, with the `encodeTime` of the oldest.
    fn take(&mut self, now_us: u64) -> Option<(u32, Vec<F>)> {
        let oldest_us = self.frames.front()?.0;
        let count = self.frames.len().min(MAX_FRAMES_PER_EVENT);
        self.taken_us.clear();
        let frames = self
            .frames
            .drain(..count)
            .map(|(timestamp_us, frame)| {
                self.taken_us.push(timestamp_us);
                frame
            })
            .collect();
        let encode_time = u32::try_from(now_us.saturating_sub(oldest_us) / 1000).unwrap_or(u32::MAX);
        Some((encode_time, frames))
    }

    /// Puts back the frames the last `take` returned, for when they could not be encoded.
    fn restore(&mut self, frames: Vec<F>) {
        let taken_us = core::mem::take(&mut self.taken_us);
        for pending in taken_us.into_iter().zip(frames).rev() {
            self.frames.push_front(pending);
        }
    }
}

#[diplomat::bridge]
pub mod ffi {
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
    /// takes the oldest pending frames, at most 32767 per call.
    /// Contact ids and `contactFlags` are derived here, so every frame stays within the legal
    /// combinations of [MS-RDPEI] 2.2.3.3.1.1 "RDPINPUT_CONTACT_DATA".
    /// Frame timing comes from a monotonic clock read at each push.
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct TouchFrameBuilder {
        /// Tracks the contacts and derives each frame; its frames move to `pending` after every push,
        /// so each frame carries the time of the push that built it.
        inner: ironrdp::rdpei::contact::TouchFrameBuilder,
        pending: super::PendingFrames<ironrdp::rdpei::pdu::TouchFrame>,
        epoch: std::time::Instant,
    }

    impl TouchFrameBuilder {
        /// Tracks at most `max_contacts` contacts, the `max_touch_contacts` given to
        /// `ClientConnector::with_dynamic_channel_rdpei`.
        pub fn new(max_contacts: u16) -> Box<TouchFrameBuilder> {
            Box::new(TouchFrameBuilder {
                inner: ironrdp::rdpei::contact::TouchFrameBuilder::new(max_contacts),
                pending: super::PendingFrames::new(),
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
            self.collect_frames(now_us);
        }

        /// Like `push`, with the contact pressure normalized to `0..=1024`.
        pub fn push_with_pressure(&mut self, external_id: u64, phase: TouchPhase, x: i32, y: i32, pressure: u32) {
            let now_us = self.now_us();
            self.inner.push(external_id, phase.into(), x, y, Some(pressure), now_us);
            self.collect_frames(now_us);
        }

        /// Cancels every tracked contact, for example when the window loses focus.
        ///
        /// The cancellations wait with the other pending frames for the next encode.
        pub fn cancel_all(&mut self) {
            let now_us = self.now_us();
            self.inner.cancel_all(now_us);
            self.collect_frames(now_us);
        }

        /// Forgets every contact and pending frame without sending anything, for when the channel closed.
        pub fn reset(&mut self) {
            self.inner.reset();
            self.pending.clear();
        }

        /// Whether frames wait for the next encode.
        ///
        /// More than 32767 frames, as after a long suspension, take several encodes.
        #[diplomat::attr(auto, getter = "has_pending_frames")]
        pub fn get_has_pending_frames(&self) -> bool {
            !self.pending.is_empty()
        }

        /// Takes the oldest pending frames, at most 32767, as one touch event.
        pub(crate) fn take_event(&mut self) -> Option<ironrdp::rdpei::pdu::TouchEventPdu> {
            let (encode_time, frames) = self.pending.take(self.now_us())?;
            Some(ironrdp::rdpei::pdu::TouchEventPdu::new(encode_time, frames))
        }

        /// Puts back the event the last `take_event` returned, for when it could not be encoded.
        pub(crate) fn restore(&mut self, event: ironrdp::rdpei::pdu::TouchEventPdu) {
            self.pending.restore(event.frames);
        }

        /// Moves the frames the pushes at `now_us` built into `pending`, stamped with that time.
        ///
        /// Taking after every push leaves at most the frame of that push in `inner`, so the time of
        /// each frame is known exactly.
        fn collect_frames(&mut self, now_us: u64) {
            if let Some(event) = self.inner.take_event(now_us) {
                for frame in event.frames {
                    self.pending.push(now_us, frame);
                }
            }
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
    /// the oldest pending frames, at most 32767 per call.
    /// `contactFlags` follow the contact lifetime of [MS-RDPEI] 3.1.1.1 "Touch Contact State Transitions".
    /// One pen is tracked at a time with `deviceId` 0, because multipen injection is not requested;
    /// samples of another pen are ignored until the tracked pen leaves the range.
    ///
    /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct PenFrameBuilder {
        tracked: Option<super::TrackedPen>,
        pending: super::PendingFrames<ironrdp::rdpei::pdu::PenFrame>,
        /// Time of the last frame, for `frameOffset`; `None` between transactions.
        last_frame_us: Option<u64>,
        epoch: std::time::Instant,
    }

    impl PenFrameBuilder {
        pub fn new() -> Box<PenFrameBuilder> {
            Box::new(PenFrameBuilder {
                tracked: None,
                pending: super::PendingFrames::new(),
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
            self.last_frame_us = None;
        }

        /// Whether frames wait for the next encode.
        ///
        /// More than 32767 frames, as after a long suspension, take several encodes.
        #[diplomat::attr(auto, getter = "has_pending_frames")]
        pub fn get_has_pending_frames(&self) -> bool {
            !self.pending.is_empty()
        }

        /// Takes the oldest pending frames, at most 32767, as one pen event.
        pub(crate) fn take_event(&mut self) -> Option<ironrdp::rdpei::pdu::PenEventPdu> {
            let (encode_time, frames) = self.pending.take(self.now_us())?;
            Some(ironrdp::rdpei::pdu::PenEventPdu::new(encode_time, frames))
        }

        /// Puts back the event the last `take_event` returned, for when it could not be encoded.
        pub(crate) fn restore(&mut self, event: ironrdp::rdpei::pdu::PenEventPdu) {
            self.pending.restore(event.frames);
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
            self.pending.push(
                now_us,
                ironrdp::rdpei::pdu::PenFrame::new(frame_offset, vec![contact.clone()]),
            );
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
