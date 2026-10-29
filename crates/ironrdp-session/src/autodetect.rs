//! Client answers to the network auto-detect requests of [\[MS-RDPBCGR\] 2.2.14].
//!
//! [\[MS-RDPBCGR\] 2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/925b8e3a-fe60-43fe-aaff-29f07dc18993

use ironrdp_core::MonotonicInstant;
use ironrdp_pdu::rdp::autodetect::{
    AutoDetectRequest, AutoDetectResponse, BW_RESULTS_CONTINUOUS, BW_START_RELIABLE_UDP, BW_STOP_RELIABLE_UDP,
};
use tracing::debug;

/// Answers RTT requests and continuous reliable bandwidth measurements on one transport.
///
/// Connect-time measurements belong to the connection sequence, which `ironrdp-connector`
/// handles, and lossy-transport measurements are not answered. Each transport keeps its own
/// responder, because a continuous measurement counts the data received on the transport it
/// runs on ([\[MS-RDPBCGR\] 3.2.5.14]).
///
/// [\[MS-RDPBCGR\] 3.2.5.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/16ffa852-8aa7-481c-99a0-36c1a9a198f6
#[derive(Debug, Default)]
pub(crate) struct AutoDetectResponder {
    bandwidth: Option<BandwidthMeasurement>,
}

/// `timeDelta` reported when no timed window exists, and the floor for one that does: the
/// server divides `byteCount * 8` by it ([\[MS-RDPBCGR\] 3.3.5.14]).
///
/// [\[MS-RDPBCGR\] 3.3.5.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/b716fd9a-5a0b-4970-b5d0-f1be451408b7
const UNMEASURABLE_INTERVAL_MS: u32 = 1;

#[derive(Debug)]
struct BandwidthMeasurement {
    started_at: MonotonicInstant,
    bytes: u32,
}

impl AutoDetectResponder {
    /// Counts received bytes while a bandwidth window is open.
    pub(crate) fn record_bytes(&mut self, bytes: usize) {
        if let Some(measurement) = self.bandwidth.as_mut() {
            measurement.bytes = measurement
                .bytes
                .saturating_add(u32::try_from(bytes).unwrap_or(u32::MAX));
        }
    }

    /// Returns the response `request` calls for, if any.
    ///
    /// Without an arrival time for both the Start and the Stop, a Bandwidth Measure Stop is
    /// answered with `timeDelta = 1` and `byteCount = 0`.
    pub(crate) fn respond(
        &mut self,
        request: AutoDetectRequest,
        received_at: Option<MonotonicInstant>,
    ) -> Option<AutoDetectResponse> {
        match request {
            AutoDetectRequest::RttRequest { sequence_number, .. } => {
                Some(AutoDetectResponse::RttResponse { sequence_number })
            }
            AutoDetectRequest::BandwidthMeasureStart {
                request_type: BW_START_RELIABLE_UDP,
                ..
            } => {
                self.bandwidth = received_at.map(|started_at| BandwidthMeasurement { started_at, bytes: 0 });
                None
            }
            AutoDetectRequest::BandwidthMeasureStop {
                sequence_number,
                request_type: BW_STOP_RELIABLE_UDP,
                ..
            } => {
                let (time_delta_ms, byte_count) = match (self.bandwidth.take(), received_at) {
                    (Some(measurement), Some(stopped_at)) => (
                        u32::try_from(stopped_at.duration_since(measurement.started_at).as_millis())
                            .unwrap_or(u32::MAX)
                            .max(UNMEASURABLE_INTERVAL_MS),
                        measurement.bytes,
                    ),
                    (Some(measurement), None) => {
                        debug!(
                            dropped_bytes = measurement.bytes,
                            "Bandwidth Measure Stop arrived with no arrival time although its window was open; \
                             dropping the accumulated count"
                        );
                        (UNMEASURABLE_INTERVAL_MS, 0)
                    }
                    (None, _) => {
                        debug!("Bandwidth Measure Stop arrived with no open window; reporting no measurement");
                        (UNMEASURABLE_INTERVAL_MS, 0)
                    }
                };
                Some(AutoDetectResponse::BandwidthMeasureResults {
                    sequence_number,
                    response_type: BW_RESULTS_CONTINUOUS,
                    time_delta_ms,
                    byte_count,
                })
            }
            request => {
                // Connect-time measurements only occur during the connection sequence, lossy-transport
                // measurements are not answered here, and a Network Characteristics Result needs no answer.
                debug!(?request, "Not answering auto-detect request");
                None
            }
        }
    }
}
