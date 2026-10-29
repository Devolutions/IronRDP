//! Connection telemetry ([MS-RDPET]).
//!
//! The report is sent once per connection, after both graphics pipeline milestones are observed.
//! Capability exchange precedes any other graphics pipeline message ([MS-RDPEGFX] section 1.7),
//! so the confirmed capabilities mark the first graphics message.
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

/// Connection milestones of one connection, and whether its report was sent.
pub(crate) struct ConnectionTelemetry {
    started: Instant,
    graphics_channel_opened: Option<Instant>,
    first_graphics_received: Option<Instant>,
    done: bool,
}

impl ConnectionTelemetry {
    /// Starts tracking a connection initiated at `started`.
    pub(crate) fn new(started: Instant) -> Self {
        Self {
            started,
            graphics_channel_opened: None,
            first_graphics_received: None,
            done: false,
        }
    }

    /// Records the graphics pipeline milestones reached so far.
    ///
    /// Call after each session loop iteration; a milestone is timed at the end of the iteration that reached it.
    pub(crate) fn observe(&mut self, active_stage: &ActiveStage) {
        if self.first_graphics_received.is_some() {
            return;
        }
        let Some(graphics) = active_stage.get_dvc::<GraphicsPipelineClient>() else {
            return;
        };
        let now = Instant::now();
        self.graphics_channel_opened.get_or_insert(now);
        if graphics.processor().negotiated_capabilities().is_some() {
            self.first_graphics_received = Some(now);
        }
    }

    /// Returns the report once both milestones are recorded and the telemetry channel is open.
    // UNTESTED: at most one report is returned per connection, and only once both graphics milestones are known.
    // Servers log each report they receive, so a change here must keep `done` set after the first report or failure.
    pub(crate) fn report(&mut self, active_stage: &ActiveStage) -> Option<DvcMessageBatch> {
        if self.done || self.first_graphics_received.is_none() {
            return None;
        }

        let telemetry = active_stage.get_dvc::<TelemetryClient>()?;
        let channel_id = telemetry.channel_id();
        let report = TelemetryPdu {
            prompt_for_credentials_ms: 0,
            prompt_for_credentials_done_ms: 0,
            graphics_channel_opened_ms: self.millis_since_start(self.graphics_channel_opened),
            first_graphics_received_ms: self.millis_since_start(self.first_graphics_received),
        };
        self.done = true;

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
