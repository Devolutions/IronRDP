use ironrdp_rdpei::contact::{ContactSample, ContactState, TouchFrameBuilder, TouchPhase, is_steady, transition};
use ironrdp_rdpei::pdu::{PenContactFlags, TouchContactFlags, TouchEventPdu};
use proptest::prelude::*;

const DOWN: TouchContactFlags = TouchContactFlags::DOWN
    .union(TouchContactFlags::INRANGE)
    .union(TouchContactFlags::INCONTACT);
const MOVE: TouchContactFlags = TouchContactFlags::UPDATE
    .union(TouchContactFlags::INRANGE)
    .union(TouchContactFlags::INCONTACT);

fn sample(in_range: bool, in_contact: bool, canceled: bool) -> ContactSample {
    ContactSample {
        in_range,
        in_contact,
        canceled,
    }
}

/// `(contact id, flags)` for every contact of every frame.
fn frames(event: &TouchEventPdu) -> Vec<Vec<(u8, TouchContactFlags)>> {
    event
        .frames
        .iter()
        .map(|frame| {
            let mut contacts: Vec<_> = frame
                .contacts
                .iter()
                .map(|contact| (contact.contact_id, contact.contact_flags))
                .collect();
            contacts.sort_by_key(|(contact_id, _)| *contact_id);
            contacts
        })
        .collect()
}

// oracle: spec MS-RDPEI §3.1.1.1
#[test]
fn hover_engage_and_leave_follow_the_contact_lifetime() {
    let hover = transition(None, sample(true, false, false)).expect("hover");
    assert_eq!(hover.flags, TouchContactFlags::UPDATE | TouchContactFlags::INRANGE);
    assert_eq!(hover.next, Some(ContactState::Hovering));

    let down = transition(hover.next, sample(true, true, false)).expect("down");
    assert_eq!(down.flags, DOWN);

    let lift = transition(down.next, sample(true, false, false)).expect("lift");
    assert_eq!(lift.flags, TouchContactFlags::UP | TouchContactFlags::INRANGE);
    assert_eq!(lift.next, Some(ContactState::Hovering));

    let leave = transition(lift.next, sample(false, false, false)).expect("leave");
    assert_eq!(leave.flags, TouchContactFlags::UPDATE);
    assert_eq!(leave.next, None);
}

// oracle: spec MS-RDPEI §3.1.1.1
#[test]
fn cancel_of_an_unknown_contact_sends_nothing() {
    assert_eq!(transition(None, sample(false, false, true)), None);
    assert_eq!(transition(None, sample(false, false, false)), None);
}

// oracle: spec MS-RDPEI §2.2.3.3.1.1
proptest! {
    #[test]
    fn every_transition_emits_a_legal_flag_set(
        samples in proptest::collection::vec((any::<bool>(), any::<bool>(), any::<bool>()), 1..32),
    ) {
        let mut state = None;
        for (in_range, in_contact, canceled) in samples {
            if let Some(next) = transition(state, sample(in_range, in_contact, canceled)) {
                prop_assert!(next.flags.is_legal(), "{:?}", next.flags);
                prop_assert!(next.pen_flags().is_legal());
                prop_assert_eq!(next.keeps_position(), state == Some(ContactState::Engaged) && next.next != Some(ContactState::Engaged));
                state = next.next;
            }
        }
    }

    #[test]
    fn frames_list_each_contact_once_with_legal_flags(
        events in proptest::collection::vec(
            (0u64..4, prop_oneof![
                Just(TouchPhase::Started),
                Just(TouchPhase::Moved),
                Just(TouchPhase::Ended),
                Just(TouchPhase::Canceled),
            ], any::<bool>()),
            1..48,
        ),
    ) {
        let mut builder = TouchFrameBuilder::new(3);
        let mut timestamp = 0;
        for (id, phase, flush) in events {
            timestamp += 1_000;
            builder.push(id, phase, 0, 0, None, timestamp);
            if flush && let Some(event) = builder.take_event(timestamp) {
                for frame in &event.frames {
                    prop_assert!(frame.contacts.len() <= 3);
                    let mut ids: Vec<_> = frame.contacts.iter().map(|contact| contact.contact_id).collect();
                    ids.sort_unstable();
                    ids.dedup();
                    prop_assert_eq!(ids.len(), frame.contacts.len());
                    for contact in &frame.contacts {
                        prop_assert!(contact.contact_flags.is_legal(), "{:?}", contact.contact_flags);
                    }
                }
            }
        }
    }
}

