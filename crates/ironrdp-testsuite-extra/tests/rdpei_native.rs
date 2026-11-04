use ironrdp_rdpei::pdu::{
    CsReadyFlags, CsReadyPdu, PenContact, PenContactFlags, PenEventPdu, RdpInputProtocolVersion, TouchContactFlags,
    TouchEventPdu,
};
use ironrdp_rdpei_native::__test::{
    frame_timestamps, pen_departures_first, pointer_flags, touch_contact_rect, touch_orientation,
};
use ironrdp_rdpei_native::WindowsInput;
use rstest::rstest;
use windows::Win32::UI::Input::Pointer::{
    POINTER_FLAG_CANCELED, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP,
    POINTER_FLAG_UPDATE,
};

#[rstest]
#[case::up(0, 270)]
#[case::left(90, 180)]
#[case::down(180, 90)]
#[case::right(270, 0)]
#[case::last_degree(359, 271)]
fn orientation_uses_windows_axes(#[case] rdpei: u32, #[case] native: u32) -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI 2.2.3.3.1.1 measures counter-clockwise from up.
    // POINTER_TOUCH_INFO measures clockwise from right.
    // https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_touch_info
    assert_eq!(touch_orientation(Some(rdpei))?, native);
    Ok(())
}

#[test]
fn invalid_orientation_is_rejected() {
    // oracle: spec MS-RDPEI 2.2.3.3.1.1 bounds orientation to 0..=359.
    assert!(touch_orientation(Some(360)).is_err());
    assert!(touch_orientation(Some(u32::MAX)).is_err());
}

#[test]
fn omitted_orientation_maps_rdpei_up_to_windows_up() -> anyhow::Result<()> {
    // oracle: MS-RDPEI 2.2.3.3.1.1 assumes zero degrees when orientation is absent.
    assert_eq!(touch_orientation(None)?, 270);
    Ok(())
}

#[test]
fn contact_rectangle_is_rotated_around_the_contact_point() -> anyhow::Result<()> {
    // oracle: MS-RDPEI 2.2.3.3.1.1 rotates the relative contact rectangle counter-clockwise.
    assert_eq!(
        touch_contact_rect(100, 200, -2, -1, 2, 1, Some(90))?,
        (99, 198, 101, 202)
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
    // oracle: spec MS-RDPEI 2.2.3.3.1.1 and Windows POINTER_FLAGS define these corresponding states.
    // https://learn.microsoft.com/en-us/windows/win32/inputmsg/pointer-flags-contants
    assert_eq!(pointer_flags(rdpei), native);
}

#[test]
fn frames_preserve_microsecond_offsets_and_millisecond_age() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI 2.2.3.3 and 2.2.3.3.1 define encodeTime in ms and frameOffset in us.
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
    // oracle: MS-RDPEI 2.2.3.3.1 frameOffset is measured from the preceding frame.
    // TouchFrameBuilder carries that timestamp across separately flushed events.
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
    // oracle: spec InjectTouchInput forbids future timestamps.
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
    // oracle: spec InjectTouchInput requires at least 0.1 ms between performance-counter timestamps.
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

#[rstest]
#[case::nonzero_first(20, vec![1])]
#[case::overflow(20, vec![0, u64::MAX, 1])]
#[case::before_clock_origin(2000, vec![0])]
#[case::empty(0, vec![])]
fn impossible_timestamps_are_rejected(#[case] age: u32, #[case] offsets: Vec<u64>) {
    // oracle: spec MS-RDPEI 2.2.3.3.1 requires a zero first offset.
    // Native timestamps must fit the performance counter and events must contain frames.
    assert!(frame_timestamps(10_000_000, 10_000_000, 0, false, CsReadyFlags::empty(), age, &offsets).is_err());
}

#[test]
fn pen_departures_are_processed_before_new_devices() {
    // oracle: MS-RDPEI 2.2.3.7.1.1 contact flags identify contacts leaving the digitizer range.
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
fn disabled_timestamps_ignore_both_timing_fields() -> anyhow::Result<()> {
    // oracle: spec MS-RDPEI 2.2.3.2 requires ignoring encodeTime and frameOffset when disabled.
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
async fn ownership_is_exclusive_and_uninitialized_input_is_rejected() -> anyhow::Result<()> {
    // oracle: mutant permitting simultaneous injectors or retaining ownership after drop.
    // No OS injection is performed by construction, reset, or uninitialized calls.
    let mut input = WindowsInput::new()?;
    assert!(WindowsInput::new().is_err());
    assert!(
        input
            .touch(TouchEventPdu {
                encode_time: 0,
                frames: vec![]
            })
            .await
            .is_err()
    );
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
    input.reset();
    assert!(WindowsInput::new().is_err());
    input.initialize(CsReadyPdu::new(CsReadyFlags::empty(), RdpInputProtocolVersion::V100, 0))?;
    input.reset();
    assert!(input.dismiss(0).await.is_err());
    drop(input);
    let _next = WindowsInput::new()?;
    Ok(())
}
