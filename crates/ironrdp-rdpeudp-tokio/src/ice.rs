//! ICE candidate gathering and connectivity checks for RDPEUDP2.
//!
//! This module uses ICE ([RFC 8445]) with UDP host, server-reflexive, and TURN relay candidates.
//! Candidate signaling remains the caller's responsibility.
//! The API does not define or imply an AVD Shortpath signaling contract.
//!
//! [RFC 8445]: https://www.rfc-editor.org/rfc/rfc8445

use core::error::Error;
use core::fmt;
use core::net::{IpAddr, Ipv6Addr, SocketAddr};
use core::time::Duration;
use std::io;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::warn;
use webrtc_ice::agent::agent_config::AgentConfig;
use webrtc_ice::agent::{Agent, OnCandidateHdlrFn};
use webrtc_ice::candidate::candidate_base::unmarshal_candidate;
use webrtc_ice::candidate::{Candidate, CandidateType};
use webrtc_ice::mdns::MulticastDnsMode;
use webrtc_ice::network_type::NetworkType;
use webrtc_ice::url::{ProtoType, SchemeType, Url};
use webrtc_util::Conn;

use crate::driver::DatagramIo;
use crate::error::UdpTransportErrorExt as _;
use crate::transport::{
    UdpAcceptConfig, UdpTransport, UdpTransportConfig, accept_udp_with_datagram_io, connect_udp_with_datagram_io,
};

const MAX_ICE_SERVERS: usize = 8;
const MAX_ICE_CANDIDATES: usize = 64;
const MAX_ICE_CANDIDATE_LENGTH: usize = 4096;
const MAX_ICE_CREDENTIAL_LENGTH: usize = 256;
const MAX_ICE_TIMEOUT: Duration = Duration::from_secs(120);

/// STUN or UDP TURN server configuration.
///
/// TURN credentials are supplied by the caller and are never included in debug output.
/// Only UDP STUN and TURN URLs are accepted.
#[derive(Clone)]
pub struct IceServerConfig {
    /// Server URL, such as `stun:stun.example.net:3478` or `turn:turn.example.net:3478?transport=udp`.
    pub url: String,
    /// TURN username; omit for STUN.
    pub username: Option<String>,
    /// TURN long-term credential; omit for STUN.
    pub password: Option<String>,
}

impl IceServerConfig {
    /// Configure an unauthenticated STUN server.
    pub fn stun(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            username: None,
            password: None,
        }
    }

    /// Configure a UDP TURN server with caller-provided credentials.
    pub fn turn(url: impl Into<String>, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            username: Some(username.into()),
            password: Some(password.into()),
        }
    }
}

impl fmt::Debug for IceServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceServerConfig")
            .field("url", &self.url)
            .field("username", &self.username.as_ref().map(|_| "[redacted]"))
            .field("password", &self.password.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

/// Local candidate-gathering configuration.
#[derive(Debug, Clone)]
pub struct IceGatherConfig {
    /// STUN and TURN servers used to gather server-reflexive and relay candidates.
    pub servers: Vec<IceServerConfig>,
    /// Total budget for agent initialization and candidate gathering.
    pub timeout: Duration,
}

impl Default for IceGatherConfig {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            timeout: Duration::from_secs(15),
        }
    }
}

/// ICE username fragment and password exchanged by the caller's signaling layer.
#[derive(Clone, PartialEq, Eq)]
pub struct IceCredentials {
    /// ICE username fragment.
    pub username_fragment: String,
    /// ICE password.
    pub password: String,
}

impl fmt::Debug for IceCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceCredentials")
            .field("username_fragment", &self.username_fragment)
            .field("password", &"[redacted]")
            .finish()
    }
}

/// Gathered local candidates and credentials, ready for caller-defined signaling.
///
/// This value owns the ICE agent and must be passed to [`connect_udp_with_ice`] or [`accept_udp_with_ice`] to establish a transport.
/// Dropping it cancels the agent and schedules its asynchronous cleanup.
pub struct IceGatheredCandidates {
    credentials: IceCredentials,
    candidates: Vec<String>,
    agent: IceAgentGuard,
}

impl IceGatheredCandidates {
    /// The local ICE credentials to deliver to the remote peer.
    pub fn credentials(&self) -> &IceCredentials {
        &self.credentials
    }

    /// ICE candidate attribute values, without the SDP `a=` prefix.
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }
}

