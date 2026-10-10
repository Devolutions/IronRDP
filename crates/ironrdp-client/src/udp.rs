//! Internal client handling for a reliable UDP tunnel.

use ironrdp_rdpeudp_tokio::UdpTransport;
use ironrdp_session::{ActiveStage, SessionError, SessionResult};
use tracing::warn;

/// Drops a failed sideband and withdraws it from future Soft-Sync negotiation.
/// A channel already moved to the tunnel cannot resume over TCP without reconnecting.
/// Both receive closure and auto-detect send failures use this policy.
pub(crate) fn disable_failed_tunnel(
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
