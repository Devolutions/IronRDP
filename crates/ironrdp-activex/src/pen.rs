//! ActiveX pen → MS-RDPEI pen contact tracking.
//!
//! Maps Win32 `WM_POINTER*` pen samples onto RDPEI pen frames ([MS-RDPEI] 2.2.3.7 "RDPINPUT_PEN_EVENT_PDU"
//! and 3.1.1.1 "Touch Contact State Transitions").
//! Every digitizer sample becomes one pen frame, so a coalesced `WM_POINTERUPDATE` and its history
//! become one PDU with one frame per sample, timed by the digitizer.
//!
//! [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d

use std::collections::HashMap;

use ironrdp_rdpei::contact::{self, ContactSample, ContactState};
use ironrdp_rdpei::pdu::{PenContact, PenContactDataFlags, PenContactFlags, PenEventPdu, PenFlags, PenFrame};

/// Pens the server injects at once when multipen injection is negotiated ([MS-RDPEI] 2.2.3.1
/// "RDPINPUT_SC_READY_PDU").
///
/// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
const MAX_MULTIPEN_DEVICES: u8 = 4;

/// Win32 `POINTER_PEN_INFO::penFlags` values (`PEN_FLAG_*`).
const WIN32_PEN_FLAG_BARREL: u32 = 0x1;
const WIN32_PEN_FLAG_INVERTED: u32 = 0x2;
const WIN32_PEN_FLAG_ERASER: u32 = 0x4;

/// One Win32 pen sample converted into desktop coordinates and RDPEI units.
#[derive(Debug, Clone)]
pub(crate) struct PenSample {
    pub pointer_id: u32,
    pub x: i32,
    pub y: i32,
    /// When true, coordinates are a placeholder and the last tracked position is kept.
    pub preserve_position: bool,
    pub in_range: bool,
    pub in_contact: bool,
    pub canceled: bool,
    pub pen_flags: PenFlags,
    /// Normalized to 0..=1024.
    pub pressure: Option<u32>,
    /// Clockwise degrees, 0..=359.
    pub rotation: Option<u16>,
    /// Degrees, -90..=90.
    pub tilt_x: Option<i16>,
    /// Degrees, -90..=90.
    pub tilt_y: Option<i16>,
    /// Microseconds on a monotonic clock when the digitizer produced the sample.
    pub timestamp_us: u64,
}

impl PenSample {
    /// Converts the Win32 `POINTER_PEN_INFO` value fields to RDPEI units.
    ///
    /// `pen_mask` says which of `pressure`, `rotation`, `tilt_x` and `tilt_y` the digitizer reported.
    pub(crate) fn win32_values(
        pen_flags: u32,
        pen_mask: u32,
        pressure: u32,
        rotation: u32,
        tilt_x: i32,
        tilt_y: i32,
    ) -> (PenFlags, Option<u32>, Option<u16>, Option<i16>, Option<i16>) {
        const PEN_MASK_PRESSURE: u32 = 0x1;
        const PEN_MASK_ROTATION: u32 = 0x2;
        const PEN_MASK_TILT_X: u32 = 0x4;
        const PEN_MASK_TILT_Y: u32 = 0x8;

        let mut flags = PenFlags::empty();
        if pen_flags & WIN32_PEN_FLAG_BARREL != 0 {
            flags |= PenFlags::BARREL_PRESSED;
        }
        if pen_flags & WIN32_PEN_FLAG_ERASER != 0 {
            flags |= PenFlags::ERASER_PRESSED;
        }
        if pen_flags & WIN32_PEN_FLAG_INVERTED != 0 {
            flags |= PenFlags::INVERTED;
        }

        let tilt = |value: i32| i16::try_from(value.clamp(-90, 90)).expect("clamped to -90..=90");
        (
            flags,
            (pen_mask & PEN_MASK_PRESSURE != 0).then_some(pressure.min(1024)),
            (pen_mask & PEN_MASK_ROTATION != 0).then(|| u16::try_from(rotation % 360).expect("reduced modulo 360")),
            (pen_mask & PEN_MASK_TILT_X != 0).then(|| tilt(tilt_x)),
            (pen_mask & PEN_MASK_TILT_Y != 0).then(|| tilt(tilt_y)),
        )
    }
}