impl fmt::Debug for IceGatheredCandidates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceGatheredCandidates")
            .field("credentials", &self.credentials)
            .field("candidate_count", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

/// Remote ICE credentials and candidates supplied by the caller's signaling layer.
#[derive(Debug, Clone)]
pub struct RemoteIceDescription {
    /// Credentials received from the remote ICE agent.
    pub credentials: IceCredentials,
    /// Candidate attribute values, without the SDP `a=` prefix.
    pub candidates: Vec<String>,
}

/// ICE connectivity-checking role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    /// Initiate connectivity checks and nominate the selected pair.
    Controlling,
    /// Respond to connectivity checks and accept the selected pair.
    Controlled,
}

/// Failure while gathering candidates, checking a remote candidate pair, or establishing the RDPEUDP2 transport over the selected ICE connection.
#[derive(Debug)]
pub enum IceTransportError {
    /// A caller-supplied value exceeded the documented limit or was unsupported.
    InvalidConfiguration(&'static str),
    /// Candidate gathering did not finish before its deadline.
    GatheringTimeout,
    /// ICE connectivity checks did not select a candidate pair before the deadline.
    ConnectivityTimeout,
    /// ICE gathering completed without any local candidates.
    NoLocalCandidates,
    /// ICE completed without exposing a selected remote address.
    NoSelectedCandidatePair,
    /// The ICE implementation rejected a server, candidate, or connectivity operation.
    Agent(Box<dyn Error + Send + Sync>),
    /// RDPEUDP2, TLS, or RDPEMT failed after ICE selected a path.
    Transport(crate::UdpTransportError),
}

impl fmt::Display for IceTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(reason) => write!(f, "invalid ICE configuration: {reason}"),
            Self::GatheringTimeout => write!(f, "ICE candidate gathering timed out"),
            Self::ConnectivityTimeout => write!(f, "ICE connectivity checks timed out"),
            Self::NoLocalCandidates => write!(f, "ICE gathering produced no local candidates"),
            Self::NoSelectedCandidatePair => write!(f, "ICE did not select a candidate pair"),
            Self::Agent(error) => write!(f, "ICE agent failed: {error}"),
            Self::Transport(error) => write!(f, "RDPEUDP2 transport over ICE failed: {error}"),
        }
    }
}

impl Error for IceTransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Agent(error) => Some(error.as_ref()),
            Self::Transport(error) => Some(error),
            Self::InvalidConfiguration(_)
            | Self::GatheringTimeout
            | Self::ConnectivityTimeout
            | Self::NoLocalCandidates
            | Self::NoSelectedCandidatePair => None,
        }
    }
}

/// Gather host candidates and, when configured, STUN server-reflexive and TURN relay candidates ([RFC 8445] §5.1.1 "Gathering Candidates").
///
/// The caller must signal the returned credentials and candidates to a remote ICE agent using its own established signaling contract.
/// This function gathers UDP candidates only; it does not implement application signaling.
///
/// # Errors
///
/// Returns [`IceTransportError::InvalidConfiguration`] for unsupported server URLs or input limits, [`IceTransportError::GatheringTimeout`] if gathering exceeds `config.timeout`, and [`IceTransportError::Agent`] if the ICE agent rejects its configuration or encounters a network error.
///
/// [RFC 8445]: https://www.rfc-editor.org/rfc/rfc8445#section-5.1.1
pub async fn gather_ice_candidates(config: IceGatherConfig) -> Result<IceGatheredCandidates, IceTransportError> {
    gather_ice_candidates_with_types(
        config,
        vec![
            CandidateType::Host,
            CandidateType::ServerReflexive,
            CandidateType::Relay,
        ],
    )
    .await
}

