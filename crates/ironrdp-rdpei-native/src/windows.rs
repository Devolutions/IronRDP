use core::time::Duration;
use core::{
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ironrdp_rdpei::pdu::{
    CsReadyFlags, CsReadyPdu, PenContact, PenContactFlags, PenEventPdu, PenFlags, RdpInputProtocolVersion,
    TouchContactFlags, TouchEventPdu,
};
use windows::Win32::Foundation::{ERROR_NOT_READY, POINT, RECT};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, HSYNTHETICPOINTERDEVICE, POINTER_FEEDBACK_NONE,
    POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::Pointer::{
    InitializeTouchInjection, InjectSyntheticPointerInput, InjectTouchInput, POINTER_FLAG_CANCELED, POINTER_FLAG_DOWN,
    POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_FLAGS, POINTER_INFO,
    POINTER_PEN_INFO, POINTER_TOUCH_INFO, TOUCH_FEEDBACK_DEFAULT, TOUCH_FEEDBACK_NONE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MAX_TOUCH_COUNT, PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PEN_MASK_ROTATION,
    PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, PT_TOUCH, TOUCH_MASK_CONTACTAREA, TOUCH_MASK_ORIENTATION,
    TOUCH_MASK_PRESSURE,
};

use crate::error::{self, WindowsInputResult};

macro_rules! ensure_input {
    ($condition:expr, $context:literal) => {
        if !($condition) {
            return Err(error::invalid_input($context));
        }
    };
}

macro_rules! ensure_state {
    ($condition:expr, $context:literal) => {
        if !($condition) {
            return Err(error::invalid_state($context));
        }
    };
}

trait ErrorContext<T> {
    fn context(self, context: &'static str) -> WindowsInputResult<T>;
}

impl<T, E> ErrorContext<T> for Result<T, E>
where
    E: ironrdp_error::Source,
{
    fn context(self, context: &'static str) -> WindowsInputResult<T> {
        self.map_err(|source| error::native_error(context, source))
    }
}

struct Device(HSYNTHETICPOINTERDEVICE);

static INPUT_OWNED: AtomicBool = AtomicBool::new(false);

struct Ownership(PhantomData<Rc<()>>);

impl Drop for Ownership {
    fn drop(&mut self) {
        INPUT_OWNED.store(false, Ordering::Release);
    }
}

struct Pen {
    device: Device,
    previous: POINTER_PEN_INFO,
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: this process owns the device, and injection finishes before it is dropped.
        unsafe { DestroySyntheticPointerDevice(self.0) };
    }
}

/// Translates [RDPINPUT_TOUCH_EVENT_PDU] and [RDPINPUT_PEN_EVENT_PDU] into native pointer injection.
///
/// Only one instance may exist per process.
/// Keep it on its creation thread and await each operation before starting the next.
/// Reset after an injection error or cancellation of an asynchronous operation.
///
/// [RDPINPUT_TOUCH_EVENT_PDU]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.3>
/// [RDPINPUT_PEN_EVENT_PDU]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.7>
pub struct WindowsInput {
    ready: Option<CsReadyPdu>,
    touch_initialized: bool,
    frequency: u64,
    last_touch_timestamp: u64,
    contacts: HashMap<u8, POINTER_TOUCH_INFO>,
    pens: HashMap<u8, Pen>,
    /// INVARIANT: this field is dropped after native contact and device cleanup.
    ///
    /// Touch injection is process-global, and the `Rc` marker keeps all operations on one thread.
    _ownership: Ownership,
}

impl Drop for WindowsInput {
    fn drop(&mut self) {
        self.reset();
    }
}

impl WindowsInput {
    /// Reserves this process's touch injector without generating input.
    pub fn new() -> WindowsInputResult<Self> {
        ensure_state!(
            INPUT_OWNED
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
            "a Windows pointer injector already exists"
        );
        Ok(Self {
            ready: None,
            touch_initialized: false,
            frequency: 0,
            last_touch_timestamp: 0,
            contacts: HashMap::new(),
            pens: HashMap::new(),
            _ownership: Ownership(PhantomData),
        })
    }

    /// Cancels previous input and configures injection from [RDPINPUT_CS_READY_PDU] capabilities.
    ///
    /// Failure leaves the injector uninitialized.
    ///
    /// [RDPINPUT_CS_READY_PDU]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.2>
    pub fn initialize(&mut self, ready: CsReadyPdu) -> WindowsInputResult<()> {
        self.reset();
        let mut frequency = 0;
        // SAFETY: the output points to a writable counter-frequency value.
        unsafe { QueryPerformanceFrequency(&mut frequency) }.context("query pointer clock frequency")?;
        self.frequency = u64::try_from(frequency)
            .map_err(|source| error::clock_error_with_source("invalid pointer clock frequency", source))?;
        ensure_state!(self.frequency != 0, "zero pointer clock frequency");
        let touch_enabled = ready.max_touch_contacts != 0;
        if touch_enabled {
            self.initialize_touch(ready.flags.contains(CsReadyFlags::SHOW_TOUCH_VISUALS))?;
        }
        self.ready = Some(ready);
        Ok(())
    }

    /// Cancels active touch contacts, releases pens, and clears negotiation.
    ///
    /// Native cancellation is best-effort and failures are logged.
    /// This can block for up to 100 ms while waiting for the touch injection clock.
    pub fn reset(&mut self) {
        if !self.contacts.is_empty() {
            let mut contacts: Vec<_> = self
                .contacts
                .values()
                .map(|previous| {
                    let mut contact = *previous;
                    contact.pointerInfo.pointerFlags =
                        cancellation_flags(previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT));
                    contact
                })
                .collect();
            let deadline = std::time::Instant::now() + Duration::from_millis(100);
            loop {
                if self.last_touch_timestamp != 0 {
                    let timestamp = match performance_counter() {
                        Ok(timestamp) => timestamp,
                        Err(error) => {
                            tracing::warn!(%error, "Failed to timestamp touch cancellation");
                            break;
                        }
                    };
                    if timestamp.saturating_sub(self.last_touch_timestamp) < minimum_interval(self.frequency) {
                        if deadline <= std::time::Instant::now() {
                            tracing::warn!("Touch cancellation clock did not advance");
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    for contact in &mut contacts {
                        contact.pointerInfo.PerformanceCount = timestamp;
                    }
                }
                // UNTESTED: active contacts must be canceled before releasing ownership.
                // Verify engaged and hovering cancellation on an interactive desktop when changing cleanup.
                // SAFETY: these are this thread's initialized, active touch contacts.
                match unsafe { InjectTouchInput(&contacts) } {
                    Ok(()) => break,
                    Err(error)
                        if error.code() == ERROR_NOT_READY.to_hresult() && std::time::Instant::now() < deadline =>
                    {
                        // Drop cannot await the next injection clock interval.
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Failed to cancel touch contacts");
                        break;
                    }
                }
            }
        }
        self.contacts.clear();
        self.last_touch_timestamp = 0;
        self.pens.clear();
        self.ready = None;
        self.touch_initialized = false;
    }

    fn initialize_touch(&mut self, show_visuals: bool) -> WindowsInputResult<()> {
        if !self.touch_initialized {
            let feedback = if show_visuals {
                TOUCH_FEEDBACK_DEFAULT
            } else {
                TOUCH_FEEDBACK_NONE
            };
            // SAFETY: the contact count and feedback mode are documented input values.
            unsafe { InitializeTouchInjection(MAX_TOUCH_COUNT, feedback) }.context("initialize touch injection")?;
            self.touch_initialized = true;
        }
        Ok(())
    }

    /// Injects touch frames in order, preserving omitted active contacts.
    ///
    /// Contact flags and frame timing follow [RDPINPUT_TOUCH_EVENT_PDU].
    /// Orientation follows [RDPINPUT_TOUCH_CONTACT].
    ///
    /// [RDPINPUT_TOUCH_EVENT_PDU]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.3>
    /// [RDPINPUT_TOUCH_CONTACT]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.3.1.1>
    pub async fn touch(&mut self, event: TouchEventPdu) -> WindowsInputResult<()> {
        let ready = self
            .ready
            .as_ref()
            .ok_or_else(|| error::invalid_state("RDPEI client is not ready"))?;
        ensure_input!(!event.frames.is_empty(), "empty touch event");
        let native_max_contacts = usize::try_from(MAX_TOUCH_COUNT)
            .map_err(|source| error::invalid_state("invalid maximum touch contact count").with_source(source))?;
        let max_contacts = usize::from(ready.max_touch_contacts).min(native_max_contacts);
        let timestamps = frame_timestamps(
            self.frequency,
            performance_counter()?,
            self.last_touch_timestamp,
            !self.contacts.is_empty(),
            ready.flags,
            event.encode_time,
            &mut event.frames.iter().map(|frame| frame.frame_offset),
        )?;
        for (frame, timestamp) in event.frames.into_iter().zip(timestamps) {
            ensure_input!(
                !frame.contacts.is_empty() && frame.contacts.len() <= max_contacts,
                "invalid touch frame size"
            );
            let mut contacts = HashMap::with_capacity(frame.contacts.len());
            for contact in frame.contacts {
                ensure_input!(contact.contact_flags.is_legal(), "invalid touch contact flags");
                let orientation = validated_touch_orientation(contact.fields.orientation)?;
                let mut native = POINTER_TOUCH_INFO {
                    pointerInfo: POINTER_INFO {
                        pointerType: PT_TOUCH,
                        pointerId: u32::from(contact.contact_id),
                        pointerFlags: pointer_flags(contact.contact_flags),
                        ptPixelLocation: POINT {
                            x: contact.x,
                            y: contact.y,
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                };
                if contact.contact_flags.contains(TouchContactFlags::CANCELED) {
                    let engaged = self
                        .contacts
                        .get(&contact.contact_id)
                        .is_some_and(|previous| previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT));
                    native.pointerInfo.pointerFlags = cancellation_flags(engaged);
                }
                if let (Some(left), Some(top), Some(right), Some(bottom)) = (
                    contact.fields.contact_rect_left,
                    contact.fields.contact_rect_top,
                    contact.fields.contact_rect_right,
                    contact.fields.contact_rect_bottom,
                ) {
                    native.touchMask |= TOUCH_MASK_CONTACTAREA;
                    native.rcContact = touch_contact_rect_for_orientation(
                        contact.x,
                        contact.y,
                        left,
                        top,
                        right,
                        bottom,
                        orientation,
                    )?;
                }
                native.touchMask |= TOUCH_MASK_ORIENTATION;
                native.orientation = windows_touch_orientation(orientation);
                if let Some(pressure) = contact.fields.pressure {
                    ensure_input!(pressure <= 1024, "invalid touch pressure");
                    native.touchMask |= TOUCH_MASK_PRESSURE;
                    native.pressure = pressure;
                }
                ensure_input!(
                    contacts.insert(contact.contact_id, native).is_none(),
                    "duplicate touch contact ID"
                );
            }
            let moved_releases = contacts
                .iter()
                .filter_map(|(&id, contact)| {
                    let previous = self.contacts.get(&id)?;
                    (contact.pointerInfo.pointerFlags.contains(POINTER_FLAG_UP)
                        && previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT)
                        && (contact.pointerInfo.ptPixelLocation.x != previous.pointerInfo.ptPixelLocation.x
                            || contact.pointerInfo.ptPixelLocation.y != previous.pointerInfo.ptPixelLocation.y))
                        .then(|| {
                            let mut update = *contact;
                            update.pointerInfo.pointerFlags =
                                POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
                            (id, update)
                        })
                })
                .collect::<HashMap<_, _>>();
            if !moved_releases.is_empty() {
                // Windows requires UP to reuse the preceding position or it cancels every contact.
                self.inject_contacts(
                    moved_releases,
                    max_contacts,
                    timestamp.saturating_sub(minimum_interval(self.frequency)),
                )
                .await?;
            }
            self.inject_contacts(contacts, max_contacts, timestamp).await?;
        }
        Ok(())
    }

    /// Takes a hovering contact out of range; engaged or unknown contacts are unchanged.
    ///
    /// [RDPINPUT_DISMISS_HOVERING_TOUCH_CONTACT_PDU] defines this event.
    ///
    /// [RDPINPUT_DISMISS_HOVERING_TOUCH_CONTACT_PDU]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.6>
    pub async fn dismiss(&mut self, contact_id: u8) -> WindowsInputResult<()> {
        let ready = self
            .ready
            .as_ref()
            .ok_or_else(|| error::invalid_state("RDPEI client is not ready"))?;
        let native_max_contacts = usize::try_from(MAX_TOUCH_COUNT)
            .map_err(|source| error::invalid_state("invalid maximum touch contact count").with_source(source))?;
        let max_contacts = usize::from(ready.max_touch_contacts).min(native_max_contacts);
        let Some(mut contact) = self.contacts.get(&contact_id).copied() else {
            return Ok(());
        };
        if contact.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT) {
            return Ok(());
        }
        contact.pointerInfo.pointerFlags = POINTER_FLAG_UPDATE;
        let timestamp = if self.last_touch_timestamp == 0 {
            0
        } else {
            performance_counter()?
        };
        self.inject_contacts(HashMap::from([(contact_id, contact)]), max_contacts, timestamp)
            .await
    }

    /// Injects pen frames, owning at most four synthetic pen devices.
    ///
    /// Pen contact state follows [RDPINPUT_PEN_EVENT_PDU] and [RDPINPUT_PEN_CONTACT].
    ///
    /// [RDPINPUT_PEN_EVENT_PDU]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.7>
    /// [RDPINPUT_PEN_CONTACT]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/72a8cb65-7f6c-407c-a21a-3d970721fed0#section_2.2.3.7.1.1>
    pub async fn pen(&mut self, event: PenEventPdu) -> WindowsInputResult<()> {
        let ready = self
            .ready
            .as_ref()
            .ok_or_else(|| error::invalid_state("RDPEI client is not ready"))?;
        ensure_input!(!event.frames.is_empty(), "empty pen event");
        ensure_state!(
            ready.protocol_version.supports_pen(),
            "RDPEI pen input was not negotiated"
        );
        let multipen = ready.protocol_version == RdpInputProtocolVersion::V300
            && ready.flags.contains(CsReadyFlags::ENABLE_MULTIPEN_INJECTION);
        let minimum_interval = minimum_interval(self.frequency);
        let continuing = self
            .pens
            .values()
            .any(|pen| pen.previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INRANGE));
        let timestamps = {
            let previous = self
                .pens
                .values()
                .map(|pen| pen.previous.pointerInfo.PerformanceCount)
                .max()
                .unwrap_or(0);
            let previous = if previous == 0 {
                0
            } else {
                previous
                    .checked_add(2 * minimum_interval)
                    .ok_or_else(|| error::clock_error("pen timestamp overflow"))?
            };
            frame_timestamps(
                self.frequency,
                performance_counter()?,
                previous,
                continuing,
                ready.flags,
                event.encode_time,
                &mut event.frames.iter().map(|frame| frame.frame_offset),
            )?
        };
        for (mut frame, timestamp) in event.frames.into_iter().zip(timestamps) {
            ensure_input!(
                !frame.contacts.is_empty() && frame.contacts.len() <= if multipen { 4 } else { 1 },
                "invalid pen frame size"
            );
            let mut devices = HashSet::with_capacity(frame.contacts.len());
            prioritize_pen_departures(&mut frame.contacts);
            for contact in frame.contacts {
                ensure_state!(
                    multipen || contact.device_id == 0,
                    "multiple pen devices were not negotiated"
                );
                ensure_input!(devices.insert(contact.device_id), "duplicate pen device ID");
                ensure_input!(contact.contact_flags.is_legal(), "invalid pen contact flags");
                if contact.contact_flags.contains(PenContactFlags::CANCELED) {
                    if self.pens.remove(&contact.device_id).is_none() {
                        tracing::trace!(
                            device_id = contact.device_id,
                            "Ignoring cancellation for an inactive pen"
                        );
                    }
                    continue;
                }
                let mut native = POINTER_PEN_INFO {
                    pointerInfo: POINTER_INFO {
                        pointerType: PT_PEN,
                        // Keep pointer identities unique across concurrent synthetic devices.
                        pointerId: u32::from(contact.device_id) + 1,
                        pointerFlags: pointer_flags(TouchContactFlags::from_bits_retain(contact.contact_flags.bits())),
                        ptPixelLocation: POINT {
                            x: contact.x,
                            y: contact.y,
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                };
                if let Some(flags) = contact.fields.pen_flags {
                    set_pen_flags(&mut native, flags);
                }
                if let Some(pressure) = contact.fields.pressure {
                    ensure_input!(pressure <= 1024, "invalid pen pressure");
                    native.penMask |= PEN_MASK_PRESSURE;
                    native.pressure = pressure;
                }
                if let Some(rotation) = contact.fields.rotation {
                    ensure_input!(rotation < 360, "invalid pen rotation");
                    native.penMask |= PEN_MASK_ROTATION;
                    native.rotation = u32::from(rotation);
                }
                if let Some(tilt) = contact.fields.tilt_x {
                    ensure_input!((-90..=90).contains(&tilt), "invalid pen x tilt");
                    native.penMask |= PEN_MASK_TILT_X;
                    native.tiltX = i32::from(tilt);
                }
                if let Some(tilt) = contact.fields.tilt_y {
                    ensure_input!((-90..=90).contains(&tilt), "invalid pen y tilt");
                    native.penMask |= PEN_MASK_TILT_Y;
                    native.tiltY = i32::from(tilt);
                }
                if !self.pens.contains_key(&contact.device_id) && self.pens.len() == 4 {
                    let unused = self
                        .pens
                        .iter()
                        .find_map(|(&id, pen)| {
                            (!pen.previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INRANGE)).then_some(id)
                        })
                        .ok_or_else(|| error::invalid_state("more than four pen devices are active"))?;
                    self.pens.remove(&unused);
                }
                let pen = match self.pens.entry(contact.device_id) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        // SAFETY: a synthetic pen supports exactly one contact; this process owns the handle.
                        let handle = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_NONE) }
                            .context("create pen device")?;
                        entry.insert(Pen {
                            device: Device(handle),
                            previous: POINTER_PEN_INFO::default(),
                        })
                    }
                };
                if native.pointerInfo.pointerFlags.contains(POINTER_FLAG_UP) {
                    native.penFlags &= !PEN_FLAG_INVERTED;
                    native.penFlags |= pen.previous.penFlags & PEN_FLAG_INVERTED;
                    if !pen.previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT) {
                        native.pointerInfo.pointerFlags &= !POINTER_FLAG_UP;
                        native.pointerInfo.pointerFlags |= POINTER_FLAG_UPDATE;
                    }
                }
                if native.pointerInfo.pointerFlags.contains(POINTER_FLAG_DOWN)
                    && !pen.previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT)
                {
                    if pen.previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INRANGE)
                        && (pen.previous.penFlags ^ native.penFlags) & PEN_FLAG_INVERTED != 0
                    {
                        // Windows keeps the tip selection until the pointer leaves range.
                        let mut exit = pen.previous;
                        exit.pointerInfo.pointerFlags = POINTER_FLAG_UPDATE;
                        inject_pen(
                            pen,
                            self.frequency,
                            timestamp.saturating_sub(2 * minimum_interval),
                            exit,
                        )
                        .await?;
                    }
                    // Windows establishes inversion during hover, not on the first DOWN sample.
                    let mut hover = native;
                    hover.pointerInfo.pointerFlags = POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE;
                    inject_pen(pen, self.frequency, timestamp.saturating_sub(minimum_interval), hover).await?;
                }
                inject_pen(pen, self.frequency, timestamp, native).await?;
            }
        }
        Ok(())
    }

    async fn inject_contacts(
        &mut self,
        mut contacts: HashMap<u8, POINTER_TOUCH_INFO>,
        max_contacts: usize,
        mut timestamp: u64,
    ) -> WindowsInputResult<()> {
        for (&id, previous) in &self.contacts {
            contacts.entry(id).or_insert_with(|| {
                let mut current = *previous;
                current.pointerInfo.pointerFlags &= POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
                current.pointerInfo.pointerFlags |= POINTER_FLAG_UPDATE;
                current
            });
        }
        ensure_input!(contacts.len() <= max_contacts, "too many active touch contacts");
        let mut native: Vec<_> = contacts.values().copied().collect();
        let minimum_interval = minimum_interval(self.frequency);
        if timestamp != 0 && self.last_touch_timestamp != 0 {
            timestamp = timestamp.max(
                self.last_touch_timestamp
                    .checked_add(minimum_interval)
                    .ok_or_else(|| error::clock_error("touch timestamp overflow"))?,
            );
        }
        let show_visuals = self
            .ready
            .as_ref()
            .ok_or_else(|| error::invalid_state("RDPEI client is not ready"))?
            .flags
            .contains(CsReadyFlags::SHOW_TOUCH_VISUALS);
        self.initialize_touch(show_visuals)?;
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        loop {
            if timestamp != 0 {
                if performance_counter()? < timestamp {
                    ensure_input!(
                        std::time::Instant::now() < deadline,
                        "touch timestamp is too far in the future"
                    );
                    futures_timer::Delay::new(Duration::from_millis(1)).await;
                    continue;
                }
                for contact in &mut native {
                    contact.pointerInfo.PerformanceCount = timestamp;
                }
            }
            // UNTESTED: Windows must retain unrelated contacts and honor individual cancellation and moved UP samples.
            // Check simultaneous contacts, moved releases, and cancellation on an interactive desktop when changing this.
            // InjectSyntheticPointerInput drops CANCELED; InjectTouchInput preserves individual cancellation.
            // SAFETY: the initialized device accepts these bounded IDs and fully initialized touch records.
            match unsafe { InjectTouchInput(&native) } {
                Ok(()) => break,
                Err(error) if error.code() == ERROR_NOT_READY.to_hresult() && std::time::Instant::now() < deadline => {
                    if timestamp != 0 {
                        timestamp = timestamp
                            .checked_add(minimum_interval)
                            .ok_or_else(|| error::clock_error("touch timestamp overflow"))?;
                    }
                    futures_timer::Delay::new(Duration::from_millis(1)).await;
                }
                Err(error) => return Err(error::native_error("inject touch frame", error)),
            }
        }
        self.last_touch_timestamp = timestamp;
        for contact in contacts.values_mut() {
            contact.pointerInfo.PerformanceCount = timestamp;
        }
        self.contacts = contacts
            .into_iter()
            .filter(|(_, touch)| touch.pointerInfo.pointerFlags.contains(POINTER_FLAG_INRANGE))
            .collect();
        Ok(())
    }
}

