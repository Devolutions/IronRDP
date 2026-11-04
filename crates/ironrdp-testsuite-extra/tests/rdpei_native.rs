use ironrdp_rdpei::pdu::{
    CsReadyFlags, CsReadyPdu, PenContact, PenContactFlags, PenEventPdu, PenFlags, RdpInputProtocolVersion,
    TouchContactFlags, TouchEventPdu,
};
use ironrdp_rdpei_native::__test::{
    frame_timestamps, pen_departures_first, pen_flags, pointer_flags, touch_contact_rect, touch_orientation,
};
use ironrdp_rdpei_native::{WindowsInput, WindowsInputErrorKind};
use proptest::prelude::*;
use rstest::rstest;
use windows::Win32::UI::Input::Pointer::{
    POINTER_FLAG_CANCELED, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP,
    POINTER_FLAG_UPDATE,
};
use windows::Win32::UI::WindowsAndMessaging::{PEN_FLAG_ERASER, PEN_FLAG_INVERTED};

static INPUT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[rstest]
#[case::up(0, 270)]
#[case::left(90, 180)]
#[case::down(180, 90)]
#[case::right(270, 0)]
#[case::last_degree(359, 271)]
fn orientation_uses_windows_axes(#[case] rdpei: u32, #[case] native: u32) -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT.
    // The specification measures orientation counter-clockwise from up.
    // POINTER_TOUCH_INFO measures clockwise from right.
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_touch_info
    assert_eq!(touch_orientation(Some(rdpei))?, native);
    Ok(())
}

#[test]
fn invalid_orientation_is_rejected() {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT.
    // The specification bounds orientation to 0..=359.
    assert!(touch_orientation(Some(360)).is_err());
    assert!(touch_orientation(Some(u32::MAX)).is_err());
}

#[test]
fn orientation_mapping_covers_every_rdpei_degree() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT measures counter-clockwise from up.
    // POINTER_TOUCH_INFO measures clockwise from right.
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_touch_info
    for rdpei in 0..360 {
        let native = (270 + 360 - rdpei) % 360;
        assert_eq!(touch_orientation(Some(rdpei))?, native);
    }
    Ok(())
}

#[test]
fn omitted_orientation_maps_rdpei_up_to_windows_up() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT.
    // The specification assumes zero degrees when orientation is absent.
    assert_eq!(touch_orientation(None)?, 270);
    Ok(())
}

#[test]
fn contact_rectangle_is_rotated_around_the_contact_point() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT.
    // The specification rotates the relative contact rectangle counter-clockwise.
    assert_eq!(
        touch_contact_rect(100, 200, -2, -1, 2, 1, Some(90))?,
        (99, 198, 101, 202)
    );
    Ok(())
}

#[test]
fn zero_degree_contact_rectangle_keeps_its_relative_bounds() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT.
    // The specification rotates the relative rectangle by the given angle.
    assert_eq!(
        touch_contact_rect(100, 200, -2, -1, 2, 1, Some(0))?,
        (98, 199, 102, 201)
    );
    Ok(())
}

#[test]
fn diagonal_contact_rectangle_uses_enclosing_pixel_bounds() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §4.1.2 Touch Contact Oriented at 45 Degrees.
    assert_eq!(
        touch_contact_rect(100, 200, -2, -1, 2, 1, Some(45))?,
        (97, 197, 103, 203)
    );
    Ok(())
}

#[rstest]
#[case::down(TouchContactFlags::DOWN, POINTER_FLAG_DOWN.0)]
#[case::update(TouchContactFlags::UPDATE, POINTER_FLAG_UPDATE.0)]
#[case::up(TouchContactFlags::UP, POINTER_FLAG_UP.0)]
#[case::range(TouchContactFlags::INRANGE, POINTER_FLAG_INRANGE.0)]
#[case::contact(TouchContactFlags::INCONTACT, POINTER_FLAG_INCONTACT.0)]
#[case::cancel(TouchContactFlags::CANCELED, POINTER_FLAG_CANCELED.0)]
fn contact_flags_keep_their_meaning(#[case] rdpei: TouchContactFlags, #[case] native: u32) {
    // oracle: spec MS-RDPEI §2.2.3.3.1.1 RDPINPUT_TOUCH_CONTACT defines these contact states.
    // https://learn.microsoft.com/en-us/windows/win32/inputmsg/pointer-flags-contants
    assert_eq!(pointer_flags(rdpei), native);
}