async fn gather_ice_candidates_with_types(
    config: IceGatherConfig,
    candidate_types: Vec<CandidateType>,
) -> Result<IceGatheredCandidates, IceTransportError> {
    validate_timeout(config.timeout)?;
    if MAX_ICE_SERVERS < config.servers.len() {
        return Err(IceTransportError::InvalidConfiguration("too many ICE servers"));
    }

    let deadline = Instant::now() + config.timeout;
    let agent_config = AgentConfig {
        urls: config
            .servers
            .into_iter()
            .map(parse_ice_server)
            .collect::<Result<_, _>>()?,
        candidate_types,
        network_types: vec![NetworkType::Udp4, NetworkType::Udp6],
        multicast_dns_mode: MulticastDnsMode::Disabled,
        include_loopback: cfg!(test),
        ..AgentConfig::default()
    };

    let agent = tokio::time::timeout_at(deadline, Agent::new(agent_config))
        .await
        .map_err(|_| IceTransportError::GatheringTimeout)?
        .map_err(agent_error)?;
    let agent = IceAgentGuard::new(agent);

    let (candidate_tx, mut candidate_rx) = mpsc::channel(MAX_ICE_CANDIDATES + 1);
    let handler: OnCandidateHdlrFn = Box::new(move |candidate| {
        let candidate_tx = candidate_tx.clone();
        Box::pin(async move {
            let candidate = candidate.map(|candidate| candidate.marshal());
            let _ = candidate_tx.send(candidate).await;
        })
    });
    agent.agent().on_candidate(handler);
    agent.agent().gather_candidates().map_err(agent_error)?;

    let candidates = tokio::time::timeout_at(deadline, async {
        let mut candidates = Vec::new();
        loop {
            match candidate_rx.recv().await {
                Some(Some(candidate)) => {
                    if candidates.len() < MAX_ICE_CANDIDATES {
                        candidates.push(candidate);
                    }
                }
                Some(None) => return Ok(candidates),
                None => {
                    return Err(IceTransportError::Agent(Box::new(io::Error::other(
                        "ICE candidate event stream closed before gathering completed",
                    ))));
                }
            }
        }
    })
    .await
    .map_err(|_| IceTransportError::GatheringTimeout)??;

    if candidates.is_empty() {
        return Err(IceTransportError::NoLocalCandidates);
    }

    let (username_fragment, password) = agent.agent().get_local_user_credentials().await;
    Ok(IceGatheredCandidates {
        credentials: IceCredentials {
            username_fragment,
            password,
        },
        candidates,
        agent,
    })
}

/// Establish an RDPEUDP2 transport over the nominated ICE candidate pair ([RFC 8445] §6 "Performing Connectivity Checks").
///
/// ICE uses the caller-supplied remote credentials and candidate attribute values.
/// `config.server_addr` is replaced with the nominated ICE peer address while `config.tls.certificate_validation_endpoint` is preserved as configured.
/// The returned transport keeps the ICE agent alive for consent freshness, TURN allocation maintenance, and ongoing data transfer.
///
/// [RFC 8445]: https://www.rfc-editor.org/rfc/rfc8445#section-6
pub async fn connect_udp_with_ice(
    config: UdpTransportConfig,
    local: IceGatheredCandidates,
    remote: RemoteIceDescription,
    role: IceRole,
    timeout: Duration,
) -> Result<UdpTransport, IceTransportError> {
    let (io, agent) = connect_ice_candidates(local, remote, role, timeout).await?;
    let peer = io.peer_addr().ok_or(IceTransportError::NoSelectedCandidatePair)?;
    let mut config = config;
    config.server_addr = peer;
    let mut transport = connect_udp_with_datagram_io(config, io)
        .await
        .map_err(IceTransportError::Transport)?;
    transport.ice_agent = Some(agent);
    Ok(transport)
}

/// Accept an RDPEUDP2 transport over the nominated ICE candidate pair ([RFC 8445] §6 "Performing Connectivity Checks").
///
/// This variant is useful to ICE-capable RDP servers and local test peers.
/// Existing [`crate::accept_udp`] remains the direct, non-ICE path.
///
/// [RFC 8445]: https://www.rfc-editor.org/rfc/rfc8445#section-6
pub async fn accept_udp_with_ice(
    config: UdpAcceptConfig,
    local: IceGatheredCandidates,
    remote: RemoteIceDescription,
    role: IceRole,
    timeout: Duration,
) -> Result<UdpTransport, IceTransportError> {
    validate_timeout(config.accept_timeout)?;
    let accept_timeout = config.accept_timeout;
    let (io, agent) = connect_ice_candidates(local, remote, role, timeout).await?;
    let mut transport = tokio::time::timeout(accept_timeout, accept_udp_with_datagram_io(io, config))
        .await
        .map_err(|_| IceTransportError::Transport(crate::UdpTransportError::handshake_timeout("accept udp with ice")))?
        .map_err(IceTransportError::Transport)?;
    transport.ice_agent = Some(agent);
    Ok(transport)
}