#[derive(Debug, Clone)]
struct TrackedPen {
    state: ContactState,
    /// Last contact sent for this pen, replayed in frames produced by other pens.
    last: PenContact,
}

/// Tracks pens in range and builds RDPEI pen events.
#[derive(Debug, Default)]
pub(crate) struct PenContactTracker {
    by_pointer: HashMap<u32, TrackedPen>,
    multipen: bool,
    /// Timestamp of the last frame sent, for `frameOffset`.
    last_frame_us: Option<u64>,
}

impl PenContactTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn has_active_contacts(&self) -> bool {
        !self.by_pointer.is_empty()
    }

    /// Forgets every pen without telling the server, for when the channel stopped accepting pen frames.
    ///
    /// `multipen` is the new negotiated multipen state.
    pub(crate) fn reset(&mut self, multipen: bool) {
        self.by_pointer.clear();
        self.multipen = multipen;
        self.last_frame_us = None;
    }

    /// Builds a pen event for samples ordered from oldest to newest, advancing the local state.
    ///
    /// Each sample becomes its own frame.
    /// Returns `None` when no sample produces a contact transition.
    /// Callers must keep the event until the session accepts it: the tracker already moved past it.
    pub(crate) fn process_samples(&mut self, samples: &[PenSample], now_us: u64) -> Option<PenEventPdu> {
        let mut frames = Vec::with_capacity(samples.len());
        let mut oldest_us = None;

        for sample in samples {
            let tracked = self.by_pointer.get(&sample.pointer_id);
            let Some(transition) = contact::transition(
                tracked.map(|pen| pen.state),
                ContactSample {
                    in_range: sample.in_range,
                    in_contact: sample.in_contact,
                    canceled: sample.canceled,
                },
            ) else {
                continue;
            };

            let device_id = match tracked {
                Some(pen) => pen.last.device_id,
                None => match self.allocate_device_id() {
                    Some(device_id) => device_id,
                    // A second pen without multipen, or a fifth pen with it: the server cannot take it.
                    None => continue,
                },
            };
            // A lift or cancel is sent where the pen last touched; it moves only after the transition.
            let (x, y) = match tracked {
                Some(pen) if sample.preserve_position || transition.keeps_position() => (pen.last.x, pen.last.y),
                _ => (sample.x, sample.y),
            };

            let mut contact = PenContact::new(device_id, x, y, transition.pen_flags()).with_pen_flags(sample.pen_flags);
            if let Some(pressure) = sample.pressure {
                contact = contact.with_pressure(pressure);
            }
            if let Some(rotation) = sample.rotation {
                contact = contact.with_rotation(rotation);
            }
            // Digitizers may report one tilt axis only, so each axis gets its own field flag.
            if let Some(tilt_x) = sample.tilt_x {
                contact.fields_present.insert(PenContactDataFlags::TILTX_PRESENT);
                contact.fields.tilt_x = Some(tilt_x);
            }
            if let Some(tilt_y) = sample.tilt_y {
                contact.fields_present.insert(PenContactDataFlags::TILTY_PRESENT);
                contact.fields.tilt_y = Some(tilt_y);
            }

            let mut contacts = Vec::with_capacity(self.by_pointer.len() + 1);
            contacts.extend(
                self.by_pointer
                    .iter()
                    .filter(|(pointer_id, _)| **pointer_id != sample.pointer_id)
                    .map(|(_, pen)| steady_contact(pen)),
            );
            contacts.push(contact.clone());

            match transition.next {
                Some(state) => {
                    self.by_pointer
                        .insert(sample.pointer_id, TrackedPen { state, last: contact });
                }
                None => {
                    self.by_pointer.remove(&sample.pointer_id);
                }
            }

            let frame_offset = self
                .last_frame_us
                .map_or(0, |last| sample.timestamp_us.saturating_sub(last));
            // The transaction ends once every pen is out of range, and the next one starts at offset 0.
            self.last_frame_us = (!self.by_pointer.is_empty()).then_some(sample.timestamp_us);
            oldest_us.get_or_insert(sample.timestamp_us);
            frames.push(PenFrame::new(frame_offset, contacts));
        }

        // `encodeTime` is the delay from the oldest frame's sample to encoding, in milliseconds.
        let encode_time = u32::try_from(now_us.saturating_sub(oldest_us?) / 1000).unwrap_or(u32::MAX);
        Some(PenEventPdu::new(encode_time, frames))
    }

    /// Takes every tracked pen out of range, for focus loss or capture change.
    pub(crate) fn release_all(&mut self, now_us: u64) -> Option<PenEventPdu> {
        if self.by_pointer.is_empty() {
            return None;
        }

        let contacts = self
            .by_pointer
            .drain()
            .map(|(_, pen)| {
                let flags = PenContactFlags::from_bits_truncate(contact::release_flags(pen.state).bits());
                PenContact::new(pen.last.device_id, pen.last.x, pen.last.y, flags)
            })
            .collect();

        let frame_offset = self.last_frame_us.take().map_or(0, |last| now_us.saturating_sub(last));
        Some(PenEventPdu::new(0, vec![PenFrame::new(frame_offset, contacts)]))
    }

    fn allocate_device_id(&self) -> Option<u8> {
        let limit = if self.multipen { MAX_MULTIPEN_DEVICES } else { 1 };
        (0..limit).find(|id| self.by_pointer.values().all(|pen| pen.last.device_id != *id))
    }
}