// oracle: reference FreeRDP/FreeRDP@16a7c2ec2fd0:channels/rdpei/client/rdpei_main.c (rdpei_add_frame)
#[test]
fn events_between_takes_form_one_frame_that_restates_unchanged_contacts() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(100, TouchPhase::Started, 10, 10, None, 1_000);
    builder.push(200, TouchPhase::Started, 20, 20, None, 1_200);
    let event = builder.take_event(3_000).expect("two downs");
    assert_eq!(frames(&event), [vec![(0, DOWN), (1, DOWN)]]);
    assert_eq!(event.encode_time, 2);

    builder.push(200, TouchPhase::Moved, 25, 25, None, 9_000);
    let event = builder.take_event(9_000).expect("one move");
    assert_eq!(frames(&event), [vec![(0, MOVE), (1, MOVE)]]);
    let unchanged = event.frames[0]
        .contacts
        .iter()
        .find(|contact| contact.contact_id == 0)
        .expect("restated contact");
    assert_eq!((unchanged.x, unchanged.y), (10, 10));

    builder.push(100, TouchPhase::Ended, 10, 10, None, 10_000);
    builder.push(200, TouchPhase::Canceled, 25, 25, None, 10_000);
    let event = builder.take_event(10_000).expect("up and cancel");
    assert_eq!(
        frames(&event),
        [vec![
            (0, TouchContactFlags::UP),
            (1, TouchContactFlags::UP | TouchContactFlags::CANCELED)
        ]]
    );
    builder.cancel_all(11_000);
    assert!(builder.take_event(11_000).is_none());
}

// oracle: spec MS-RDPEI §3.1.1.1
#[test]
fn a_lift_is_sent_at_the_last_engaged_position() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(1, TouchPhase::Started, 10, 20, None, 0);
    builder.push(2, TouchPhase::Started, 50, 60, None, 0);
    builder.take_event(0).expect("downs");
    builder.push(1, TouchPhase::Ended, 15, 25, None, 1_000);
    builder.push(2, TouchPhase::Canceled, 55, 65, None, 1_000);
    let event = builder.take_event(1_000).expect("lifts");
    let mut positions: Vec<_> = event.frames[0]
        .contacts
        .iter()
        .map(|contact| (contact.contact_id, contact.x, contact.y))
        .collect();
    positions.sort_unstable();
    assert_eq!(positions, [(0, 10, 20), (1, 50, 60)]);
}

// oracle: spec MS-RDPEI §3.1.1.1
#[test]
fn only_updates_that_keep_the_state_are_steady() {
    assert!(is_steady(MOVE));
    assert!(is_steady(TouchContactFlags::UPDATE | TouchContactFlags::INRANGE));
    assert!(!is_steady(DOWN));
    assert!(!is_steady(TouchContactFlags::UP | TouchContactFlags::INRANGE));
    assert!(!is_steady(TouchContactFlags::UPDATE));
    assert!(!is_steady(TouchContactFlags::UPDATE | TouchContactFlags::CANCELED));
}

