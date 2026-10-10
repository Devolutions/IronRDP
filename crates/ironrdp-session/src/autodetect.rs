//! Client answers to the network auto-detect requests of [MS-RDPBCGR] 2.2.14.
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38

use ironrdp_core::MonotonicInstant;
use ironrdp_pdu::rdp::autodetect::{
    AutoDetectRequest, AutoDetectResponse, BW_RESULTS_CONNECT_TIME, BW_RESULTS_CONTINUOUS, BW_START_CONNECT_TIME,
    BW_START_RELIABLE_UDP, BW_STOP_CONNECT_TIME, BW_STOP_RELIABLE_UDP,
};
use tracing::debug;

/// Answers the RTT and bandwidth measurement requests of a session.
///
/// A session has one Network Characteristics Byte Count store and one timer, whichever
/// transport carries the Start and the Stop: a continuous measurement counts the data received
/// from the server on every transport ([MS-RDPBCGR] 3.2.5.14). The caller sends each response
/// back on the transport its request arrived on. Timestamps come from the caller, from one
/// monotonic clock for every transport.
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/16ffa852-8aa7-481c-99a0-36c1a9a198f6
#[derive(Debug, Default)]
pub(crate) struct AutoDetectResponder {
    bandwidth: Option<BandwidthMeasurement>,
}

/// Size of the auto-detect header fields [MS-RDPBCGR] 3.2.5.14 counts along with payloadLength:
/// headerLength, headerTypeId, sequenceNumber, requestType and payloadLength itself.
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/16ffa852-8aa7-481c-99a0-36c1a9a198f6
const AUTO_DETECT_HEADER_LEN: u32 = 8;

/// Reported as `timeDelta` for a window that was not timed, and the floor for one that was:
/// a server computing `byteCount * 8 / timeDelta` divides by it ([MS-RDPBCGR] 3.3.5.14).
const UNMEASURABLE_INTERVAL_MS: u32 = 1;

/// Bytes a connect-time Payload or Stop adds to the count: payloadLength plus the auto-detect
/// header, but not the security header. This is the rule the connector applies to the same PDUs.
fn counted_len(payload_len: usize) -> u32 {
    u32::try_from(payload_len)
        .unwrap_or(u32::MAX)
        .saturating_add(AUTO_DETECT_HEADER_LEN)
}

#[derive(Debug)]
struct BandwidthMeasurement {
    started_at: MonotonicInstant,
    bytes: u32,
    continuous: bool,
}

impl AutoDetectResponder {
    /// Counts received bytes while a continuous bandwidth window is open.
    pub(crate) fn record_bytes(&mut self, bytes: usize) {
        if let Some(measurement) = self.bandwidth.as_mut().filter(|measurement| measurement.continuous) {
            measurement.bytes = measurement
                .bytes
                .saturating_add(u32::try_from(bytes).unwrap_or(u32::MAX));
        }
    }

    /// Returns the response `request` calls for, if any.
    ///
    /// Without an arrival time, a bandwidth measurement is answered with an untimed, zero-byte
    /// result.
    pub(crate) fn respond(
        &mut self,
        request: AutoDetectRequest,
        received_at: Option<MonotonicInstant>,
    ) -> Option<AutoDetectResponse> {
        match request {
            AutoDetectRequest::RttRequest { sequence_number, .. } => {
                Some(AutoDetectResponse::RttResponse { sequence_number })
            }
            AutoDetectRequest::BandwidthMeasureStart { request_type, .. }
                if matches!(request_type, BW_START_CONNECT_TIME | BW_START_RELIABLE_UDP) =>
            {
                self.bandwidth = received_at.map(|started_at| BandwidthMeasurement {
                    started_at,
                    bytes: 0,
                    continuous: request_type == BW_START_RELIABLE_UDP,
                });
                None
            }
            AutoDetectRequest::BandwidthMeasurePayload { payload, .. } => {
                if let Some(measurement) = self.bandwidth.as_mut().filter(|measurement| !measurement.continuous) {
                    measurement.bytes = measurement.bytes.saturating_add(counted_len(payload.len()));
                }
                None
            }
            AutoDetectRequest::BandwidthMeasureStop {
                sequence_number,
                request_type,
                payload,
            } if matches!(request_type, BW_STOP_CONNECT_TIME | BW_STOP_RELIABLE_UDP) => {
                let continuous = request_type == BW_STOP_RELIABLE_UDP;
                let measurement = self
                    .bandwidth
                    .take()
                    .filter(|measurement| measurement.continuous == continuous);
                let stop_bytes = if continuous {
                    0
                } else {
                    counted_len(payload.as_ref().map_or(0, Vec::len))
                };
                let (time_delta_ms, byte_count) = match (measurement, received_at) {
                    (Some(measurement), Some(stopped_at)) => (
                        u32::try_from(stopped_at.duration_since(measurement.started_at).as_millis())
                            .unwrap_or(u32::MAX)
                            .max(UNMEASURABLE_INTERVAL_MS),
                        measurement.bytes.saturating_add(stop_bytes),
                    ),
                    (Some(measurement), None) => {
                        // The window was timed but this Stop was not, so there is nothing to
                        // divide the count by. Log the drop so it does not look like the
                        // ordinary no-window case.
                        debug!(
                            dropped_bytes = measurement.bytes,
                            "Bandwidth Measure Stop arrived with no arrival time although its window was open; \
                             dropping the accumulated count"
                        );
                        (UNMEASURABLE_INTERVAL_MS, 0)
                    }
                    (None, _) => (UNMEASURABLE_INTERVAL_MS, 0),
                };
                Some(AutoDetectResponse::BandwidthMeasureResults {
                    sequence_number,
                    response_type: if continuous {
                        BW_RESULTS_CONTINUOUS
                    } else {
                        BW_RESULTS_CONNECT_TIME
                    },
                    time_delta_ms,
                    byte_count,
                })
            }
            request @ AutoDetectRequest::NetworkCharacteristicsResult { .. } => {
                // The message-channel processor surfaces this request itself; requests from
                // the UDP tunnel reach this responder directly.
                debug!(?request, "Received network characteristics from server");
                None
            }
            request => {
                // Measurements for a lossy transport are not answered here.
                debug!(?request, "Ignoring auto-detect request for another transport");
                None
            }
        }
    }
}