/// A pen that did not move in this frame, restated with the flags that keep its state.
fn steady_contact(pen: &TrackedPen) -> PenContact {
    let flags = match pen.state {
        ContactState::Hovering => PenContactFlags::UPDATE | PenContactFlags::INRANGE,
        ContactState::Engaged => PenContactFlags::UPDATE | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
    };
    PenContact {
        contact_flags: flags,
        ..pen.last.clone()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn sample(pointer_id: u32, x: i32, in_range: bool, in_contact: bool, timestamp_us: u64) -> PenSample {
        PenSample {
            pointer_id,
            x,
            y: 0,
            preserve_position: false,
            in_range,
            in_contact,
            canceled: false,
            pen_flags: PenFlags::empty(),
            pressure: None,
            rotation: None,
            tilt_x: None,
            tilt_y: None,
            timestamp_us,
        }
    }

    fn flags(pdu: &PenEventPdu) -> Vec<PenContactFlags> {
        pdu.frames
            .iter()
            .map(|frame| frame.contacts.last().expect("contact").contact_flags)
            .collect()
    }

    // oracle: spec MS-RDPEI §3.1.1.1
    #[test]
    fn hover_down_draw_lift_leave() {
        let mut tracker = PenContactTracker::new();
        let samples = [
            sample(5, 0, true, false, 1_000),
            sample(5, 1, true, true, 2_000),
            sample(5, 2, true, true, 3_000),
            sample(5, 3, true, false, 4_000),
            sample(5, 3, false, false, 5_000),
        ];
        let pdu = tracker.process_samples(&samples, 5_000).expect("pdu");
        assert_eq!(
            flags(&pdu),
            [
                PenContactFlags::UPDATE | PenContactFlags::INRANGE,
                PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
                PenContactFlags::UPDATE | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
                PenContactFlags::UP | PenContactFlags::INRANGE,
                PenContactFlags::UPDATE,
            ]
        );
        assert!(flags(&pdu).iter().all(|flags| flags.is_legal()));
        assert!(!tracker.has_active_contacts());
    }

    // oracle: spec MS-RDPEI §3.1.1.1
    #[test]
    fn lift_and_cancel_keep_the_last_engaged_position() {
        let mut tracker = PenContactTracker::new();
        let samples = [
            sample(5, 10, true, true, 0),
            sample(5, 20, true, true, 1_000),
            sample(5, 25, true, false, 2_000),
            sample(5, 30, true, false, 3_000),
        ];
        let pdu = tracker.process_samples(&samples, 3_000).expect("pdu");
        let xs: Vec<_> = pdu.frames.iter().map(|frame| frame.contacts[0].x).collect();
        assert_eq!(xs, [10, 20, 20, 30]);

        tracker
            .process_samples(&[sample(5, 40, true, true, 4_000)], 4_000)
            .expect("down");
        let mut cancel = sample(5, 45, false, false, 5_000);
        cancel.canceled = true;
        let pdu = tracker.process_samples(&[cancel], 5_000).expect("cancel");
        assert_eq!(pdu.frames[0].contacts[0].x, 40);
    }

    fn arbitrary_sample() -> impl Strategy<Value = PenSample> {
        (0u32..6, any::<i16>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
            |(pointer_id, x, in_range, in_contact, canceled)| PenSample {
                canceled,
                ..sample(pointer_id, i32::from(x), in_range, in_contact, 0)
            },
        )
    }

    // oracle: spec MS-RDPEI §2.2.3.7.1.1 for flags and device IDs, §2.2.3.1 for the four-pen limit
    proptest! {
        #[test]
        fn frames_carry_legal_flags_and_device_ids(
            multipen in any::<bool>(),
            samples in proptest::collection::vec(arbitrary_sample(), 1..64),
        ) {
            let mut tracker = PenContactTracker::new();
            tracker.reset(multipen);
            // Up to four pens with multipen injection, otherwise only device 0.
            let limit = if multipen { 4 } else { 1 };
            for sample in samples {
                let Some(pdu) = tracker.process_samples(&[sample], 0) else {
                    continue;
                };
                for frame in &pdu.frames {
                    let mut device_ids: Vec<_> = frame.contacts.iter().map(|contact| contact.device_id).collect();
                    device_ids.sort_unstable();
                    device_ids.dedup();
                    prop_assert_eq!(device_ids.len(), frame.contacts.len());
                    for contact in &frame.contacts {
                        prop_assert!(contact.contact_flags.is_legal(), "{:?}", contact.contact_flags);
                        prop_assert!(contact.device_id < limit);
                    }
                }
            }
        }
    }

    // oracle: spec MS-RDPEI §2.2.3.7
    #[test]
    fn history_becomes_one_frame_per_sample_with_digitizer_offsets() {
        let mut tracker = PenContactTracker::new();
        let first = tracker
            .process_samples(&[sample(1, 0, true, true, 10_000)], 12_000)
            .expect("down");
        assert_eq!(first.frames[0].frame_offset, 0);
        assert_eq!(first.encode_time, 2);

        let history = [
            sample(1, 1, true, true, 14_000),
            sample(1, 2, true, true, 18_000),
            sample(1, 3, true, true, 26_000),
        ];
        let pdu = tracker.process_samples(&history, 30_000).expect("moves");
        let offsets: Vec<_> = pdu.frames.iter().map(|frame| frame.frame_offset).collect();
        assert_eq!(offsets, [4_000, 4_000, 8_000]);
        assert_eq!(pdu.encode_time, 16);
        let xs: Vec<_> = pdu.frames.iter().map(|frame| frame.contacts[0].x).collect();
        assert_eq!(xs, [1, 2, 3]);
    }

    // oracle: spec MS-RDPEI §2.2.3.7.1.1
    #[test]
    fn optional_fields_set_field_flags() {
        let mut tracker = PenContactTracker::new();
        let mut down = sample(1, 0, true, true, 0);
        (down.pen_flags, down.pressure, down.rotation, down.tilt_x, down.tilt_y) =
            PenSample::win32_values(WIN32_PEN_FLAG_BARREL, 0xF, 512, 45, -30, 20);
        let pdu = tracker.process_samples(&[down], 0).expect("down");
        let contact = &pdu.frames[0].contacts[0];
        assert_eq!(contact.fields.pen_flags, Some(PenFlags::BARREL_PRESSED));
        assert_eq!(contact.fields.pressure, Some(512));
        assert_eq!(contact.fields.rotation, Some(45));
        assert_eq!((contact.fields.tilt_x, contact.fields.tilt_y), (Some(-30), Some(20)));
        assert_eq!(contact.fields_present.bits(), 0x1F);

        let mut tilt_x_only = sample(1, 1, true, true, 1);
        tilt_x_only.tilt_x = Some(10);
        let pdu = tracker.process_samples(&[tilt_x_only], 1).expect("move");
        assert_eq!(
            pdu.frames[0].contacts[0].fields_present,
            PenContactDataFlags::PENFLAGS_PRESENT | PenContactDataFlags::TILTX_PRESENT
        );
    }

    // oracle: reference winuser.h PEN_FLAG_* and PEN_MASK_* values, spec MS-RDPEI §2.2.3.7.1.1 field ranges
    #[test]
    fn win32_values_map_flags_and_clamp_ranges() {
        let (flags, pressure, rotation, tilt_x, tilt_y) = PenSample::win32_values(
            WIN32_PEN_FLAG_ERASER | WIN32_PEN_FLAG_INVERTED,
            0xF,
            4096,
            725,
            -120,
            95,
        );
        assert_eq!(flags, PenFlags::ERASER_PRESSED | PenFlags::INVERTED);
        assert_eq!(pressure, Some(1024));
        assert_eq!(rotation, Some(5));
        assert_eq!((tilt_x, tilt_y), (Some(-90), Some(90)));

        let (_, pressure, rotation, tilt_x, tilt_y) = PenSample::win32_values(0, 0, 512, 45, 10, 10);
        assert_eq!((pressure, rotation, tilt_x, tilt_y), (None, None, None, None));
    }

    // oracle: spec MS-RDPEI §2.2.3.7.1.1
    #[test]
    fn second_pen_is_dropped_without_multipen() {
        let mut tracker = PenContactTracker::new();
        tracker
            .process_samples(&[sample(1, 0, true, true, 0)], 0)
            .expect("first");
        assert!(tracker.process_samples(&[sample(2, 0, true, true, 1)], 1).is_none());
    }

    // oracle: spec MS-RDPEI §2.2.3.1
    #[test]
    fn multipen_assigns_device_ids_and_restates_other_pens() {
        let mut tracker = PenContactTracker::new();
        tracker.reset(true);
        tracker
            .process_samples(&[sample(10, 1, true, true, 0)], 0)
            .expect("first");
        let pdu = tracker
            .process_samples(&[sample(20, 2, true, false, 1_000)], 1_000)
            .expect("second");
        let contacts = &pdu.frames[0].contacts;
        assert_eq!(contacts.len(), 2);
        assert_eq!((contacts[0].device_id, contacts[0].x), (0, 1));
        assert_eq!(
            contacts[0].contact_flags,
            PenContactFlags::UPDATE | PenContactFlags::INRANGE | PenContactFlags::INCONTACT
        );
        assert_eq!((contacts[1].device_id, contacts[1].x), (1, 2));

        for pointer_id in [30, 40] {
            tracker
                .process_samples(&[sample(pointer_id, 0, true, false, 2_000)], 2_000)
                .expect("third and fourth");
        }
        assert!(
            tracker
                .process_samples(&[sample(50, 0, true, false, 3_000)], 3_000)
                .is_none()
        );
    }

    // oracle: spec MS-RDPEI §3.1.1.1
    #[test]
    fn cancel_and_release_end_tracking() {
        let mut tracker = PenContactTracker::new();
        tracker
            .process_samples(&[sample(1, 7, true, true, 0)], 0)
            .expect("down");
        let mut cancel = sample(1, 0, false, false, 1_000);
        cancel.canceled = true;
        cancel.preserve_position = true;
        let pdu = tracker.process_samples(&[cancel], 1_000).expect("cancel");
        assert_eq!(
            pdu.frames[0].contacts[0].contact_flags,
            PenContactFlags::UP | PenContactFlags::CANCELED
        );
        assert_eq!(pdu.frames[0].contacts[0].x, 7);

        tracker
            .process_samples(&[sample(2, 0, true, false, 2_000)], 2_000)
            .expect("hover");
        let pdu = tracker.release_all(3_000).expect("release");
        assert_eq!(pdu.frames[0].contacts[0].contact_flags, PenContactFlags::UPDATE);
        assert_eq!(pdu.frames[0].frame_offset, 1_000);
        assert!(tracker.release_all(4_000).is_none());
    }

    // oracle: spec MS-RDPEI §2.2.3.7.1
    #[test]
    fn a_new_transaction_starts_at_frame_offset_zero() {
        let mut tracker = PenContactTracker::new();
        tracker
            .process_samples(&[sample(1, 0, true, true, 1_000)], 1_000)
            .expect("down");
        tracker
            .process_samples(&[sample(1, 0, false, false, 2_000)], 2_000)
            .expect("up");
        let pdu = tracker
            .process_samples(&[sample(1, 0, true, true, 90_000_000)], 90_000_000)
            .expect("next down");
        assert_eq!(pdu.frames[0].frame_offset, 0);
    }
}