#[test]
fn frames_preserve_microsecond_offsets_and_millisecond_age() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3 RDPINPUT_TOUCH_EVENT_PDU and §2.2.3.3.1 RDPINPUT_TOUCH_FRAME.
    assert_eq!(
        frame_timestamps(
            10_000_000,
            10_000_000,
            0,
            false,
            CsReadyFlags::empty(),
            20,
            &[0, 16_000]
        )?,
        [9_800_000, 9_960_000]
    );
    Ok(())
}

#[test]
fn continuation_events_accept_nonzero_first_frame_offset() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.3.1 RDPINPUT_TOUCH_FRAME measures frameOffset from the preceding frame.
    // oracle: reference crates/ironrdp-rdpei/src/contact.rs carries the timestamp across flushed events.
    assert_eq!(
        frame_timestamps(
            10_000_000,
            10_000_000,
            9_700_000,
            true,
            CsReadyFlags::empty(),
            20,
            &[16_000, 8_000]
        )?,
        [9_860_000, 9_940_000]
    );
    Ok(())
}

#[test]
fn rounded_encoding_age_does_not_push_the_last_frame_into_the_future() -> anyhow::Result<()> {
    // oracle: spec Windows API InjectTouchInput §Remarks forbids future timestamps.
    // A 16 ms interval with a rounded-down encoding age must still end no later than the current counter.
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-injecttouchinput
    assert_eq!(
        frame_timestamps(10_000_000, 10_000_000, 0, false, CsReadyFlags::empty(), 0, &[0, 16_000])?,
        [9_840_000, 10_000_000]
    );
    Ok(())
}

#[test]
fn successive_events_leave_a_native_clock_interval() -> anyhow::Result<()> {
    // oracle: spec Windows API InjectTouchInput §Remarks requires 0.1 ms between performance-counter timestamps.
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-injecttouchinput
    assert_eq!(
        frame_timestamps(
            10_000_000,
            10_000_000,
            10_000_000,
            false,
            CsReadyFlags::empty(),
            0,
            &[0]
        )?,
        [10_001_000]
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    // oracle: spec Windows API InjectTouchInput Remarks requires 0.1 ms between injected frames.
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-injecttouchinput
    // oracle: spec MS-RDPEI section 2.2.3.3.1 RDPINPUT_TOUCH_FRAME defines offsets in microseconds.
    // oracle: mutant frame timestamp scheduling accepts values beyond its 100 ms wait horizon.
    #[test]
    fn frame_timestamps_stay_in_order_and_within_the_native_clock_window(
        continuing in any::<bool>(),
        first_offset in 0u64..=10_000,
        later_offsets in proptest::collection::vec(0u64..=10_000, 0..=8),
        encode_time in 0u32..=100,
        previous in 0u64..=1_000_000_000,
    ) {
        const FREQUENCY: u64 = 10_000_000;
        const NOW: u64 = 1_000_000_000;
        let first_offset = if continuing { first_offset } else { 0 };
        let offsets = core::iter::once(first_offset).chain(later_offsets).collect::<Vec<_>>();
        let timestamps = frame_timestamps(
            FREQUENCY,
            NOW,
            previous,
            continuing,
            CsReadyFlags::empty(),
            encode_time,
            &offsets,
        )
        .expect("generated timestamp inputs fit the native clock");
        prop_assert_eq!(timestamps.len(), offsets.len());
        prop_assert!(timestamps.windows(2).all(|pair| pair[0] <= pair[1]));
        if previous != 0 {
            prop_assert!(timestamps[0] >= previous + FREQUENCY.div_ceil(10_000));
        }
        prop_assert!(timestamps.iter().all(|timestamp| *timestamp <= NOW + FREQUENCY / 10));
    }
}

#[test]
fn a_new_pointer_event_rejects_a_nonzero_first_offset() {
    // oracle: reference crates/ironrdp-rdpei/src/contact.rs resets offsets when a contact transaction ends.
    assert!(frame_timestamps(10_000_000, 10_000_000, 0, false, CsReadyFlags::empty(), 20, &[1]).is_err());
}

#[test]
fn timestamp_offset_overflow_is_rejected() {
    // oracle: mutant timestamp offset arithmetic wraps instead of rejecting an unrepresentable span.
    assert!(
        frame_timestamps(
            10_000_000,
            10_000_000,
            0,
            false,
            CsReadyFlags::empty(),
            20,
            &[0, u64::MAX, 1]
        )
        .is_err()
    );
}

#[test]
fn timestamps_before_the_clock_origin_are_rejected() {
    // oracle: mutant timestamp arithmetic accepts a value before the performance-counter origin.
    assert!(frame_timestamps(10_000_000, 10_000_000, 0, false, CsReadyFlags::empty(), 2000, &[0]).is_err());
}

#[test]
fn an_empty_pointer_event_is_rejected() {
    // oracle: mutant empty pointer events pass timestamp generation without a frame.
    assert!(frame_timestamps(10_000_000, 10_000_000, 0, false, CsReadyFlags::empty(), 0, &[]).is_err());
}

#[test]
fn pen_departures_are_processed_before_new_devices() {
    // oracle: mutant arrivals processed before departures cannot reuse a freed synthetic pen slot.
    let mut contacts = [
        PenContact::new(
            4,
            20,
            20,
            PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
        ),
        PenContact::new(0, 10, 10, PenContactFlags::UP),
    ];
    pen_departures_first(&mut contacts);
    assert_eq!(contacts[0].device_id, 0);
    assert_eq!(contacts[1].device_id, 4);
}

#[test]
fn eraser_button_does_not_change_the_pen_inversion_flag() {
    // oracle: spec MS-RDPEI §2.2.3.7.1.1 RDPINPUT_PEN_CONTACT.
    // Windows Pen Flags define separate eraser and inversion states.
    assert_eq!(pen_flags(PenFlags::ERASER_PRESSED), PEN_FLAG_ERASER);
    assert_eq!(pen_flags(PenFlags::INVERTED), PEN_FLAG_INVERTED);
    assert_eq!(
        pen_flags(PenFlags::ERASER_PRESSED | PenFlags::INVERTED),
        PEN_FLAG_ERASER | PEN_FLAG_INVERTED
    );
}

#[test]
fn disabled_timestamps_ignore_both_timing_fields() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI §2.2.3.2 RDPINPUT_CS_READY_PDU requires ignoring encodeTime and frameOffset when disabled.
    assert_eq!(
        frame_timestamps(
            0,
            0,
            u64::MAX,
            false,
            CsReadyFlags::DISABLE_TIMESTAMP_INJECTION,
            u32::MAX,
            &[u64::MAX, u64::MAX]
        )?,
        [0, 0]
    );
    Ok(())
}