fn performance_counter() -> WindowsInputResult<u64> {
    let mut counter = 0;
    // SAFETY: the output points to a writable counter value.
    unsafe { QueryPerformanceCounter(&mut counter) }.context("query pointer clock")?;
    u64::try_from(counter).map_err(|source| error::clock_error_with_source("invalid pointer clock value", source))
}

fn counter_ticks(frequency: u64, microseconds: u64) -> WindowsInputResult<u64> {
    u64::try_from(u128::from(frequency) * u128::from(microseconds) / 1_000_000)
        .map_err(|source| error::clock_error_with_source("pointer timestamp exceeds the native clock range", source))
}

fn minimum_interval(frequency: u64) -> u64 {
    frequency.div_ceil(10_000)
}

pub(super) fn frame_timestamps(
    frequency: u64,
    now: u64,
    previous: u64,
    continuing: bool,
    flags: CsReadyFlags,
    encode_time: u32,
    offsets: &mut dyn Iterator<Item = u64>,
) -> WindowsInputResult<Vec<u64>> {
    if flags.contains(CsReadyFlags::DISABLE_TIMESTAMP_INJECTION) {
        return Ok(offsets.map(|_| 0).collect());
    }
    let mut timestamps = Vec::new();
    let mut first_offset = 0;
    let mut span = 0u64;
    let minimum_interval = minimum_interval(frequency);
    for offset in offsets {
        if timestamps.is_empty() {
            first_offset = offset;
            ensure_input!(continuing || offset == 0, "first pointer frame offset is not zero");
            timestamps.push(0);
        } else {
            span = span
                .checked_add(offset)
                .ok_or_else(|| error::clock_error("pointer frame offset overflow"))?;
            timestamps.push(span);
        }
    }
    ensure_input!(!timestamps.is_empty(), "empty pointer event");
    // Rounded encoding ages and synthetic taps can be shorter than their frame span.
    let age = (u64::from(encode_time) * 1000).max(span);
    let mut base = now
        .checked_sub(counter_ticks(frequency, age)?)
        .ok_or_else(|| error::clock_error("pointer timestamp predates the native clock"))?;
    if previous != 0 {
        base = base.max(
            previous
                .checked_add(minimum_interval)
                .ok_or_else(|| error::clock_error("pointer timestamp overflow"))?,
        );
        base = base.max(
            previous
                .checked_add(counter_ticks(frequency, first_offset)?)
                .ok_or_else(|| error::clock_error("pointer timestamp overflow"))?,
        );
    }
    ensure_input!(
        2 * minimum_interval < base,
        "pointer timestamp leaves no clock range for native hover samples"
    );
    let latest = now
        .checked_add(frequency / 10)
        .ok_or_else(|| error::clock_error("pointer clock overflow"))?;
    for timestamp in &mut timestamps {
        *timestamp = base
            .checked_add(counter_ticks(frequency, *timestamp)?)
            .ok_or_else(|| error::clock_error("pointer timestamp overflow"))?;
        ensure_input!(*timestamp <= latest, "pointer timestamp is too far in the future");
    }
    Ok(timestamps)
}

