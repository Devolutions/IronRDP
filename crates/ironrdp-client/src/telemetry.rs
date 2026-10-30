//! Connection telemetry ([MS-RDPET]).
//!
//! The report is sent once per connection, after both graphics pipeline milestones are observed.
//! Capability exchange precedes any other graphics pipeline message ([MS-RDPEGFX] section 1.7),
//! so the capabilities confirmation, which makes the pipeline active, marks the first graphics message.
//! A connection without the graphics pipeline sends no report, which the protocol allows ([MS-RDPET] section 3.2.5.1).
//! The client shows no credentials prompt, so the credentials milestones are zero.
//!
//! [MS-RDPET]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/

use std::time::Instant;

use ironrdp_dvc::DvcMessageBatch;
use ironrdp_egfx::client::GraphicsPipelineClient;
use ironrdp_rdpet::client::TelemetryClient;
use ironrdp_rdpet::pdu::TelemetryPdu;
use ironrdp_session::ActiveStage;
use tracing::{debug, warn};

/// Channel state left by one session loop iteration.
#[derive(Debug, Clone, Copy, Default)]
pub struct Observation {
    /// The graphics pipeline channel is open.
    pub graphics_channel_open: bool,
    /// The graphics pipeline confirmed its capabilities.
    pub graphics_active: bool,
    /// The telemetry channel is open.
    pub telemetry_channel_open: bool,
}

impl Observation {
    /// Reads the channel state of `active_stage`.
    // UNTESTED: a capabilities confirmation counts even when its capability set does not parse.
    // The graphics pipeline still becomes active then, so `is_active` is the signal, not `negotiated_capabilities`.
    pub fn of(active_stage: &ActiveStage) -> Self {
        let graphics = active_stage.get_dvc::<GraphicsPipelineClient>();
        Self {
            graphics_channel_open: graphics.is_some(),
            graphics_active: graphics.is_some_and(|graphics| graphics.processor().is_active()),
            telemetry_channel_open: active_stage.get_dvc::<TelemetryClient>().is_some(),
        }
    }
}

/// Connection milestones of one connection, and whether its report was sent.
#[derive(Debug)]
pub struct ConnectionTelemetry {
    started: Instant,
    graphics_channel_opened: Option<Instant>,
    first_graphics_received: Option<Instant>,
    telemetry_channel_open: bool,
    done: bool,
}

impl ConnectionTelemetry {
    /// Starts tracking a connection initiated at `started`.
    pub fn new(started: Instant) -> Self {
        Self {
            started,
            graphics_channel_opened: None,
            first_graphics_received: None,
            telemetry_channel_open: false,
            done: false,
        }
    }

    /// Records the channel state an iteration ending at `now` left.
    ///
    /// Call at the end of each session loop iteration, after [`Self::take_report`].
    pub fn observe(&mut self, observation: Observation, now: Instant) {
        if observation.graphics_channel_open {
            self.graphics_channel_opened.get_or_insert(now);
        }
        if observation.graphics_active {
            self.first_graphics_received.get_or_insert(now);
        }
        self.telemetry_channel_open = observation.telemetry_channel_open;
    }

    /// Returns the report once both milestones are recorded and the telemetry channel is open.
    ///
    /// Only earlier observations count, so the iteration that opens the telemetry channel never carries the report:
    /// the report would otherwise precede that iteration's DRDYNVC Create Response, and the server would drop it.
    /// At most one report is returned per connection, because servers log each report they receive.
    pub fn take_report(&mut self) -> Option<TelemetryPdu> {
        if self.done || !self.telemetry_channel_open {
            return None;
        }
        let first_graphics_received = self.first_graphics_received?;
        self.done = true;

        Some(TelemetryPdu {
            prompt_for_credentials_ms: 0,
            prompt_for_credentials_done_ms: 0,
            graphics_channel_opened_ms: self.millis_since_start(self.graphics_channel_opened),
            first_graphics_received_ms: self.millis_since_start(Some(first_graphics_received)),
        })
    }

    /// Encodes the report from [`Self::take_report`] for the telemetry channel of `active_stage`.
    pub fn report(&mut self, active_stage: &ActiveStage) -> Option<DvcMessageBatch> {
        let report = self.take_report()?;
        let Some(telemetry) = active_stage.get_dvc::<TelemetryClient>() else {
            warn!("Telemetry channel closed before the connection telemetry was sent");
            return None;
        };
        let channel_id = telemetry.channel_id();

        match telemetry
            .processor()
            .send_report(report)
            .and_then(|messages| DvcMessageBatch::try_new(channel_id, messages))
        {
            Ok(batch) => {
                debug!(?report, "Sending connection telemetry");
                Some(batch)
            }
            Err(error) => {
                warn!(%error, "Unable to send connection telemetry");
                None
            }
        }
    }

    fn millis_since_start(&self, milestone: Option<Instant>) -> u32 {
        milestone.map_or(0, |milestone| {
            u32::try_from(milestone.saturating_duration_since(self.started).as_millis()).unwrap_or(u32::MAX)
        })
    }
}