async fn connect_ice_candidates(
    local: IceGatheredCandidates,
    remote: RemoteIceDescription,
    role: IceRole,
    timeout: Duration,
) -> Result<(IceDatagramIo, IceAgentGuard), IceTransportError> {
    validate_timeout(timeout)?;
    if remote.candidates.is_empty() {
        return Err(IceTransportError::InvalidConfiguration(
            "remote candidate list is empty",
        ));
    }
    if MAX_ICE_CANDIDATES < remote.candidates.len() {
        return Err(IceTransportError::InvalidConfiguration(
            "too many remote ICE candidates",
        ));
    }
    validate_credentials(&remote.credentials)?;

    let agent = local.agent.agent();
    for raw_candidate in &remote.candidates {
        if MAX_ICE_CANDIDATE_LENGTH < raw_candidate.len() {
            return Err(IceTransportError::InvalidConfiguration(
                "remote ICE candidate is too long",
            ));
        }
        let candidate = unmarshal_candidate(raw_candidate).map_err(agent_error)?;
        if !candidate.network_type().is_udp() || candidate.component() != 1 {
            return Err(IceTransportError::InvalidConfiguration(
                "RDPEUDP2 requires UDP candidates for component 1",
            ));
        }
        if candidate.address().parse::<IpAddr>().is_err() {
            return Err(IceTransportError::InvalidConfiguration(
                "remote ICE candidate address must be an IP literal",
            ));
        }
        let candidate: Arc<dyn Candidate + Send + Sync> = Arc::new(candidate);
        agent.add_remote_candidate(&candidate).map_err(agent_error)?;
    }

    let (cancel_tx, cancel_rx) = mpsc::channel(1);
    let connection: Arc<dyn Conn + Send + Sync> = match role {
        IceRole::Controlling => tokio::time::timeout(
            timeout,
            agent.dial(
                cancel_rx,
                remote.credentials.username_fragment,
                remote.credentials.password,
            ),
        )
        .await
        .map_err(|_| IceTransportError::ConnectivityTimeout)?
        .map_err(agent_error)?,
        IceRole::Controlled => tokio::time::timeout(
            timeout,
            agent.accept(
                cancel_rx,
                remote.credentials.username_fragment,
                remote.credentials.password,
            ),
        )
        .await
        .map_err(|_| IceTransportError::ConnectivityTimeout)?
        .map_err(agent_error)?,
    };
    drop(cancel_tx);
    let _selected_pair = agent
        .get_selected_candidate_pair()
        .ok_or(IceTransportError::NoSelectedCandidatePair)?;

    Ok((IceDatagramIo { connection }, local.agent))
}

struct IceDatagramIo {
    connection: Arc<dyn Conn + Send + Sync>,
}

#[expect(
    clippy::manual_async_fn,
    reason = "The driver requires Send futures for tokio::spawn"
)]
impl DatagramIo for IceDatagramIo {
    fn local_addr(&self) -> Option<SocketAddr> {
        Conn::local_addr(self.connection.as_ref()).ok()
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        Conn::remote_addr(self.connection.as_ref())
    }

    fn send<'a>(&'a self, buf: &'a [u8]) -> impl Future<Output = io::Result<usize>> + Send + 'a {
        async move {
            let sent = Conn::send(self.connection.as_ref(), buf)
                .await
                .map_err(io::Error::other)?;
            if buf.len() <= sent {
                // Direct sockets report `buf.len()`, whereas TURN relay sockets may report the encapsulating packet size.
                return Ok(buf.len());
            }
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "ICE datagram send completed with an invalid byte count",
            ))
        }
    }

    fn recv<'a>(&'a self, buf: &'a mut [u8]) -> impl Future<Output = io::Result<usize>> + Send + 'a {
        async move {
            Conn::recv(self.connection.as_ref(), buf)
                .await
                .map_err(io::Error::other)
        }
    }
}

pub(crate) struct IceAgentGuard {
    agent: Arc<Agent>,
    close_tx: Option<oneshot::Sender<()>>,
    close_task: Option<JoinHandle<()>>,
}

impl IceAgentGuard {
    fn new(agent: Agent) -> Self {
        let agent = Arc::new(agent);
        let cleanup_agent = Arc::clone(&agent);
        let (close_tx, close_rx) = oneshot::channel();
        let close_task = tokio::spawn(async move {
            let _ = close_rx.await;
            if let Err(error) = cleanup_agent.close().await {
                warn!(%error, "Failed to close ICE agent");
            }
        });

        Self {
            agent,
            close_tx: Some(close_tx),
            close_task: Some(close_task),
        }
    }

