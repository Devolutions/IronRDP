use core::time::Duration;
use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, anyhow};
use ironrdp_server::{
    CsReadyFlags, CsReadyPdu, PenEventPdu, PenFlags, RdpInputProtocolVersion, TouchContactFlags, TouchEventPdu,
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

struct Device(HSYNTHETICPOINTERDEVICE);

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

#[derive(Default)]
pub(super) struct Input {
    ready: Option<CsReadyPdu>,
    touch_initialized: bool,
    frequency: u64,
    last_touch_timestamp: u64,
    contacts: HashMap<u8, POINTER_TOUCH_INFO>,
    pens: HashMap<u8, Pen>,
}

impl Drop for Input {
    fn drop(&mut self) {
        self.reset();
    }
}

impl Input {
    pub(super) fn initialize(&mut self, ready: CsReadyPdu) -> anyhow::Result<()> {
        self.reset();
        self.touch_initialized = false;
        let mut frequency = 0;
        // SAFETY: the output points to a writable counter-frequency value.
        unsafe { QueryPerformanceFrequency(&mut frequency) }.context("query pointer clock frequency")?;
        self.frequency = u64::try_from(frequency).context("invalid pointer clock frequency")?;
        anyhow::ensure!(self.frequency != 0, "zero pointer clock frequency");
        let touch_enabled = ready.max_touch_contacts != 0;
        self.ready = Some(ready);
        if touch_enabled {
            self.initialize_touch()?;
        }
        Ok(())
    }

    pub(super) fn reset(&mut self) {
        if !self.contacts.is_empty() {
            let mut contacts: Vec<_> = self
                .contacts
                .values()
                .map(|previous| {
                    let mut contact = *previous;
                    contact.pointerInfo.pointerFlags = POINTER_FLAG_CANCELED
                        | if previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT) {
                            POINTER_FLAG_UP
                        } else {
                            POINTER_FLAG_UPDATE
                        };
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
                    if timestamp.saturating_sub(self.last_touch_timestamp) < self.frequency.div_ceil(10_000) {
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
                        tracing::warn!(%error, "Failed to cancel session touch contacts");
                        break;
                    }
                }
            }
        }
        self.contacts.clear();
        self.last_touch_timestamp = 0;
        self.pens.clear();
    }

    fn initialize_touch(&mut self) -> anyhow::Result<()> {
        if !self.touch_initialized {
            let ready = self.ready.as_ref().context("RDPEI client is not ready")?;
            let feedback = if ready.flags.contains(CsReadyFlags::SHOW_TOUCH_VISUALS) {
                TOUCH_FEEDBACK_DEFAULT
            } else {
                TOUCH_FEEDBACK_NONE
            };
            anyhow::ensure!(ready.max_touch_contacts != 0, "RDPEI touch input was not negotiated");
            // Windows bounds pointer IDs by this count; RDPEI IDs span all 256 values.
            // SAFETY: the contact count and feedback mode are documented input values.
            unsafe { InitializeTouchInjection(MAX_TOUCH_COUNT, feedback) }
                .context("initialize session touch injection")?;
            self.touch_initialized = true;
        }
        Ok(())
    }

    pub(super) async fn touch(&mut self, event: TouchEventPdu) -> anyhow::Result<()> {
        let ready = self.ready.as_ref().context("RDPEI client is not ready")?;
        let max_contacts = usize::from(ready.max_touch_contacts).min(usize::try_from(MAX_TOUCH_COUNT)?);
        let timestamps = frame_timestamps(
            self.frequency,
            performance_counter()?,
            self.last_touch_timestamp,
            ready.flags,
            event.encode_time,
            event.frames.iter().map(|frame| frame.frame_offset),
        )?;
        for (frame, timestamp) in event.frames.into_iter().zip(timestamps) {
            anyhow::ensure!(
                !frame.contacts.is_empty() && frame.contacts.len() <= max_contacts,
                "invalid touch frame size"
            );
            let mut contacts = HashMap::with_capacity(frame.contacts.len());
            for contact in frame.contacts {
                anyhow::ensure!(contact.contact_flags.is_legal(), "invalid touch contact flags");
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
                    // Cancellation uses UP for an engaged contact and UPDATE for hover.
                    let engaged = self
                        .contacts
                        .get(&contact.contact_id)
                        .is_some_and(|previous| previous.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT));
                    native.pointerInfo.pointerFlags =
                        POINTER_FLAG_CANCELED | if engaged { POINTER_FLAG_UP } else { POINTER_FLAG_UPDATE };
                }
                if let (Some(left), Some(top), Some(right), Some(bottom)) = (
                    contact.fields.contact_rect_left,
                    contact.fields.contact_rect_top,
                    contact.fields.contact_rect_right,
                    contact.fields.contact_rect_bottom,
                ) {
                    anyhow::ensure!(left <= right && top <= bottom, "invalid touch contact rectangle");
                    native.touchMask |= TOUCH_MASK_CONTACTAREA;
                    native.rcContact = RECT {
                        left: contact
                            .x
                            .checked_add(i32::from(left))
                            .context("touch rectangle left overflow")?,
                        top: contact
                            .y
                            .checked_add(i32::from(top))
                            .context("touch rectangle top overflow")?,
                        right: contact
                            .x
                            .checked_add(i32::from(right))
                            .context("touch rectangle right overflow")?,
                        bottom: contact
                            .y
                            .checked_add(i32::from(bottom))
                            .context("touch rectangle bottom overflow")?,
                    };
                }
                if let Some(orientation) = contact.fields.orientation {
                    native.touchMask |= TOUCH_MASK_ORIENTATION;
                    native.orientation = touch_orientation(orientation)?;
                }
                if let Some(pressure) = contact.fields.pressure {
                    anyhow::ensure!(pressure <= 1024, "invalid touch pressure");
                    native.touchMask |= TOUCH_MASK_PRESSURE;
                    native.pressure = pressure;
                }
                anyhow::ensure!(
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
                    timestamp.saturating_sub(self.frequency.div_ceil(10_000)),
                )
                .await?;
            }
            self.inject_contacts(contacts, max_contacts, timestamp).await?;
        }
        Ok(())
    }

    pub(super) async fn dismiss(&mut self, contact_id: u8) -> anyhow::Result<()> {
        let Some(mut contact) = self.contacts.get(&contact_id).copied() else {
            return Ok(());
        };
        if contact.pointerInfo.pointerFlags.contains(POINTER_FLAG_INCONTACT) {
            return Ok(());
        }
        contact.pointerInfo.pointerFlags = POINTER_FLAG_UPDATE;
        let max_contacts = usize::from(
            self.ready
                .as_ref()
                .context("RDPEI client is not ready")?
                .max_touch_contacts,
        )
        .min(usize::try_from(MAX_TOUCH_COUNT)?);
        let timestamp = if self.last_touch_timestamp == 0 {
            0
        } else {
            performance_counter()?
        };
        self.inject_contacts(HashMap::from([(contact_id, contact)]), max_contacts, timestamp)
            .await
    }

    pub(super) async fn pen(&mut self, event: PenEventPdu) -> anyhow::Result<()> {
        let ready = self.ready.as_ref().context("RDPEI client is not ready")?;
        anyhow::ensure!(
            ready.protocol_version.supports_pen(),
            "RDPEI pen input was not negotiated"
        );
        let multipen = ready.protocol_version == RdpInputProtocolVersion::V300
            && ready.flags.contains(CsReadyFlags::ENABLE_MULTIPEN_INJECTION);
        let minimum_interval = self.frequency.div_ceil(10_000);
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
                    .context("pen timestamp overflow")?
            };
            frame_timestamps(
                self.frequency,
                performance_counter()?,
                previous,
                ready.flags,
                event.encode_time,
                event.frames.iter().map(|frame| frame.frame_offset),
            )?
        };
        for (frame, timestamp) in event.frames.into_iter().zip(timestamps) {
            let mut devices = HashSet::with_capacity(frame.contacts.len());
            for contact in frame.contacts {
                anyhow::ensure!(
                    multipen || contact.device_id == 0,
                    "multiple pen devices were not negotiated"
                );
                anyhow::ensure!(devices.insert(contact.device_id), "duplicate pen device ID");
                anyhow::ensure!(contact.contact_flags.is_legal(), "invalid pen contact flags");
                if contact
                    .contact_flags
                    .contains(ironrdp_server::PenContactFlags::CANCELED)
                {
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
                    for (rdpei, windows) in [
                        (PenFlags::BARREL_PRESSED, PEN_FLAG_BARREL),
                        (PenFlags::ERASER_PRESSED, PEN_FLAG_ERASER),
                        (PenFlags::INVERTED, PEN_FLAG_INVERTED),
                    ] {
                        if flags.contains(rdpei) {
                            native.penFlags |= windows;
                        }
                    }
                    // Windows selects the eraser through inversion and derives its pressed state from contact.
                    if native.penFlags & PEN_FLAG_ERASER != 0 {
                        native.penFlags &= !PEN_FLAG_ERASER;
                        native.penFlags |= PEN_FLAG_INVERTED;
                    }
                }
                if let Some(pressure) = contact.fields.pressure {
                    anyhow::ensure!(pressure <= 1024, "invalid pen pressure");
                    native.penMask |= PEN_MASK_PRESSURE;
                    native.pressure = pressure;
                }
                if let Some(rotation) = contact.fields.rotation {
                    anyhow::ensure!(rotation < 360, "invalid pen rotation");
                    native.penMask |= PEN_MASK_ROTATION;
                    native.rotation = u32::from(rotation);
                }
                if let Some(tilt) = contact.fields.tilt_x {
                    anyhow::ensure!((-90..=90).contains(&tilt), "invalid pen x tilt");
                    native.penMask |= PEN_MASK_TILT_X;
                    native.tiltX = i32::from(tilt);
                }
                if let Some(tilt) = contact.fields.tilt_y {
                    anyhow::ensure!((-90..=90).contains(&tilt), "invalid pen y tilt");
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
                        .context("more than four pen devices are active")?;
                    self.pens.remove(&unused);
                }
                let pen = match self.pens.entry(contact.device_id) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        // SAFETY: a synthetic pen supports exactly one contact; this process owns the handle.
                        let handle = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_NONE) }
                            .context("create session pen device")?;
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
    ) -> anyhow::Result<()> {
        for (&id, previous) in &self.contacts {
            contacts.entry(id).or_insert_with(|| {
                let mut current = *previous;
                current.pointerInfo.pointerFlags &= POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
                current.pointerInfo.pointerFlags |= POINTER_FLAG_UPDATE;
                current
            });
        }
        anyhow::ensure!(contacts.len() <= max_contacts, "too many active touch contacts");
        let mut native: Vec<_> = contacts.values().copied().collect();
        let minimum_interval = self.frequency.div_ceil(10_000);
        if timestamp != 0 && self.last_touch_timestamp != 0 {
            timestamp = timestamp.max(
                self.last_touch_timestamp
                    .checked_add(minimum_interval)
                    .context("touch timestamp overflow")?,
            );
        }
        self.initialize_touch()?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        loop {
            if timestamp != 0 {
                if performance_counter()? < timestamp {
                    anyhow::ensure!(
                        tokio::time::Instant::now() < deadline,
                        "touch timestamp is too far in the future"
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    continue;
                }
                for contact in &mut native {
                    contact.pointerInfo.PerformanceCount = timestamp;
                }
            }
            // InjectSyntheticPointerInput drops CANCELED; InjectTouchInput preserves individual cancellation.
            // SAFETY: the initialized device accepts these bounded IDs and fully initialized touch records.
            match unsafe { InjectTouchInput(&native) } {
                Ok(()) => break,
                Err(error)
                    if error.code() == ERROR_NOT_READY.to_hresult() && tokio::time::Instant::now() < deadline =>
                {
                    if timestamp != 0 {
                        timestamp = timestamp
                            .checked_add(minimum_interval)
                            .context("touch timestamp overflow")?;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => return Err(anyhow!(error).context("inject session touch frame")),
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

fn performance_counter() -> anyhow::Result<u64> {
    let mut counter = 0;
    // SAFETY: the output points to a writable counter value.
    unsafe { QueryPerformanceCounter(&mut counter) }.context("query pointer clock")?;
    u64::try_from(counter).context("invalid pointer clock value")
}

fn counter_ticks(frequency: u64, microseconds: u64) -> anyhow::Result<u64> {
    u64::try_from(u128::from(frequency) * u128::from(microseconds) / 1_000_000)
        .context("pointer timestamp exceeds the native clock range")
}

fn frame_timestamps(
    frequency: u64,
    now: u64,
    previous: u64,
    flags: CsReadyFlags,
    encode_time: u32,
    offsets: impl Iterator<Item = u64>,
) -> anyhow::Result<Vec<u64>> {
    if flags.contains(CsReadyFlags::DISABLE_TIMESTAMP_INJECTION) {
        return Ok(offsets.map(|_| 0).collect());
    }
    let mut timestamps = Vec::new();
    let mut span = 0u64;
    for offset in offsets {
        anyhow::ensure!(
            !timestamps.is_empty() || offset == 0,
            "first pointer frame offset is not zero"
        );
        span = span.checked_add(offset).context("pointer frame offset overflow")?;
        timestamps.push(span);
    }
    anyhow::ensure!(!timestamps.is_empty(), "empty pointer event");
    // Rounded encoding ages and synthetic taps can be shorter than their frame span.
    let age = (u64::from(encode_time) * 1000).max(span);
    let mut base = now
        .checked_sub(counter_ticks(frequency, age)?)
        .context("pointer timestamp predates the native clock")?;
    if previous != 0 {
        base = base.max(
            previous
                .checked_add(frequency.div_ceil(10_000))
                .context("pointer timestamp overflow")?,
        );
    }
    anyhow::ensure!(
        2 * frequency.div_ceil(10_000) < base,
        "pointer timestamp leaves no clock range for native hover samples"
    );
    let latest = now.checked_add(frequency / 10).context("pointer clock overflow")?;
    for timestamp in &mut timestamps {
        *timestamp = base
            .checked_add(counter_ticks(frequency, *timestamp)?)
            .context("pointer timestamp overflow")?;
        anyhow::ensure!(*timestamp <= latest, "pointer timestamp is too far in the future");
    }
    Ok(timestamps)
}

fn touch_orientation(orientation: u32) -> anyhow::Result<u32> {
    anyhow::ensure!(orientation < 360, "invalid touch orientation");
    // RDPEI measures counter-clockwise from up; Windows measures clockwise from right.
    Ok((630 - orientation) % 360)
}

fn pointer_flags(flags: TouchContactFlags) -> POINTER_FLAGS {
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

async fn inject_pen(
    pen: &mut Pen,
    frequency: u64,
    mut timestamp: u64,
    mut native: POINTER_PEN_INFO,
) -> anyhow::Result<()> {
    let minimum_interval = frequency.div_ceil(10_000);
    if timestamp != 0 && pen.previous.pointerInfo.PerformanceCount != 0 {
        timestamp = timestamp.max(
            pen.previous
                .pointerInfo
                .PerformanceCount
                .checked_add(minimum_interval)
                .context("pen timestamp overflow")?,
        );
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    loop {
        if timestamp != 0 {
            if performance_counter()? < timestamp {
                anyhow::ensure!(
                    tokio::time::Instant::now() < deadline,
                    "pen timestamp is too far in the future"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
                continue;
            }
            native.pointerInfo.PerformanceCount = timestamp;
        }
        let contacts = [POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 { penInfo: native },
        }];
        // SAFETY: the device and its matching, initialized contact array remain live through the call.
        match unsafe { InjectSyntheticPointerInput(pen.device.0, &contacts) } {
            Ok(()) => {
                pen.previous = native;
                return Ok(());
            }
            Err(error) if error.code() == ERROR_NOT_READY.to_hresult() && tokio::time::Instant::now() < deadline => {
                // Windows rejects successive frames in the same 0.1 ms clock interval without consuming them.
                if timestamp != 0 {
                    timestamp = timestamp
                        .checked_add(minimum_interval)
                        .context("pen timestamp overflow")?;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(error) => return Err(anyhow!(error).context("inject session pointer frame")),
        }
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_server::CsReadyFlags;

    use super::{frame_timestamps, touch_orientation};

    #[test]
    fn touch_orientation_matches_windows_axes() -> anyhow::Result<()> {
        // oracle: MS-RDPEI 2.2.3.3.1.1 starts upward and increases counter-clockwise;
        // https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_touch_info
        // starts rightward and increases clockwise.
        assert_eq!(touch_orientation(0)?, 270);
        assert_eq!(touch_orientation(90)?, 180);
        assert_eq!(touch_orientation(180)?, 90);
        assert_eq!(touch_orientation(270)?, 0);
        assert_eq!(touch_orientation(359)?, 271);
        Ok(())
    }

    #[test]
    fn touch_frames_preserve_timestamp_intervals() -> anyhow::Result<()> {
        // oracle: MS-RDPEI 2.2.3.3 and 2.2.3.3.1 define the oldest-frame age in milliseconds and offsets in microseconds.
        assert_eq!(
            frame_timestamps(
                10_000_000,
                10_000_000,
                0,
                CsReadyFlags::empty(),
                20,
                [0, 16_000].into_iter()
            )?,
            [9_800_000, 9_960_000]
        );
        Ok(())
    }

    #[test]
    fn synthetic_taps_do_not_generate_future_timestamps() -> anyhow::Result<()> {
        // oracle: InjectTouchInput forbids future timestamps; preserve the caller's 16 ms interval despite an encoding age of zero.
        assert_eq!(
            frame_timestamps(
                10_000_000,
                10_000_000,
                0,
                CsReadyFlags::empty(),
                0,
                [0, 16_000].into_iter()
            )?,
            [9_840_000, 10_000_000]
        );
        Ok(())
    }

    #[test]
    fn pointer_timestamps_reject_invalid_or_unbounded_offsets() {
        // oracle: MS-RDPEI requires a zero first offset; native timestamps cannot overflow or predate the clock origin.
        assert!(frame_timestamps(10_000_000, 10_000_000, 0, CsReadyFlags::empty(), 20, [1].into_iter()).is_err());
        assert!(
            frame_timestamps(
                10_000_000,
                10_000_000,
                0,
                CsReadyFlags::empty(),
                20,
                [0, u64::MAX, 1].into_iter()
            )
            .is_err()
        );
        assert!(frame_timestamps(10_000_000, 10_000_000, 0, CsReadyFlags::empty(), 2000, [0].into_iter()).is_err());
    }

    #[test]
    fn disabled_timestamp_injection_ignores_timing_fields() -> anyhow::Result<()> {
        // oracle: MS-RDPEI 2.2.3.2 requires ignoring encodeTime and frameOffset when timestamp injection is disabled.
        assert_eq!(
            frame_timestamps(
                0,
                0,
                u64::MAX,
                CsReadyFlags::DISABLE_TIMESTAMP_INJECTION,
                u32::MAX,
                [u64::MAX, u64::MAX].into_iter()
            )?,
            [0, 0]
        );
        Ok(())
    }
}