#[tokio::test]
async fn ownership_is_exclusive_and_released_after_drop() -> anyhow::Result<()> {
    // oracle: mutant permitting simultaneous injectors or retaining ownership after drop.
    let _guard = INPUT_TEST_LOCK.lock().await;
    let mut input = WindowsInput::new()?;
    assert!(WindowsInput::new().is_err());
    input.reset();
    assert!(WindowsInput::new().is_err());
    drop(input);
    let _next = WindowsInput::new()?;
    Ok(())
}

#[tokio::test]
async fn uninitialized_input_operations_are_rejected() -> anyhow::Result<()> {
    // oracle: mutant accepting input operations before CS_READY initialization.
    let _guard = INPUT_TEST_LOCK.lock().await;
    let mut input = WindowsInput::new()?;
    assert!(matches!(
        input
            .touch(TouchEventPdu {
                encode_time: 0,
                frames: vec![],
            })
            .await,
        Err(error) if matches!(error.kind(), WindowsInputErrorKind::InvalidState)
    ));
    assert!(
        input
            .pen(PenEventPdu {
                encode_time: 0,
                frames: vec![]
            })
            .await
            .is_err()
    );
    assert!(input.dismiss(0).await.is_err());
    Ok(())
}

#[tokio::test]
async fn touch_initialization_can_be_repeated_after_reset() -> anyhow::Result<()> {
    // oracle: mutant reset leaves touch initialization unavailable after reset.
    let _guard = INPUT_TEST_LOCK.lock().await;
    let mut input = WindowsInput::new()?;
    let touch_ready = CsReadyPdu::new(CsReadyFlags::empty(), RdpInputProtocolVersion::V100, 1);
    input.initialize(touch_ready.clone())?;
    input.reset();
    input.initialize(touch_ready)?;
    input.reset();
    Ok(())
}
