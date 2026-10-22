//! ActiveX touch → MS-RDPEI contact tracking.
//!
//! Maps Win32 `WM_POINTER*` touch samples onto the RDPEI contact state machine
//! ([MS-RDPEI] 2.2.3.3.1.1 / 3.1.1.1).

use std::collections::HashMap;
use std::time::Instant;

use ironrdp_rdpei::contact::{self, ContactSample, ContactState};
use ironrdp_rdpei::pdu::{TouchContact, TouchEventPdu, TouchFrame};

#[derive(Debug, Clone)]
struct TrackedContact {
    contact_id: u8,
    phase: ContactState,
    x: i32,
    y: i32,
    started_at: Instant,
}

/// One Win32 pointer sample converted into desktop coordinates.
#[derive(Debug, Clone)]
pub(crate) struct TouchSample {
    pub pointer_id: u32,
    pub x: i32,
    pub y: i32,
    /// When true, coordinates are a placeholder and the last tracked position should be kept.
    pub preserve_position: bool,
    pub in_range: bool,
    pub in_contact: bool,
    pub canceled: bool,
    /// RDPEI orientation degrees (0 = up, counter-clockwise), already converted from Win32.
    pub orientation: Option<u32>,
    pub pressure: Option<u32>,
    /// Contact rect offsets relative to `(x, y)`, not absolute corners.
    pub contact_rect: Option<(i16, i16, i16, i16)>,
}

/// Tracks active contacts and builds RDPEI touch frames.
#[derive(Debug, Default)]
pub(crate) struct TouchContactTracker {
    by_pointer: HashMap<u32, TrackedContact>,
    next_contact_id: u8,
}

