use core::time::Duration;
use std::time::Instant;

use ironrdp_client::telemetry::{ConnectionTelemetry, Observation};
use ironrdp_rdpet::pdu::TelemetryPdu;
use proptest::prelude::*;

const GRAPHICS_OPEN: Observation = Observation {
    graphics_channel_open: true,
    graphics_active: false,
    telemetry_channel_open: false,
};

const GRAPHICS_ACTIVE: Observation = Observation {
    graphics_channel_open: true,
    graphics_active: true,
    telemetry_channel_open: false,
};

const AWAITING_GRAPHICS: Observation = Observation {
    graphics_channel_open: true,
    graphics_active: false,
    telemetry_channel_open: true,
};

const ALL_OPEN: Observation = Observation {
    graphics_channel_open: true,
    graphics_active: true,
    telemetry_channel_open: true,
};

fn after(started: Instant, millis: u64) -> Instant {
    started + Duration::from_millis(millis)
}

// oracle: spec MS-RDPET §2.2.1
#[test]
fn report_times_milestones_from_connection_start() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);

    assert_eq!(telemetry.step(GRAPHICS_OPEN, after(started, 5), true), None);
    assert_eq!(telemetry.step(ALL_OPEN, after(started, 12), true), None);
    let report = telemetry.step(ALL_OPEN, after(started, 30), true);

    assert_eq!(
        report,
        Some(TelemetryPdu {
            prompt_for_credentials_ms: 0,
            prompt_for_credentials_done_ms: 0,
            graphics_channel_opened_ms: 5,
            first_graphics_received_ms: 12,
        })
    );
}

// oracle: spec MS-RDPEDYC §3.2.3.2.1
#[test]
fn report_waits_for_iteration_after_telemetry_channel_opens() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);
    assert_eq!(telemetry.step(GRAPHICS_ACTIVE, after(started, 10), true), None);

    // This iteration answers the telemetry channel's Create Request, so it carries no report.
    assert_eq!(telemetry.step(ALL_OPEN, after(started, 20), true), None);

    let report = telemetry
        .step(ALL_OPEN, after(started, 30), true)
        .expect("report after the channel opened");
    assert_eq!(report.graphics_channel_opened_ms, 10);
    assert_eq!(report.first_graphics_received_ms, 10);
}

// oracle: mutant sending the report in an iteration that already carries another dynamic channel batch
#[test]
fn report_waits_for_iteration_without_other_batch() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);
    telemetry.step(ALL_OPEN, after(started, 10), true);

    assert_eq!(telemetry.step(ALL_OPEN, after(started, 20), false), None);

    assert!(telemetry.step(ALL_OPEN, after(started, 30), true).is_some());
}

// oracle: spec MS-RDPET §1.3
#[test]
fn report_is_sent_once_per_connection() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);
    telemetry.step(ALL_OPEN, after(started, 10), true);

    assert!(telemetry.step(ALL_OPEN, after(started, 20), true).is_some());

    assert_eq!(telemetry.step(ALL_OPEN, after(started, 30), true), None);
}

// oracle: spec MS-RDPET §3.2.5.1
#[test]
fn no_report_before_first_graphics_message() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);

    telemetry.step(AWAITING_GRAPHICS, after(started, 10), true);

    assert_eq!(telemetry.step(AWAITING_GRAPHICS, after(started, 20), true), None);
}

fn observation() -> impl Strategy<Value = Observation> {
    (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(graphics_channel_open, active, telemetry_channel_open)| {
        Observation {
            graphics_channel_open,
            // The graphics pipeline is active only on its open channel.
            graphics_active: graphics_channel_open && active,
            telemetry_channel_open,
        }
    })
}

#[test]
fn report_is_due_exactly_once_its_preconditions_hold() {
    // oracle: property model (one report, in the first sending iteration after the telemetry channel was open
    // and the graphics milestones were reached)
    proptest!(|(steps in prop::collection::vec((observation(), 0u32..50, any::<bool>()), 0..40))| {
        let started = Instant::now();
        let mut telemetry = ConnectionTelemetry::new(started);
        let mut elapsed = 0u32;
        let mut graphics_channel_opened = None;
        let mut first_graphics_received = None;
        let mut telemetry_channel_was_open = false;
        let mut sent = false;

        for (observation, delta, can_send) in steps {
            elapsed += delta;
            let report = telemetry.step(observation, after(started, u64::from(elapsed)), can_send);

            let due = !sent && can_send && telemetry_channel_was_open && first_graphics_received.is_some();
            prop_assert_eq!(report.is_some(), due);
            if let Some(report) = report {
                sent = true;
                prop_assert_eq!(Some(report.graphics_channel_opened_ms), graphics_channel_opened);
                prop_assert_eq!(Some(report.first_graphics_received_ms), first_graphics_received);
            }

            if observation.graphics_channel_open {
                graphics_channel_opened.get_or_insert(elapsed);
            }
            if observation.graphics_active {
                first_graphics_received.get_or_insert(elapsed);
            }
            telemetry_channel_was_open = observation.telemetry_channel_open;
        }
    });
}