pub(super) fn touch_orientation(orientation: Option<u32>) -> WindowsInputResult<u32> {
    Ok(windows_touch_orientation(validated_touch_orientation(orientation)?))
}

fn validated_touch_orientation(orientation: Option<u32>) -> WindowsInputResult<u32> {
    let orientation = orientation.unwrap_or(0);
    ensure_input!(orientation < 360, "invalid touch orientation");
    Ok(orientation)
}

fn windows_touch_orientation(orientation: u32) -> u32 {
    // RDPEI measures counter-clockwise from up; Windows measures clockwise from right.
    (630 - orientation) % 360
}

pub(super) fn touch_contact_rect(
    x: i32,
    y: i32,
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
    orientation: Option<u32>,
) -> WindowsInputResult<RECT> {
    let orientation = validated_touch_orientation(orientation)?;
    touch_contact_rect_for_orientation(x, y, left, top, right, bottom, orientation)
}

fn touch_contact_rect_for_orientation(
    x: i32,
    y: i32,
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
    orientation: u32,
) -> WindowsInputResult<RECT> {
    ensure_input!(left <= right && top <= bottom, "invalid touch contact rectangle");
    let angle = f64::from(orientation).to_radians();
    let sin = angle.sin();
    let cos = angle.cos();
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
        let x = f64::from(x);
        let y = f64::from(y);
        let rotated_x = x * cos + y * sin;
        let rotated_y = -x * sin + y * cos;
        min_x = min_x.min(rotated_x);
        min_y = min_y.min(rotated_y);
        max_x = max_x.max(rotated_x);
        max_y = max_y.max(rotated_y);
    }
    // Trigonometric rounding around exact integer bounds must not add an extra pixel.
    const ROUNDING_EPSILON: f64 = 1e-9;
    let left = rounded_coordinate((min_x + ROUNDING_EPSILON).floor())?;
    let top = rounded_coordinate((min_y + ROUNDING_EPSILON).floor())?;
    let right = rounded_coordinate((max_x - ROUNDING_EPSILON).ceil())?;
    let bottom = rounded_coordinate((max_y - ROUNDING_EPSILON).ceil())?;
    Ok(RECT {
        left: x
            .checked_add(left)
            .ok_or_else(|| error::invalid_input("touch rectangle exceeds native coordinates"))?,
        top: y
            .checked_add(top)
            .ok_or_else(|| error::invalid_input("touch rectangle exceeds native coordinates"))?,
        right: x
            .checked_add(right)
            .ok_or_else(|| error::invalid_input("touch rectangle exceeds native coordinates"))?,
        bottom: y
            .checked_add(bottom)
            .ok_or_else(|| error::invalid_input("touch rectangle exceeds native coordinates"))?,
    })
}

