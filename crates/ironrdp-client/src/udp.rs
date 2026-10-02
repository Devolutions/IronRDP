//! Internal client handling for a reliable UDP tunnel.

use ironrdp_core::{Decode, Encode};
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};
use ironrdp_rdpeudp_tokio::UdpTransport;
use ironrdp_session::{ActiveStage, SessionError, SessionResult};
use tracing::{debug, warn};

/// Decodes the auto-detect requests among the sub-headers of a Tunnel Data PDU.
///
/// Each sub-header is the request structure itself: SubHeaderLength and SubHeaderType
/// are headerLength and headerTypeId ([MS-RDPEMT] 2.2.1.1.1). Unknown requests are skipped.
/// RTT requests are accepted for Windows interoperability in addition to the bandwidth
/// requests listed by MS-RDPEMT; RTT encapsulation on a tunnel is not specified there.
///
/// [MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/4f538fd7-3aca-4e7d-a213-13eb5f95c1ad
pub fn tunnel_auto_detect_requests(sub_headers: &[TunnelSubHeader]) -> Vec<AutoDetectRequest> {
    sub_headers
        .iter()
        .filter(|sub_header| sub_header.sub_header_type == SubHeaderType::AutoDetectRequest)
        .filter_map(|sub_header| match redecode(sub_header) {
            Ok(request) => Some(request),
            Err(error) => {
                debug!(%error, data = ?sub_header.data, "Ignoring an undecodable auto-detect request on the tunnel");
                None
            }
        })
        .collect()
}

/// Encodes a response as its wire-compatible tunnel sub-header.
pub fn tunnel_auto_detect_sub_header(response: &AutoDetectResponse) -> Option<TunnelSubHeader> {
    match redecode(response) {
        Ok(sub_header) => Some(sub_header),
        Err(error) => {
            debug!(%error, ?response, "Could not encode an auto-detect response for the tunnel");
            None
        }
    }
}

fn redecode<T: for<'de> Decode<'de>>(value: &dyn Encode) -> Result<T, String> {
    let bytes = ironrdp_core::encode_vec(value).map_err(|error| error.to_string())?;
    ironrdp_core::decode(&bytes).map_err(|error| error.to_string())
}

/// Drops a failed sideband and withdraws it from future Soft-Sync negotiation.
/// A channel already moved to the tunnel cannot resume over TCP without reconnecting.
/// Both receive closure and auto-detect send failures use this policy.
pub fn disable_failed_tunnel(
    stage: &mut ActiveStage,
    transport: &mut Option<UdpTransport>,
    error: SessionError,
) -> SessionResult<()> {
    if stage.reliable_udp_dvc_tunnel_in_use() {
        return Err(error);
    }
    *transport = None;
    stage.disable_reliable_udp_dvc_tunnel()?;
    warn!(%error, "Reliable UDP tunnel failed before channel migration; continuing with TCP");
    Ok(())
}
