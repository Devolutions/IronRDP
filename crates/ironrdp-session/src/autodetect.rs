//! Client answers to the network auto-detect requests of [MS-RDPBCGR] 2.2.14.
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38

use ironrdp_core::MonotonicInstant;
use ironrdp_pdu::rdp::autodetect::{
    AutoDetectRequest, AutoDetectResponse, BW_RESULTS_CONNECT_TIME, BW_RESULTS_CONTINUOUS, BW_START_CONNECT_TIME,
    BW_START_RELIABLE_UDP, BW_STOP_CONNECT_TIME, BW_STOP_RELIABLE_UDP,
};
use tracing::debug;

/// Answers the RTT and bandwidth measurement requests that arrive on one transport.
///
/// Each transport keeps its own, because a continuous measurement counts the data received on
/// the transport it runs on ([MS-RDPBCGR] 3.2.5.14). Timestamps come from the caller, from the
/// same monotonic clock for every request on that transport.
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/16ffa852-8aa7-481c-99a0-36c1a9a198f6
#[derive(Debug, Default)]
pub(crate) struct AutoDetectResponder {
    bandwidth: Option<BandwidthMeasurement>,
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
                    // [MS-RDPBCGR] 3.2.5.14 counts the eight-byte auto-detect
                    // header as well as payloadLength, but not the security header.
                    measurement.bytes = measurement
                        .bytes
                        .saturating_add(u32::try_from(payload.len()).unwrap_or(u32::MAX).saturating_add(8));
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
                    u32::try_from(payload.as_ref().map_or(0, Vec::len))
                        .unwrap_or(u32::MAX)
                        .saturating_add(8)
                };
                let (time_delta_ms, byte_count) = match (measurement, received_at) {
                    (Some(measurement), Some(stopped_at)) => (
                        u32::try_from(stopped_at.duration_since(measurement.started_at).as_millis())
                            .unwrap_or(u32::MAX)
                            .max(1),
                        measurement.bytes.saturating_add(stop_bytes),
                    ),
                    _ => (1, stop_bytes),
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