fn rounded_coordinate(value: f64) -> WindowsInputResult<i32> {
    ensure_input!(
        value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX),
        "rotated touch rectangle exceeds the native coordinate range"
    );
    #[expect(
        clippy::as_conversions,
        reason = "the rounded coordinate was checked against the i32 range"
    )]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the rounded coordinate was checked against the i32 range"
    )]
    Ok(value as i32)
}

pub(super) fn prioritize_pen_departures(contacts: &mut [PenContact]) {
    contacts.sort_by_key(|contact| {
        let flags = contact.contact_flags;
        !(flags.contains(PenContactFlags::CANCELED)
            || flags.contains(PenContactFlags::UP) && !flags.contains(PenContactFlags::INRANGE))
    });
}

pub(super) fn set_pen_flags(native: &mut POINTER_PEN_INFO, flags: PenFlags) {
    for (rdpei, windows) in [
        (PenFlags::BARREL_PRESSED, PEN_FLAG_BARREL),
        (PenFlags::ERASER_PRESSED, PEN_FLAG_ERASER),
        (PenFlags::INVERTED, PEN_FLAG_INVERTED),
    ] {
        if flags.contains(rdpei) {
            native.penFlags |= windows;
        }
    }
}

pub(super) fn pointer_flags(flags: TouchContactFlags) -> POINTER_FLAGS {
    let mut native = POINTER_FLAGS::default();
    for (rdpei, windows) in [
        (TouchContactFlags::DOWN, POINTER_FLAG_DOWN),
        (TouchContactFlags::UPDATE, POINTER_FLAG_UPDATE),
        (TouchContactFlags::UP, POINTER_FLAG_UP),
        (TouchContactFlags::INRANGE, POINTER_FLAG_INRANGE),
        (TouchContactFlags::INCONTACT, POINTER_FLAG_INCONTACT),
        (TouchContactFlags::CANCELED, POINTER_FLAG_CANCELED),
    ] {
        if flags.contains(rdpei) {
            native |= windows;
        }
    }
    native
}