    fn agent(&self) -> &Agent {
        &self.agent
    }

    fn signal_close(&mut self) {
        if let Some(close_tx) = self.close_tx.take() {
            let _ = close_tx.send(());
        }
    }

    pub(crate) async fn close(mut self) {
        self.signal_close();
        if let Some(close_task) = self.close_task.take() {
            if let Err(error) = close_task.await {
                warn!(%error, "ICE agent cleanup task failed");
            }
        }
    }
}

impl Drop for IceAgentGuard {
    fn drop(&mut self) {
        // Signal the spawned cleanup task and leave its JoinHandle detached so
        // `cleanup_agent.close().await` completes asynchronously on drop paths.
        self.signal_close();
    }
}

fn parse_ice_server(server: IceServerConfig) -> Result<Url, IceTransportError> {
    if 1024 < server.url.len() {
        return Err(IceTransportError::InvalidConfiguration("ICE server URL is too long"));
    }
    let mut url = Url::parse_url(&server.url).map_err(agent_error)?;
    match url.scheme {
        SchemeType::Stun if server.username.is_none() && server.password.is_none() => {}
        SchemeType::Turn if url.proto == ProtoType::Udp => {
            if url.host.parse::<Ipv6Addr>().is_ok() {
                return Err(IceTransportError::InvalidConfiguration(
                    "IPv6 TURN server URLs are not supported",
                ));
            }
            let (Some(username), Some(password)) = (server.username, server.password) else {
                return Err(IceTransportError::InvalidConfiguration(
                    "UDP TURN servers require a username and credential",
                ));
            };
            if username.is_empty() || password.is_empty() {
                return Err(IceTransportError::InvalidConfiguration(
                    "UDP TURN credentials must not be empty",
                ));
            }
            url.username = username;
            url.password = password;
        }
        _ => {
            return Err(IceTransportError::InvalidConfiguration(
                "only STUN and UDP TURN server URLs are supported",
            ));
        }
    }
    Ok(url)
}

fn validate_timeout(timeout: Duration) -> Result<(), IceTransportError> {
    if timeout < Duration::from_millis(1) || MAX_ICE_TIMEOUT < timeout {
        return Err(IceTransportError::InvalidConfiguration(
            "ICE timeout must be between 1 ms and 120 s",
        ));
    }
    Ok(())
}

fn validate_credentials(credentials: &IceCredentials) -> Result<(), IceTransportError> {
    let valid_ice_chars = |value: &str, min_length: usize| {
        (min_length..=MAX_ICE_CREDENTIAL_LENGTH).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    };
    if !valid_ice_chars(&credentials.username_fragment, 4) || !valid_ice_chars(&credentials.password, 22) {
        return Err(IceTransportError::InvalidConfiguration(
            "remote ICE credentials must use RFC 8445 lengths and characters",
        ));
    }
    Ok(())
}