impl TouchContactTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn has_active_contacts(&self) -> bool {
        !self.by_pointer.is_empty()
    }

    /// Converts Win32 pointer orientation (degrees clockwise from +X) to RDPEI
    /// orientation (degrees counter-clockwise from +Y / up).
    #[must_use]
    pub(crate) fn win32_orientation_to_rdpei(win32_degrees: u32) -> u32 {
        // RDPEI = (90 - win32) mod 360
        (90 + 360 - (win32_degrees % 360)) % 360
    }

    /// Builds a touch event for the given samples, advancing the local FSM.
    ///
    /// Returns `None` when no legal contact transitions are produced (for example
    /// an untracked cancel). Callers must keep the event until the session
    /// accepts it: the tracker already moved past it.
    pub(crate) fn process_samples(&mut self, samples: &[TouchSample]) -> Option<TouchEventPdu> {
        let mut contacts = Vec::with_capacity(samples.len());
        let mut finished = Vec::new();

        for sample in samples {
            let prev = self.by_pointer.get(&sample.pointer_id).cloned();
            // A placeholder position is usable only for a contact the server already knows.
            if sample.preserve_position && prev.is_none() {
                continue;
            }
            let Some(transition) = contact::transition(
                prev.as_ref().map(|c| c.phase),
                ContactSample {
                    in_range: sample.in_range,
                    in_contact: sample.in_contact,
                    canceled: sample.canceled,
                },
            ) else {
                continue;
            };

            let contact_id = match prev.as_ref() {
                Some(c) => c.contact_id,
                None => self.allocate_contact_id(),
            };
            // A lift or cancel is sent where the finger last touched; it moves only after the transition.
            let (x, y) = match prev.as_ref() {
                Some(c) if sample.preserve_position || transition.keeps_position() => (c.x, c.y),
                _ => (sample.x, sample.y),
            };
            let started_at = prev.as_ref().map(|c| c.started_at).unwrap_or_else(Instant::now);

            let mut contact = TouchContact::new(contact_id, x, y, transition.flags);
            if let Some(orientation) = sample.orientation {
                contact = contact.with_orientation(orientation);
            }
            if let Some(pressure) = sample.pressure {
                // RDPEI pressure is 0..=1024; Win32 touch pressure is 0..=1024 already.
                contact = contact.with_pressure(pressure.min(1024));
            }
            if let Some((left, top, right, bottom)) = sample.contact_rect {
                contact = contact.with_contact_rect(left, top, right, bottom);
            }
            contacts.push(contact);

            match transition.next {
                None => {
                    finished.push(sample.pointer_id);
                }
                Some(phase) => {
                    self.by_pointer.insert(
                        sample.pointer_id,
                        TrackedContact {
                            contact_id,
                            phase,
                            x,
                            y,
                            started_at,
                        },
                    );
                }
            }
        }

        for pointer_id in finished {
            self.by_pointer.remove(&pointer_id);
        }

        if contacts.is_empty() {
            return None;
        }

        // Single-frame PDUs are encoded immediately; encodeTime is the generation→encode
        // delay for the oldest frame, which is ~0 here (not transaction age).
        Some(TouchEventPdu::new(0, vec![TouchFrame::new(0, contacts)]))
    }

    /// Forces every tracked contact out of range (focus loss / capture change).
    pub(crate) fn release_all(&mut self) -> Option<TouchEventPdu> {
        if self.by_pointer.is_empty() {
            return None;
        }

        let mut contacts = Vec::with_capacity(self.by_pointer.len());
        for (_, tracked) in self.by_pointer.drain() {
            contacts.push(TouchContact::new(
                tracked.contact_id,
                tracked.x,
                tracked.y,
                contact::release_flags(tracked.phase),
            ));
        }

        Some(TouchEventPdu::new(0, vec![TouchFrame::new(0, contacts)]))
    }

    fn allocate_contact_id(&mut self) -> u8 {
        // Prefer unused IDs; wrap if needed. Contact IDs are 8-bit.
        let used: std::collections::HashSet<u8> = self.by_pointer.values().map(|c| c.contact_id).collect();
        for _ in 0..=u8::MAX {
            let id = self.next_contact_id;
            self.next_contact_id = self.next_contact_id.wrapping_add(1);
            if !used.contains(&id) {
                return id;
            }
        }
        // All 256 IDs in use; reuse next.
        let id = self.next_contact_id;
        self.next_contact_id = self.next_contact_id.wrapping_add(1);
        id
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpei::pdu::TouchContactFlags;

    use super::*;

    fn sample(pointer_id: u32, x: i32, y: i32, in_range: bool, in_contact: bool) -> TouchSample {
        TouchSample {
            pointer_id,
            x,
            y,
            preserve_position: false,
            in_range,
            in_contact,
            canceled: false,
            orientation: None,
            pressure: None,
            contact_rect: None,
        }
    }

    #[test]
    fn down_update_up_happy_path() {
        let mut tracker = TouchContactTracker::new();
        let down = tracker.process_samples(&[sample(7, 10, 20, true, true)]).expect("down");
        assert_eq!(
            down.frames[0].contacts[0].contact_flags,
            TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT
        );
        assert_eq!(down.encode_time, 0);

        let mov = tracker.process_samples(&[sample(7, 11, 21, true, true)]).expect("move");
        assert_eq!(
            mov.frames[0].contacts[0].contact_flags,
            TouchContactFlags::UPDATE | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT
        );

        let up = tracker.process_samples(&[sample(7, 11, 21, false, false)]).expect("up");
        assert_eq!(up.frames[0].contacts[0].contact_flags, TouchContactFlags::UP);
        assert!(!tracker.has_active_contacts());
    }

    #[test]
    fn hover_then_leave_uses_update() {
        let mut tracker = TouchContactTracker::new();
        let hover = tracker.process_samples(&[sample(1, 5, 5, true, false)]).expect("hover");
        assert_eq!(
            hover.frames[0].contacts[0].contact_flags,
            TouchContactFlags::UPDATE | TouchContactFlags::INRANGE
        );

        let leave = tracker
            .process_samples(&[sample(1, 5, 5, false, false)])
            .expect("leave");
        assert_eq!(leave.frames[0].contacts[0].contact_flags, TouchContactFlags::UPDATE);
    }

    #[test]
    fn engaged_lift_to_hover_uses_up_inrange() {
        let mut tracker = TouchContactTracker::new();
        tracker.process_samples(&[sample(1, 1, 1, true, true)]).expect("down");
        let lift = tracker.process_samples(&[sample(1, 1, 1, true, false)]).expect("lift");
        assert_eq!(
            lift.frames[0].contacts[0].contact_flags,
            TouchContactFlags::UP | TouchContactFlags::INRANGE
        );
    }

    #[test]
    fn cancel_engaged_uses_up_canceled() {
        let mut tracker = TouchContactTracker::new();
        tracker.process_samples(&[sample(3, 0, 0, true, true)]).expect("down");
        let mut cancel = sample(3, 0, 0, false, false);
        cancel.canceled = true;
        let pdu = tracker.process_samples(&[cancel]).expect("cancel");
        assert_eq!(
            pdu.frames[0].contacts[0].contact_flags,
            TouchContactFlags::UP | TouchContactFlags::CANCELED
        );
    }

    #[test]
    fn cancel_hovering_uses_update_canceled() {
        let mut tracker = TouchContactTracker::new();
        tracker.process_samples(&[sample(3, 0, 0, true, false)]).expect("hover");
        let mut cancel = sample(3, 0, 0, false, false);
        cancel.canceled = true;
        let pdu = tracker.process_samples(&[cancel]).expect("cancel");
        assert_eq!(
            pdu.frames[0].contacts[0].contact_flags,
            TouchContactFlags::UPDATE | TouchContactFlags::CANCELED
        );
    }

    #[test]
    fn untracked_cancel_is_ignored() {
        let mut tracker = TouchContactTracker::new();
        let mut cancel = sample(9, 0, 0, false, false);
        cancel.canceled = true;
        assert!(tracker.process_samples(&[cancel]).is_none());
    }

    #[test]
    fn release_all_distinguishes_hover_and_engaged() {
        let mut tracker = TouchContactTracker::new();
        tracker.process_samples(&[sample(1, 1, 1, true, false)]).expect("hover");
        tracker.process_samples(&[sample(2, 2, 2, true, true)]).expect("down");
        let pdu = tracker.release_all().expect("release");
        let flags: Vec<_> = pdu.frames[0].contacts.iter().map(|c| c.contact_flags).collect();
        assert!(flags.contains(&TouchContactFlags::UPDATE));
        assert!(flags.contains(&TouchContactFlags::UP));
    }

    #[test]
    fn preserve_position_keeps_last_coords() {
        let mut tracker = TouchContactTracker::new();
        tracker.process_samples(&[sample(1, 42, 24, true, true)]).expect("down");
        let mut outside = sample(1, 0, 0, false, false);
        outside.preserve_position = true;
        let up = tracker.process_samples(&[outside]).expect("up");
        assert_eq!(up.frames[0].contacts[0].x, 42);
        assert_eq!(up.frames[0].contacts[0].y, 24);
    }

    #[test]
    fn orientation_conversion_win32_to_rdpei() {
        assert_eq!(TouchContactTracker::win32_orientation_to_rdpei(0), 90);
        assert_eq!(TouchContactTracker::win32_orientation_to_rdpei(90), 0);
        assert_eq!(TouchContactTracker::win32_orientation_to_rdpei(180), 270);
        assert_eq!(TouchContactTracker::win32_orientation_to_rdpei(270), 180);
    }

    #[test]
    fn every_emitted_flag_set_is_legal() {
        let cases = [
            sample(1, 0, 0, true, true),
            sample(1, 1, 1, true, true),
            sample(1, 1, 1, true, false),
            sample(1, 1, 1, false, false),
        ];
        let mut tracker = TouchContactTracker::new();
        for s in cases {
            if let Some(pdu) = tracker.process_samples(&[s]) {
                for c in &pdu.frames[0].contacts {
                    assert!(c.contact_flags.is_legal(), "{:?}", c.contact_flags);
                }
            }
        }
    }
}