fn cancellation_flags(engaged: bool) -> POINTER_FLAGS {
    // UNTESTED: cancellation uses UP for engaged contacts and UPDATE for hovering contacts.
    // Verify both states on an interactive desktop when changing cancellation handling.
    POINTER_FLAG_CANCELED | if engaged { POINTER_FLAG_UP } else { POINTER_FLAG_UPDATE }
}

async fn inject_pen(
    pen: &mut Pen,
    frequency: u64,
    mut timestamp: u64,
    mut native: POINTER_PEN_INFO,
) -> WindowsInputResult<()> {
    let minimum_interval = minimum_interval(frequency);
    if timestamp != 0 && pen.previous.pointerInfo.PerformanceCount != 0 {
        timestamp = timestamp.max(
            pen.previous
                .pointerInfo
                .PerformanceCount
                .checked_add(minimum_interval)
                .ok_or_else(|| error::clock_error("pen timestamp overflow"))?,
        );
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(100);
    loop {
        if timestamp != 0 {
            if performance_counter()? < timestamp {
                ensure_input!(
                    std::time::Instant::now() < deadline,
                    "pen timestamp is too far in the future"
                );
                futures_timer::Delay::new(Duration::from_millis(1)).await;
                continue;
            }
            native.pointerInfo.PerformanceCount = timestamp;
        }
        let contacts = [POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 { penInfo: native },
        }];
        // UNTESTED: hover must establish inversion before DOWN, and device destruction must end the pen contact.
        // Check tip/eraser transitions and cancellation on an interactive desktop when changing pen injection.
        // SAFETY: the device and its matching, initialized contact array remain live through the call.
        match unsafe { InjectSyntheticPointerInput(pen.device.0, &contacts) } {
            Ok(()) => {
                pen.previous = native;
                return Ok(());
            }
            Err(error) if error.code() == ERROR_NOT_READY.to_hresult() && std::time::Instant::now() < deadline => {
                // Windows rejects successive frames in the same 0.1 ms clock interval without consuming them.
                if timestamp != 0 {
                    timestamp = timestamp
                        .checked_add(minimum_interval)
                        .ok_or_else(|| error::clock_error("pen timestamp overflow"))?;
                }
                futures_timer::Delay::new(Duration::from_millis(1)).await;
            }
            Err(error) => return Err(error::native_error("inject pointer frame", error)),
        }
    }
}
