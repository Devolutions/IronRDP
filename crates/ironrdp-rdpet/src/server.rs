//! Server processor for the MS-RDPET dynamic virtual channel.

use alloc::boxed::Box;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use tracing::warn;

use crate::CHANNEL_NAME;
use crate::pdu::TelemetryPdu;

/// Receives the connection reports a client sends on the telemetry channel.
pub trait TelemetryHandler: Send {
    /// Called for each well-formed report.
    ///
    /// [MS-RDPET] section 3.1.5.1 recommends logging the reported milestones.
    ///
    /// [MS-RDPET]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/
    fn report(&mut self, report: TelemetryPdu);
}

/// Server processor for `Microsoft::Windows::RDS::Telemetry`.
pub struct TelemetryServer {
    handler: Box<dyn TelemetryHandler>,
}

impl TelemetryServer {
    pub fn new(handler: Box<dyn TelemetryHandler>) -> Self {
        Self { handler }
    }
}

impl core::fmt::Debug for TelemetryServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TelemetryServer").finish_non_exhaustive()
    }
}

impl_as_any!(TelemetryServer);

impl DvcProcessor for TelemetryServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // Processing the report is optional (MS-RDPET section 3.1.5.1), so a malformed one does not end the session.
        match decode::<TelemetryPdu>(payload) {
            Ok(report) => self.handler.report(report),
            Err(error) => warn!(%error, "Ignoring malformed telemetry PDU"),
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for TelemetryServer {}
