use core::time::Duration;
use std::time::Instant;

use ironrdp_client::telemetry::{ConnectionTelemetry, Observation};
use ironrdp_rdpet::pdu::TelemetryPdu;

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

const ALL_OPEN: Observation = Observation {
    graphics_channel_open: true,
    graphics_active: true,
    telemetry_channel_open: true,
};

fn after(started: Instant, millis: u64) -> Instant {
    started + Duration::from_millis(millis)
}

/// Runs one session loop iteration: the report is taken before the iteration's state is observed.
fn iteration(telemetry: &mut ConnectionTelemetry, observation: Observation, now: Instant) -> Option<TelemetryPdu> {
    let report = telemetry.take_report();
    telemetry.observe(observation, now);
    report
}

// oracle: spec MS-RDPET §2.2.1
#[test]
fn report_times_milestones_from_connection_start() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);

    assert_eq!(iteration(&mut telemetry, GRAPHICS_OPEN, after(started, 5)), None);
    assert_eq!(iteration(&mut telemetry, ALL_OPEN, after(started, 12)), None);
    let report = iteration(&mut telemetry, ALL_OPEN, after(started, 30));

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
    assert_eq!(iteration(&mut telemetry, GRAPHICS_ACTIVE, after(started, 10)), None);

    // This iteration answers the telemetry channel's Create Request, so it carries no report.
    assert_eq!(iteration(&mut telemetry, ALL_OPEN, after(started, 20)), None);

    let report = iteration(&mut telemetry, ALL_OPEN, after(started, 30)).expect("report after the channel opened");
    assert_eq!(report.graphics_channel_opened_ms, 10);
    assert_eq!(report.first_graphics_received_ms, 10);
}

#[test]
fn report_is_sent_once_per_connection() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);
    iteration(&mut telemetry, ALL_OPEN, after(started, 10));

    assert!(iteration(&mut telemetry, ALL_OPEN, after(started, 20)).is_some());

    assert_eq!(iteration(&mut telemetry, ALL_OPEN, after(started, 30)), None);
}

// oracle: spec MS-RDPET §3.2.5.1
#[test]
fn no_report_before_first_graphics_message() {
    let started = Instant::now();
    let mut telemetry = ConnectionTelemetry::new(started);
    let awaiting_graphics = Observation {
        graphics_channel_open: true,
        graphics_active: false,
        telemetry_channel_open: true,
    };

    iteration(&mut telemetry, awaiting_graphics, after(started, 10));

    assert_eq!(iteration(&mut telemetry, awaiting_graphics, after(started, 20)), None);
}
