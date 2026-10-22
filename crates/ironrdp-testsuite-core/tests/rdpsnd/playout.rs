use core::time::Duration;

use ironrdp_rdpsnd::playout::{Period, PlayoutController, stretch};
use rstest::rstest;

const RATE: u32 = 48_000;
/// A 10 ms device period at 48 kHz.
const PERIOD: usize = 480;

fn controller() -> PlayoutController {
    PlayoutController::new(RATE, Duration::from_millis(100))
}

/// Feeds `periods` device periods at a constant buffer level and returns the last decision.
fn settle(controller: &mut PlayoutController, level: usize, periods: usize) -> Period {
    let mut period = Period::Silence;
    for _ in 0..periods {
        period = controller.next_period(level, PERIOD);
    }
    period
}

#[test]
fn target_is_converted_to_frames() {
    assert_eq!(controller().target(), 4_800);
}

#[test]
fn buffers_until_the_target_is_reached() {
    let mut controller = controller();

    assert_eq!(controller.next_period(4_799, PERIOD), Period::Silence);
    assert!(controller.is_buffering());
    assert_eq!(
        controller.next_period(4_800, PERIOD),
        Period::Play {
            skip: 0,
            consume: PERIOD
        }
    );
    assert!(!controller.is_buffering());
}

#[test]
fn underrun_plays_what_is_left_then_rebuffers() {
    let mut controller = controller();
    controller.next_period(4_800, PERIOD);

    assert_eq!(controller.next_period(100, PERIOD), Period::Underrun { consume: 100 });
    assert!(controller.is_buffering());
    // Playback resumes only once the target is buffered again.
    assert_eq!(controller.next_period(2_000, PERIOD), Period::Silence);
    assert!(matches!(controller.next_period(4_800, PERIOD), Period::Play { .. }));
}

#[test]
fn steady_level_within_tolerance_plays_one_period() {
    let mut controller = controller();

    // 25 ms of tolerance around the 100 ms target: 108 ms stays uncorrected.
    let period = settle(&mut controller, 5_200, 200);

    assert_eq!(
        period,
        Period::Play {
            skip: 0,
            consume: PERIOD
        }
    );
}

#[rstest]
#[case::server_clock_faster(6_400, PERIOD + 2)]
#[case::server_clock_slower(3_200, PERIOD - 2)]
fn persistent_drift_is_corrected_by_half_a_percent(#[case] level: usize, #[case] consume: usize) {
    let mut controller = controller();
    controller.next_period(4_800, PERIOD);

    let period = settle(&mut controller, level, 200);

    assert_eq!(period, Period::Play { skip: 0, consume });
}

#[test]
fn short_burst_is_smoothed_out() {
    let mut controller = controller();
    settle(&mut controller, 4_800, 50);

    // One period well above the band moves the smoothed level by only a sixteenth.
    assert_eq!(
        controller.next_period(7_200, PERIOD),
        Period::Play {
            skip: 0,
            consume: PERIOD
        }
    );
}

#[test]
fn large_backlog_is_discarded_down_to_the_target() {
    let mut controller = controller();
    controller.next_period(4_800, PERIOD);

    // 400 ms buffered is beyond target + tolerance + 200 ms.
    let period = controller.next_period(19_200, PERIOD);

    assert_eq!(
        period,
        Period::Play {
            skip: 19_200 - PERIOD - 4_800,
            consume: PERIOD
        }
    );
    // Afterwards the controller treats the buffer as settled at the target.
    assert_eq!(
        controller.next_period(4_800, PERIOD),
        Period::Play {
            skip: 0,
            consume: PERIOD
        }
    );
}

#[test]
fn backlog_present_at_start_is_played_and_drained() {
    let mut controller = controller();

    // 350 ms accumulated while the device was opening: nothing is discarded.
    assert_eq!(
        controller.next_period(16_800, PERIOD),
        Period::Play {
            skip: 0,
            consume: PERIOD + 2
        }
    );
    // Drift correction drains it while the level stays high.
    assert_eq!(
        settle(&mut controller, 16_000, 100),
        Period::Play {
            skip: 0,
            consume: PERIOD + 2
        }
    );
    // A burst on top of the draining backlog is still discarded.
    assert!(matches!(
        controller.next_period(40_000, PERIOD),
        Period::Play { skip, .. } if skip == 40_000 - PERIOD - 4_800
    ));
}

#[test]
fn stretch_copies_equal_lengths() {
    let input = [1, -1, 2, -2, 3, -3];
    let mut output = [0; 6];

    stretch(&input, 2, &mut output);

    assert_eq!(output, input);
}

#[test]
fn stretch_interpolates_extra_frames() {
    let input = [0, 100, 200];
    let mut output = [0; 5];

    stretch(&input, 1, &mut output);

    assert_eq!(output, [0, 50, 100, 150, 200]);
}

#[test]
fn stretch_drops_frames_evenly() {
    let input = [0, 10, 20, 30, 40, 50, 60];
    let mut output = [0; 4];

    stretch(&input, 1, &mut output);

    assert_eq!(output, [0, 20, 40, 60]);
}

#[test]
fn stretch_keeps_channels_apart() {
    let input = [0, 1000, 100, 900];
    let mut output = [0; 6];

    stretch(&input, 2, &mut output);

    assert_eq!(output, [0, 1000, 50, 950, 100, 900]);
}

#[test]
fn stretch_of_nothing_is_silence() {
    let mut output = [7; 4];

    stretch(&[], 2, &mut output);

    assert_eq!(output, [0; 4]);
}