// oracle: spec MS-RDPEI §2.2.3.3.1
#[test]
fn a_contact_changing_twice_starts_a_frame_offset_by_the_event_times() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(7, TouchPhase::Started, 0, 0, None, 5_000);
    builder.push(7, TouchPhase::Moved, 1, 0, None, 13_000);
    builder.push(7, TouchPhase::Moved, 2, 0, None, 20_000);
    let event = builder.take_event(20_000).expect("frames");
    let offsets: Vec<_> = event.frames.iter().map(|frame| frame.frame_offset).collect();
    assert_eq!(offsets, [0, 8_000, 7_000]);
    assert_eq!(frames(&event), [vec![(0, DOWN)], vec![(0, MOVE)], vec![(0, MOVE)]]);
}

// oracle: spec MS-RDPEI §2.2.3.2
#[test]
fn contacts_beyond_the_advertised_maximum_are_dropped() {
    let mut builder = TouchFrameBuilder::new(2);
    for id in 0..3 {
        builder.push(id, TouchPhase::Started, 0, 0, None, 0);
    }
    let event = builder.take_event(0).expect("downs");
    assert_eq!(event.frames[0].contacts.len(), 2);
}

// oracle: mutant a move of an unknown contact entering as a hover update, which toolkits report for hovering pens
#[test]
fn moves_of_unknown_contacts_are_ignored() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(1, TouchPhase::Moved, 0, 0, None, 0);
    builder.push(1, TouchPhase::Ended, 0, 0, None, 0);
    assert!(builder.take_event(0).is_none());
}

// oracle: spec MS-RDPEI §3.1.1.1
#[test]
fn cancel_all_cancels_at_the_last_position() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(1, TouchPhase::Started, 4, 5, Some(2_000), 0);
    builder.take_event(0).expect("down");
    builder.cancel_all(1_000);
    assert!(builder.has_pending_frames());
    let event = builder.take_event(1_000).expect("cancel");
    let contact = &event.frames[0].contacts[0];
    assert_eq!(
        contact.contact_flags,
        TouchContactFlags::UP | TouchContactFlags::CANCELED
    );
    assert_eq!((contact.x, contact.y), (4, 5));
    builder.cancel_all(2_000);
    assert!(!builder.has_pending_frames());
    assert!(builder.take_event(2_000).is_none());
}

// oracle: spec MS-RDPEI §2.2.3.3.1.1
#[test]
fn pressure_is_clamped_to_1024() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(1, TouchPhase::Started, 0, 0, Some(2_000), 0);
    let event = builder.take_event(0).expect("down");
    assert_eq!(event.frames[0].contacts[0].fields.pressure, Some(1024));
}

// oracle: spec MS-RDPEI §2.2.3.3.1.1
#[test]
fn exactly_the_eight_spec_flag_sets_are_legal() {
    use TouchContactFlags as F;

    let legal = [
        F::UP,
        F::UP | F::CANCELED,
        F::UPDATE,
        F::UPDATE | F::CANCELED,
        F::DOWN | F::INRANGE | F::INCONTACT,
        F::UPDATE | F::INRANGE | F::INCONTACT,
        F::UP | F::INRANGE,
        F::UPDATE | F::INRANGE,
    ];
    for bits in 0..0x40 {
        let flags = F::from_bits_truncate(bits);
        assert_eq!(flags.is_legal(), legal.contains(&flags), "{flags:?}");
        assert_eq!(
            PenContactFlags::from_bits_truncate(bits).is_legal(),
            legal.contains(&flags),
            "{flags:?}"
        );
    }
}

// oracle: spec MS-RDPEI §2.2.3.3.1
#[test]
fn a_new_transaction_starts_at_frame_offset_zero() {
    let mut builder = TouchFrameBuilder::new(10);
    builder.push(1, TouchPhase::Started, 0, 0, None, 1_000);
    builder.push(1, TouchPhase::Ended, 0, 0, None, 2_000);
    builder.take_event(2_000).expect("tap");
    builder.push(2, TouchPhase::Started, 0, 0, None, 90_000_000);
    let event = builder.take_event(90_000_000).expect("next tap");
    assert_eq!(event.frames[0].frame_offset, 0);
}