fn agent_error(error: impl Error + Send + Sync + 'static) -> IceTransportError {
    IceTransportError::Agent(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "rustls-ring")]
    use ironrdp_rdpemt::TunnelConfig;
    #[cfg(feature = "rustls-ring")]
    use ironrdp_rdpeudp::ConnectionConfig;
    use rstest::rstest;

    // oracle: mutant leaked TURN or ICE credentials in Debug output
    #[test]
    fn hides_server_and_password_credentials_from_debug_output() {
        let server = IceServerConfig::turn(
            "turn:turn.example.net:3478?transport=udp",
            "private-user",
            "private-password",
        );
        let credentials = IceCredentials {
            username_fragment: "local".into(),
            password: "private-ice-password".into(),
        };

        let debug = format!("{server:?} {credentials:?}");
        assert!(!debug.contains("private-user"));
        assert!(!debug.contains("private-password"));
        assert!(!debug.contains("private-ice-password"));
        assert!(debug.contains("[redacted]"));
    }

    // oracle: spec RFC 7065 §3.1
    #[test]
    fn accepts_udp_turn_server_url() {
        let url = parse_ice_server(IceServerConfig::turn(
            "turn:turn.example.net:3478?transport=udp",
            "user",
            "credential",
        ))
        .expect("valid UDP TURN URL");
        assert_eq!(url.scheme, SchemeType::Turn);
        assert_eq!(url.proto, ProtoType::Udp);
        assert_eq!(url.username, "user");
    }

    // oracle: mutant accepts non-UDP TURN or unauthenticated TURN server URLs
    #[rstest]
    #[case::turns_scheme(IceServerConfig::stun("turns:turn.example.net:5349"))]
    #[case::turn_without_credentials(IceServerConfig::stun("turn:turn.example.net:3478"))]
    #[case::turn_over_tcp(IceServerConfig::turn("turn:turn.example.net:3478?transport=tcp", "user", "credential"))]
    #[case::ipv6_turn_host(IceServerConfig::turn("turn:[2001:db8::1]:3478?transport=udp", "user", "credential"))]
    fn rejects_unsupported_ice_server_url(#[case] server: IceServerConfig) {
        assert!(parse_ice_server(server).is_err());
    }

    // oracle: spec RFC 8445 §16.4
    #[test]
    fn accepts_valid_remote_ice_credentials() {
        assert!(
            validate_credentials(&IceCredentials {
                username_fragment: "abcd".into(),
                password: "abcdefghijklmnopqrstuv".into(),
            })
            .is_ok()
        );
    }

    // oracle: spec RFC 8445 §16.4
    #[rstest]
    #[case::short_ufrag("abc", "abcdefghijklmnopqrstuv")]
    #[case::short_password("abcd", "short")]
    #[case::disallowed_ufrag_character("abc d", "abcdefghijklmnopqrstuv")]
    fn rejects_invalid_remote_ice_credentials(#[case] username_fragment: &str, #[case] password: &str) {
        assert!(
            validate_credentials(&IceCredentials {
                username_fragment: username_fragment.into(),
                password: password.into(),
            })
            .is_err()
        );
    }

    // oracle: spec RFC 8445 §6
    #[tokio::test]
    async fn gathers_candidates_and_establishes_an_ice_datagram_path() {
        let config = IceGatherConfig {
            timeout: Duration::from_secs(10),
            ..IceGatherConfig::default()
        };
        let (left, right) = tokio::join!(gather_ice_candidates(config.clone()), gather_ice_candidates(config));
        let left = left.expect("left candidates gathered");
        let right = right.expect("right candidates gathered");

        let left_remote = RemoteIceDescription {
            credentials: right.credentials().clone(),
            candidates: right.candidates().to_vec(),
        };
        let right_remote = RemoteIceDescription {
            credentials: left.credentials().clone(),
            candidates: left.candidates().to_vec(),
        };
        let (left_connection, right_connection) = tokio::join!(
            connect_ice_candidates(left, left_remote, IceRole::Controlling, Duration::from_secs(10)),
            connect_ice_candidates(right, right_remote, IceRole::Controlled, Duration::from_secs(10))
        );
        let (left_io, left_agent) = left_connection.expect("left ICE connection established");
        let (right_io, right_agent) = right_connection.expect("right ICE connection established");

        let mut received = [0; 5];
        let (sent, received_len) = tokio::join!(left_io.send(b"rdp2!"), right_io.recv(&mut received));
        assert_eq!(sent.expect("datagram sent"), 5);
        assert_eq!(received_len.expect("datagram received"), 5);
        assert_eq!(&received, b"rdp2!");

        left_agent.close().await;
        right_agent.close().await;
    }

    // oracle: spec MS-RDPEUDP2 §3.1.3
    #[cfg(feature = "rustls-ring")]
    #[tokio::test]
    async fn establishes_an_rdpeudp2_tunnel_over_the_selected_ice_path() {
        use crate::multitransport::MultitransportBootstrap;
        use crate::transport::{UdpAcceptConfig, UdpTransportConfig};
        use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
        use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, RequestedProtocol};
        use tokio_rustls::rustls;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let certified_key = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("certificate");
        let certificate = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
        let key =
            rustls::pki_types::PrivateKeyDer::try_from(certified_key.signing_key.serialize_der()).expect("private key");
        let tls_config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key)
                .expect("server TLS config"),
        );

        let gather_config = IceGatherConfig {
            timeout: Duration::from_secs(10),
            ..IceGatherConfig::default()
        };
        let (client_local, server_local) = tokio::join!(
            gather_ice_candidates(gather_config.clone()),
            gather_ice_candidates(gather_config)
        );
        let client_local = client_local.expect("client candidates gathered");
        let server_local = server_local.expect("server candidates gathered");
        let client_remote = RemoteIceDescription {
            credentials: server_local.credentials().clone(),
            candidates: server_local.candidates().to_vec(),
        };
        let server_remote = RemoteIceDescription {
            credentials: client_local.credentials().clone(),
            candidates: client_local.candidates().to_vec(),
        };
        let tunnel_config = TunnelConfig {
            request_id: 42,
            security_cookie: [0xAB; 16],
        };
        let mut bootstrap = MultitransportBootstrap::new(MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: tunnel_config.request_id,
            requested_protocol: RequestedProtocol::UdpFecR,
            security_cookie: tunnel_config.security_cookie,
        });

        let client_config = UdpTransportConfig::new(
            "127.0.0.1:0".parse().expect("placeholder address"),
            "localhost".into(),
            TunnelConfig {
                request_id: 0,
                security_cookie: [0; 16],
            },
        );
        let server_config = UdpAcceptConfig {
            tls_config,
            tunnel_config,
            connection_config: ConnectionConfig::default(),
            accept_timeout: Duration::from_secs(10),
        };

        let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(
                bootstrap.connect_with_ice(
                    client_config,
                    client_local,
                    client_remote,
                    IceRole::Controlling,
                    Duration::from_secs(10),
                ),
                accept_udp_with_ice(
                    server_config,
                    server_local,
                    server_remote,
                    IceRole::Controlled,
                    Duration::from_secs(10),
                )
            )
        })
        .await
        .expect("ICE plus RDPEUDP2/TLS/RDPEMT completed before the overall deadline");
        client_result.expect("client multitransport bootstrap completed");
        assert!(bootstrap.is_connected());
        assert!(bootstrap.response_pdu().is_some());
        let client = bootstrap.take_transport().expect("client tunnel established");
        let mut server = server_result.expect("server tunnel established");

        client.send(b"over ICE".to_vec()).await.expect("client send");
        let received = tokio::time::timeout(Duration::from_secs(5), server.recv())
            .await
            .expect("server receive deadline")
            .expect("server tunnel remains open");
        assert_eq!(received, b"over ICE");

        client.shutdown().await.expect("client shutdown");
        server.shutdown().await.expect("server shutdown");
    }

    // oracle: spec RFC 8656 §2
    #[cfg(feature = "rustls-ring")]
    #[tokio::test]
    async fn establishes_an_rdpeudp2_tunnel_over_relay_only_turn_path() -> Result<(), Box<dyn Error + Send + Sync>> {
        use crate::multitransport::MultitransportBootstrap;
        use crate::transport::{UdpAcceptConfig, UdpTransportConfig};
        use core::net::{IpAddr, Ipv4Addr, SocketAddr};
        use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
        use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, RequestedProtocol};
        use std::sync::Arc;
        use tokio::net::UdpSocket;
        use tokio_rustls::rustls;
        use turn::Error as TurnError;
        use turn::auth::{AuthHandler, generate_auth_key};
        use turn::relay::relay_static::RelayAddressGeneratorStatic;
        use turn::server::Server;
        use turn::server::config::{ConnConfig, ServerConfig};
        use webrtc_util::vnet::net::Net;

        const TURN_USERNAME: &str = "ironrdp-test";
        const TURN_PASSWORD: &str = "loopback-only-password";
        const TURN_REALM: &str = "ironrdp.test";

        struct TestTurnAuth;

        impl AuthHandler for TestTurnAuth {
            fn auth_handle(&self, username: &str, realm: &str, _source: SocketAddr) -> Result<Vec<u8>, TurnError> {
                if username == TURN_USERNAME && realm == TURN_REALM {
                    Ok(generate_auth_key(TURN_USERNAME, TURN_REALM, TURN_PASSWORD))
                } else {
                    Err(TurnError::ErrFakeErr)
                }
            }
        }

        let turn_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let turn_addr = turn_socket.as_ref().local_addr()?;
        let turn_server = Server::new(ServerConfig {
            conn_configs: vec![ConnConfig {
                conn: turn_socket,
                relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                    relay_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    address: "127.0.0.1".into(),
                    net: Arc::new(Net::new(None)),
                }),
            }],
            realm: TURN_REALM.into(),
            auth_handler: Arc::new(TestTurnAuth),
            channel_bind_timeout: Duration::ZERO,
            alloc_close_notify: None,
        })
        .await?;

        let scenario = async {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let certified_key = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
            let certificate = rustls::pki_types::CertificateDer::from(certified_key.cert.der().to_vec());
            let key = rustls::pki_types::PrivateKeyDer::try_from(certified_key.signing_key.serialize_der())?;
            let tls_config = Arc::new(
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(vec![certificate], key)?,
            );

            let gather_config = IceGatherConfig {
                servers: vec![IceServerConfig::turn(
                    format!("turn:{turn_addr}?transport=udp"),
                    TURN_USERNAME,
                    TURN_PASSWORD,
                )],
                timeout: Duration::from_secs(20),
            };
            let (client_local, server_local) = tokio::join!(
                gather_ice_candidates_with_types(gather_config.clone(), vec![CandidateType::Relay]),
                gather_ice_candidates_with_types(gather_config, vec![CandidateType::Relay])
            );
            let client_local = client_local?;
            let server_local = server_local?;
            if client_local.candidates().is_empty()
                || server_local.candidates().is_empty()
                || client_local
                    .candidates()
                    .iter()
                    .chain(server_local.candidates())
                    .any(|candidate| {
                        !unmarshal_candidate(candidate)
                            .is_ok_and(|candidate| candidate.candidate_type() == CandidateType::Relay)
                    })
            {
                return Err(io::Error::other("TURN gathering did not produce relay-only candidates").into());
            }

            let client_remote = RemoteIceDescription {
                credentials: server_local.credentials().clone(),
                candidates: server_local.candidates().to_vec(),
            };
            let server_remote = RemoteIceDescription {
                credentials: client_local.credentials().clone(),
                candidates: client_local.candidates().to_vec(),
            };
            let tunnel_config = TunnelConfig {
                request_id: 43,
                security_cookie: [0xCD; 16],
            };
            let mut bootstrap = MultitransportBootstrap::new(MultitransportRequestPdu {
                security_header: BasicSecurityHeader {
                    flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
                },
                request_id: tunnel_config.request_id,
                requested_protocol: RequestedProtocol::UdpFecR,
                security_cookie: tunnel_config.security_cookie,
            });
            let client_config = UdpTransportConfig::new(
                "127.0.0.1:0".parse()?,
                "localhost".into(),
                TunnelConfig {
                    request_id: 0,
                    security_cookie: [0; 16],
                },
            );
            let server_config = UdpAcceptConfig {
                tls_config,
                tunnel_config,
                connection_config: ConnectionConfig::default(),
                accept_timeout: Duration::from_secs(25),
            };

            let (client_result, server_result) = tokio::join!(
                bootstrap.connect_with_ice(
                    client_config,
                    client_local,
                    client_remote,
                    IceRole::Controlling,
                    Duration::from_secs(20),
                ),
                accept_udp_with_ice(
                    server_config,
                    server_local,
                    server_remote,
                    IceRole::Controlled,
                    Duration::from_secs(20),
                )
            );
            client_result?;
            if !bootstrap.is_connected() || bootstrap.response_pdu().is_none() {
                return Err(io::Error::other("RDPEUDP2 multitransport bootstrap did not complete").into());
            }
            let client = bootstrap
                .take_transport()
                .ok_or_else(|| io::Error::other("client ICE transport was not established"))?;
            let mut server = server_result?;

            for transport in [&client, &server] {
                let selected_pair = transport
                    .ice_agent
                    .as_ref()
                    .and_then(|agent| agent.agent().get_selected_candidate_pair())
                    .ok_or_else(|| io::Error::other("ICE did not expose its selected candidate pair"))?;
                if selected_pair.local.candidate_type() != CandidateType::Relay
                    || selected_pair.remote.candidate_type() != CandidateType::Relay
                {
                    return Err(io::Error::other("ICE did not select a relay-to-relay candidate pair").into());
                }
            }

            client.send(b"through local TURN".to_vec()).await?;
            let received = tokio::time::timeout(Duration::from_secs(5), server.recv())
                .await?
                .ok_or_else(|| io::Error::other("RDPEUDP2 server tunnel closed"))?;
            if received != b"through local TURN" {
                return Err(io::Error::other("RDPEUDP2 relay payload did not match").into());
            }

            client.shutdown().await?;
            server.shutdown().await?;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        };

        let result = tokio::time::timeout(Duration::from_secs(60), scenario).await;
        let close_result = tokio::time::timeout(Duration::from_secs(5), turn_server.close()).await;
        close_result.map_err(io::Error::other)??;
        result.map_err(io::Error::other)??;
        Ok(())
    }
}
