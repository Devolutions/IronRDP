//! Azure Virtual Desktop (AVD) connection orchestration, WAM token acquisition, and WebSocket gateway transport.

use core::time::Duration;
use std::sync::Arc;

use anyhow::{Context as _, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_connector::{ConnectorErrorExt as _, ConnectorResult, Credentials};
use ironrdp_pdu::rdp::server_redirection::{ELEMENT_TYPE_CERTIFICATE, ENCODING_TYPE_ASN1_DER};
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use rand::random;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT, WWW_AUTHENTICATE};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tracing::{debug, info};
use url::Url;

use crate::config::{Config, Destination, GatewayConfig};
use crate::entra::{EntraAuthSession, EntraPopKey};
use crate::output_channel::OutputEventSender;
use crate::rdp::{
    CliprdrFactoryRef, ConnectOutput, GatewayConnectionInfo, GatewayTunnelPolicy, RdpInputSender, RdpdrFactoryRef,
    SyncGatewayStream, finish_gateway_connection,
};
use crate::redirection::RedirectionHop;

const AVD_USER_AGENT: &str = "com.microsoft.rdc.windows.msrdc.x64/1.2.5716.0 (WINDOWS; 10.0.26100.0)";
const RDG_CLIENT_GENERATION: &str = "Win32#1.2=7";
const DEFAULT_RDS_AAD_RESOURCE_APP_ID: &str = "a4a365df-50f1-4397-bc59-1a1564b8bb9c";
const DEFAULT_WVD_CLIENT_ID: &str = "a85cf173-4192-42f8-81fa-777a763e6e2c";
const DEFAULT_AVD_SHORTPATH_UDP_PORT: u16 = 3390;
const PROTOCOL_CONFIG_JSON: &str = r#"{"ProtocolConfigVersion":1,"ClientType":"com.microsoft.rdc.windows.msrdc.x64","ClientVersion":"1.2.5716.0","TransportCapabilities":{"RDPLegacy":{"ProtocolVersion":"524305","SmilesVersion":"3","TurnCloningMode":2,"RendezvousMode":4},"NanoTransportStackPrototype":{"SupportedVersion":1,"SelectedVersion":1,"EnableDtlsPinning":false}},"SecurityCapabilities":{"SupportedAuthenticationProtocols":["CredSSP","RDSTLS","RDSAAD","StandardTLS"],"AuthOverConnectionControl":true,"RedirectedAuthentication":false}}"#;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Challenge parameters returned by an AVD orchestration gateway (`WWW-Authenticate: Claims:...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdGatewayChallenge {
    pub authority: String,
    pub client_id: String,
    pub resource: String,
    pub site_id: Option<String>,
    pub username_hint: Option<String>,
}

/// Parameters for acquiring an Entra RDS AAD Proof-of-Possession (`PoP`) token for an AVD session host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdHostPopTokenRequest {
    pub authority: String,
    pub client_id: String,
    pub resource_uri: String,
    pub req_cnf: String,
    pub username_hint: Option<String>,
}

/// Cross-platform token provider for Azure Virtual Desktop gateway orchestration and host authentication.
///
/// On Windows, when no custom provider is configured, IronRDP automatically uses the Windows
/// Web Account Manager (`WebAuthenticationCoreManager`).
pub trait AvdTokenProvider: Send + Sync {
    /// Acquire the bearer access token and resolved username for the AVD orchestration gateway.
    fn acquire_gateway_token(&self, challenge: &AvdGatewayChallenge) -> anyhow::Result<(String, String)>;

    /// Acquire a Proof-of-Possession (`PoP`) access token for the target AVD session host.
    fn acquire_host_pop_token(&self, request: &AvdHostPopTokenRequest) -> anyhow::Result<String>;
}

/// STUN or TURN server descriptor parsed from an AVD orchestration `iceServersConfig` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Default)]
struct ClaimsChallenge {
    authority: Option<String>,
    client_id: Option<String>,
    resource: Option<String>,
    site_id: Option<String>,
}

impl ClaimsChallenge {
    fn parse(header_val: &str) -> Option<Self> {
        let rest = header_val
            .trim()
            .strip_prefix("Claims:")
            .or_else(|| header_val.trim().strip_prefix("claims:"))?;
        let mut out = Self::default();
        for part in rest.split(';') {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("Authority") {
                out.authority = Some(value.to_owned());
            } else if key.eq_ignore_ascii_case("Client") {
                out.client_id = Some(value.to_owned());
            } else if key.eq_ignore_ascii_case("Resource") {
                out.resource = Some(value.to_owned());
            } else if key.eq_ignore_ascii_case("Site") {
                out.site_id = Some(value.to_owned());
            }
        }
        Some(out)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvdOrchestrationRequest<'a> {
    application: Option<&'a str>,
    load_balance_info: Option<&'a str>,
    sso_logon_token: Option<&'a str>,
    protocol_config: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvdOrchestrationResponse {
    gateway_location: String,
    gateway_load_balancer_token: Option<String>,
    redirected_server_name: Option<String>,
    redirected_server_cert: Option<String>,
    redirected_auth_blob: Option<String>,
    redirected_auth_guid: Option<String>,
    redirected_username: Option<String>,
    logon_cert_enabled: Option<bool>,
    device_id: Option<String>,
    device_tenant_id: Option<String>,
    udp_port: Option<u16>,
    shortpath_ip_address: Option<String>,
    client_rendezvous_location: Option<String>,
    ice_servers_config: Option<serde_json::Value>,
    azure_instance_network_metadata: Option<serde_json::Value>,
}

pub(crate) async fn connect_avd_gateway(
    config: &Config,
    gw: &GatewayConfig,
    input_sender: &RdpInputSender,
    output_event_sender: &OutputEventSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> ConnectorResult<ConnectOutput> {
    let outcome = orchestrate_and_prepare(config, gw)
        .await
        .map_err(|e| ironrdp_connector::ConnectorError::reason("AVD gateway orchestration", format!("{e:#}")))?;

    let ws_url = outcome.ws_url;
    let host = ws_url
        .host_str()
        .ok_or_else(|| ironrdp_connector::general_err!("AVD gateway URL has no host"))?;
    let port = ws_url.port_or_known_default().unwrap_or(443);

    info!(%host, port, target = %outcome.avd_config.destination, "Connecting to AVD WebSocket gateway");

    let socket = TcpStream::connect((host, port))
        .await
        .map_err(|e| ironrdp_connector::custom_err!("AVD gateway TCP connect", e))?;
    socket
        .set_nodelay(true)
        .map_err(|e| ironrdp_connector::custom_err!("set TCP_NODELAY", e))?;
    let client_addr = socket
        .local_addr()
        .map_err(|e| ironrdp_connector::custom_err!("get socket local address", e))?;

    let (tls_stream, _) =
        ironrdp_tls::upgrade_with_certificate_validation(socket, host, config.certificate_validation())
            .await
            .map_err(|e| ironrdp_connector::custom_err!("AVD gateway TLS upgrade", e))?;

    let mut request = ws_url
        .as_str()
        .into_client_request()
        .map_err(|e| ironrdp_connector::custom_err!("build AVD WebSocket request", e))?;
    {
        let headers = request.headers_mut();
        let bracketed_corr_id = format!("{{{}}}", outcome.correlation_id);
        if let Ok(val) = HeaderValue::from_str(&bracketed_corr_id) {
            headers.insert("RDG-Connection-Id", val.clone());
            headers.insert("RDG-Correlation-Id", val);
        }
        headers.insert("RDG-Auth-Scheme", HeaderValue::from_static("CONNID"));
        headers.insert("RDG-Client-Generation", HeaderValue::from_static(RDG_CLIENT_GENERATION));
        if let Some(cookie) = &outcome.load_balancer_cookie
            && let Ok(val) = HeaderValue::from_str(cookie)
        {
            headers.insert("Cookie", val);
        }
        if let Ok(val) = HeaderValue::from_str(&outcome.correlation_id) {
            headers.insert("x-ms-correlation-id", val);
        }
        if let Some(hint) = &outcome.activity_hint
            && let Ok(val) = HeaderValue::from_str(hint)
        {
            headers.insert("ms-wvd-activity-hint", val);
        }
        if let Some(tenant_id) = &outcome.tenant_id
            && let Ok(val) = HeaderValue::from_str(tenant_id)
        {
            headers.insert("x-ms-tenant-id", val);
        }
        if let Some(arm_path_b64) = &outcome.arm_path_b64
            && let Ok(val) = HeaderValue::from_str(arm_path_b64)
        {
            headers.insert("x-ms-opsarmpath64", val);
        }
        headers.insert("User-Agent", HeaderValue::from_static(AVD_USER_AGENT));
        headers.insert("X-MS-User-Agent", HeaderValue::from_static(AVD_USER_AGENT));
    }

    let (ws, _) = tokio_tungstenite::client_async(request, tls_stream)
        .await
        .map_err(|e| ironrdp_connector::custom_err!("AVD gateway WebSocket upgrade", e))?;
    let gw_stream = SyncGatewayStream(std::sync::Mutex::new(crate::ws::websocket_compat(ws)));

    finish_gateway_connection(
        &outcome.avd_config,
        gw_stream,
        GatewayConnectionInfo {
            client_addr,
            tunnel_policy: GatewayTunnelPolicy::Http(ironrdp_mstsgu::GwTunnelPolicy::default()),
            udp_peer: outcome.shortpath_peer,
        },
        (input_sender, output_event_sender),
        cliprdr_factory,
        rdpdr_factory,
        auto_reconnect_cookie,
    )
    .await
}

struct PreparedAvdConnection {
    ws_url: Url,
    correlation_id: String,
    load_balancer_cookie: Option<String>,
    activity_hint: Option<String>,
    tenant_id: Option<String>,
    arm_path_b64: Option<String>,
    shortpath_peer: Option<core::net::SocketAddr>,
    avd_config: Config,
}

async fn orchestrate_and_prepare(config: &Config, gw: &GatewayConfig) -> anyhow::Result<PreparedAvdConnection> {
    let ps = &config.properties;
    let arm_path = ps.get::<&str>("armpath").filter(|s| !s.is_empty());
    let endpoint_pool = ps.get::<&str>("wvd endpoint pool").filter(|s| !s.is_empty());
    let aad_tenant_id = ps.get::<&str>("aadtenantid").filter(|s| !s.is_empty());
    let remote_app = ps.remote_application_program().filter(|s| !s.is_empty());
    let sso_logon_token = ps.get::<&str>("ironrdp_avd_sso_logon_token").filter(|s| !s.is_empty());
    let load_balance_info = config
        .load_balance_info
        .as_deref()
        .or_else(|| ps.get::<&str>("loadbalanceinfo"))
        .filter(|s| !s.is_empty());

    let path = if arm_path.is_some() {
        "/api/arm/v2/connections"
    } else {
        "/api/v2/connections"
    };
    let orchestration_url = format!("https://{}{path}", gw.endpoint);
    let correlation_id = random_guid_string();
    let activity_hint = endpoint_pool.map(|pool| format!("ms-wvd-hp:{pool}"));
    let arm_path_b64 = arm_path.map(|p| STANDARD.encode(p.as_bytes()));

    let http = crate::http_client_builder()
        .context("configure AVD orchestration HTTPS client")?
        .https_only(true)
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("create AVD orchestration HTTPS client")?;

    let req_body = serde_json::to_vec(&AvdOrchestrationRequest {
        application: remote_app,
        load_balance_info,
        sso_logon_token,
        protocol_config: PROTOCOL_CONFIG_JSON,
    })?;

    let build_post = |token: Option<&str>| {
        let mut req = http
            .post(&orchestration_url)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .header(USER_AGENT, AVD_USER_AGENT)
            .header("X-MS-User-Agent", AVD_USER_AGENT)
            .header("x-ms-correlation-id", &correlation_id)
            .header("RDG-Client-Generation", RDG_CLIENT_GENERATION)
            .header("x-ms-ClientCapabilities", "ContextualAccess")
            .body(req_body.clone());
        if let Some(tenant_id) = aad_tenant_id {
            req = req.header("x-ms-tenant-id", tenant_id);
        }
        if let Some(hint) = &activity_hint {
            req = req.header("ms-wvd-activity-hint", hint);
        }
        if let Some(arm64) = &arm_path_b64 {
            req = req.header("x-ms-opsarmpath64", arm64);
        }
        if let Some(token) = token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        req
    };

    debug!(url = %orchestration_url, "Sending initial AVD orchestration probe");
    let initial_resp = build_post(None)
        .send()
        .await
        .context("send initial AVD orchestration request")?;

    let mut challenge = ClaimsChallenge::default();
    if initial_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        for val in initial_resp.headers().get_all(WWW_AUTHENTICATE) {
            if let Ok(s) = val.to_str()
                && let Some(parsed) = ClaimsChallenge::parse(s)
            {
                challenge = parsed;
                break;
            }
        }
    }

    let authority = challenge.authority.unwrap_or_else(|| match aad_tenant_id {
        Some(tid) => format!("https://login.windows.net/{tid}"),
        None => "https://login.windows.net/common".to_owned(),
    });
    let client_id = challenge.client_id.unwrap_or_else(|| DEFAULT_WVD_CLIENT_ID.to_owned());
    let resource = challenge
        .resource
        .unwrap_or_else(|| "https://www.wvd.microsoft.com".to_owned());
    let site_id = challenge.site_id;

    let username_hint = match &config.connector.credentials {
        Credentials::UsernamePassword { username, .. } if !username.is_empty() => Some(username.clone()),
        _ => gw.username.as_str().into_non_empty().map(str::to_owned),
    };

    let gw_challenge = AvdGatewayChallenge {
        authority: authority.clone(),
        client_id: client_id.clone(),
        resource,
        site_id,
        username_hint: username_hint.clone(),
    };

    let (gw_token, wam_username) =
        if let Some(prop_token) = ps.get::<&str>("ironrdp_avd_gateway_token").filter(|s| !s.is_empty()) {
            (prop_token.to_owned(), username_hint.clone().unwrap_or_default())
        } else if let Some(provider) = config.avd_token_provider.as_ref() {
            let provider = Arc::clone(provider);
            let gw_challenge_clone = gw_challenge.clone();
            tokio::task::spawn_blocking(move || provider.acquire_gateway_token(&gw_challenge_clone))
                .await
                .context("join custom AVD gateway token provider")??
        } else {
            acquire_gateway_token_via_wam(
                gw_challenge.authority,
                gw_challenge.client_id,
                gw_challenge.resource,
                gw_challenge.site_id,
                gw_challenge.username_hint,
            )
            .await?
        };

    debug!(url = %orchestration_url, user = %wam_username, "Sending authenticated AVD orchestration request");
    let auth_resp = build_post(Some(&gw_token))
        .send()
        .await
        .context("send authenticated AVD orchestration request")?;
    let status = auth_resp.status();
    let body_bytes = auth_resp
        .bytes()
        .await
        .context("read AVD orchestration response body")?;
    ensure!(
        status.is_success(),
        "AVD orchestration returned HTTP {status}: {}",
        String::from_utf8_lossy(&body_bytes)
    );

    let orch: AvdOrchestrationResponse =
        serde_json::from_slice(&body_bytes).context("decode AVD orchestration response JSON")?;

    let mut ws_url = Url::parse(&orch.gateway_location).context("parse AVD gatewayLocation URL")?;
    if ws_url.scheme() == "https" {
        let _ = ws_url.set_scheme("wss");
    } else if ws_url.scheme() == "http" {
        let _ = ws_url.set_scheme("ws");
    }

    let target_server = orch
        .redirected_server_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| config.destination.name());

    let target_certificate = orch
        .redirected_server_cert
        .as_deref()
        .and_then(decode_redirected_server_cert)
        .or_else(|| {
            config
                .redirection
                .as_ref()
                .and_then(|hop| hop.target_certificate.clone())
        });

    let password_cookie = orch
        .redirected_auth_blob
        .as_deref()
        .and_then(decode_redirected_auth_blob)
        .or_else(|| config.redirection.as_ref().and_then(|hop| hop.password_cookie.clone()));

    let redirected_session_id = config.redirection.as_ref().map_or(0, |hop| hop.session_id);

    let resolved_username = orch
        .redirected_username
        .as_deref()
        .filter(|s| !s.is_empty())
        .map_or(wam_username, str::to_owned);

    let ice_servers = orch
        .ice_servers_config
        .as_ref()
        .map(parse_ice_servers_config)
        .unwrap_or_default();
    if !ice_servers.is_empty() || orch.client_rendezvous_location.is_some() {
        debug!(
            ice_server_count = ice_servers.len(),
            rendezvous = ?orch.client_rendezvous_location,
            "Parsed AVD Shortpath ICE/rendezvous metadata from orchestration"
        );
    }

    let shortpath_peer = resolve_shortpath_peer(config, &orch);

    let mut avd_config = config.clone();
    avd_config.destination = Destination::from_parts(target_server.to_owned(), 3389);
    avd_config.load_balance_info = None;
    avd_config.use_redirection_server_name = true;
    avd_config.redirection = Some(RedirectionHop {
        connect_hosts: Vec::new(),
        session_id: redirected_session_id,
        password_cookie,
        target_certificate,
    });
    if let Some(auth_guid) = orch.redirected_auth_guid.as_deref().filter(|s| !s.is_empty()) {
        avd_config.properties.insert("redirectionguid", auth_guid.to_owned());
    }
    if let Some(logon_cert) = orch.logon_cert_enabled {
        avd_config
            .properties
            .insert("uselogoncertificate", i64::from(logon_cert));
    }
    avd_config.connector.credentials = match &config.connector.credentials {
        Credentials::UsernamePassword { password, .. } => Credentials::UsernamePassword {
            username: resolved_username.clone(),
            password: password.clone(),
        },
        other => other.clone(),
    };

    let needs_entra_auth = avd_config.entra_auth.is_some() || ps.get::<bool>("enablerdsaadauth").unwrap_or(false);
    let entra_expired_or_missing = avd_config
        .entra_auth
        .as_ref()
        .is_none_or(|session| session.is_expired());
    if needs_entra_auth && entra_expired_or_missing {
        let pop_key = tokio::task::spawn_blocking(EntraPopKey::generate)
            .await
            .context("join Entra PoP key generation")??;
        let req_cnf = pop_key.req_cnf()?;
        let rds_resource_uri = if let Some(device_id) = orch.device_id.as_deref().filter(|s| !s.is_empty()) {
            format!("ms-device-service://{DEFAULT_RDS_AAD_RESOURCE_APP_ID}/id/{device_id}")
        } else {
            format!("ms-device-service://{DEFAULT_RDS_AAD_RESOURCE_APP_ID}/name/{target_server}")
        };
        let tenant_for_auth = orch
            .device_tenant_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(aad_tenant_id);
        let aadj_authority = match tenant_for_auth {
            Some(tid) => format!("https://login.windows.net/{tid}"),
            None => authority,
        };
        let pop_req = AvdHostPopTokenRequest {
            authority: aadj_authority,
            client_id,
            resource_uri: rds_resource_uri.clone(),
            req_cnf,
            username_hint: Some(resolved_username).filter(|s| !s.is_empty()),
        };
        let pop_token = if let Some(prop_pop) = ps.get::<&str>("ironrdp_entra_pop_token").filter(|s| !s.is_empty()) {
            prop_pop.to_owned()
        } else if let Some(provider) = config.avd_token_provider.as_ref() {
            let provider = Arc::clone(provider);
            tokio::task::spawn_blocking(move || provider.acquire_host_pop_token(&pop_req))
                .await
                .context("join custom AVD host PoP token provider")??
        } else {
            acquire_aadj_pop_token_via_wam(
                pop_req.authority,
                pop_req.client_id,
                pop_req.resource_uri,
                pop_req.req_cnf,
                pop_req.username_hint,
            )
            .await?
        };
        let entra_session = EntraAuthSession::from_pop_token(pop_token, pop_key, rds_resource_uri, tenant_for_auth)?;
        avd_config.entra_auth = Some(Arc::new(entra_session));
        avd_config.connector.enable_tls = true;
        avd_config.connector.enable_credssp = false;
    }

    Ok(PreparedAvdConnection {
        ws_url,
        correlation_id,
        load_balancer_cookie: orch.gateway_load_balancer_token,
        activity_hint,
        tenant_id: aad_tenant_id.map(str::to_owned),
        arm_path_b64,
        shortpath_peer,
        avd_config,
    })
}

trait StrExt {
    fn into_non_empty(self) -> Option<Self>
    where
        Self: Sized;
}

impl StrExt for &str {
    fn into_non_empty(self) -> Option<Self> {
        (!self.is_empty()).then_some(self)
    }
}

fn decode_redirected_server_cert(outer_b64: &str) -> Option<Vec<u8>> {
    let utf16_bytes = STANDARD.decode(outer_b64.trim()).ok()?;
    if !utf16_bytes.len().is_multiple_of(2) {
        return None;
    }
    let code_units: Vec<u16> = utf16_bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&u| u != 0)
        .collect();
    let inner_b64 = String::from_utf16(&code_units).ok()?;
    let container_bytes = STANDARD.decode(inner_b64.trim()).ok()?;
    let mut rest = container_bytes.as_slice();
    while rest.len() >= 12 {
        let element_type = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let encoding = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
        let element_size = usize::try_from(u32::from_le_bytes([rest[8], rest[9], rest[10], rest[11]])).ok()?;
        rest = &rest[12..];
        if rest.len() < element_size {
            return None;
        }
        let data = &rest[..element_size];
        rest = &rest[element_size..];
        if element_type == ELEMENT_TYPE_CERTIFICATE && encoding == ENCODING_TYPE_ASN1_DER {
            return Some(data.to_vec());
        }
    }
    None
}

fn decode_redirected_auth_blob(outer_b64: &str) -> Option<Vec<u8>> {
    let decoded = STANDARD.decode(outer_b64.trim()).ok()?;
    if decoded.is_empty() {
        return None;
    }
    if decoded.len().is_multiple_of(2) {
        let code_units: Vec<u16> = decoded
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&u| u != 0)
            .collect();
        if !code_units.is_empty()
            && let Ok(inner_b64) = String::from_utf16(&code_units)
            && let Ok(inner_bytes) = STANDARD.decode(inner_b64.trim())
            && !inner_bytes.is_empty()
        {
            return Some(inner_bytes);
        }
    }
    Some(decoded)
}

fn parse_ice_servers_config(val: &serde_json::Value) -> Vec<AvdIceServer> {
    let parsed_json: serde_json::Value;
    let root = match val {
        serde_json::Value::String(s) => {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
                return Vec::new();
            };
            parsed_json = v;
            &parsed_json
        }
        other => other,
    };

    let entries = if let Some(arr) = root.as_array() {
        arr.as_slice()
    } else if let Some(arr) = root
        .get("iceServers")
        .or_else(|| root.get("IceServers"))
        .and_then(serde_json::Value::as_array)
    {
        arr.as_slice()
    } else {
        return Vec::new();
    };

    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let urls_val = entry
            .get("urls")
            .or_else(|| entry.get("Urls"))
            .or_else(|| entry.get("url"))
            .or_else(|| entry.get("Url"));
        let urls: Vec<String> = match urls_val {
            Some(serde_json::Value::String(s)) if !s.is_empty() => vec![s.clone()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        if urls.is_empty() {
            continue;
        }
        let username = entry
            .get("username")
            .or_else(|| entry.get("Username"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let credential = entry
            .get("credential")
            .or_else(|| entry.get("Credential"))
            .or_else(|| entry.get("password"))
            .or_else(|| entry.get("Password"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        out.push(AvdIceServer {
            urls,
            username,
            credential,
        });
    }
    out
}

fn resolve_shortpath_peer(config: &Config, orch: &AvdOrchestrationResponse) -> Option<core::net::SocketAddr> {
    #[cfg(feature = "udp")]
    let udp_enabled = config.udp_transport_enabled;
    #[cfg(not(feature = "udp"))]
    let udp_enabled = false;
    let _ = config;
    if !udp_enabled {
        return None;
    }

    let port = orch
        .udp_port
        .filter(|&p| p != 0)
        .unwrap_or(DEFAULT_AVD_SHORTPATH_UDP_PORT);

    if let Some(ip_str) = orch
        .shortpath_ip_address
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(sock) = ip_str.parse::<core::net::SocketAddr>() {
            return Some(sock);
        }
        if let Ok(ip) = ip_str.parse::<core::net::IpAddr>() {
            return Some(core::net::SocketAddr::new(ip, port));
        }
    }

    if let Some(meta) = &orch.azure_instance_network_metadata
        && let Some(ip) = extract_ip_from_network_metadata(meta)
    {
        return Some(core::net::SocketAddr::new(ip, port));
    }

    None
}

fn extract_ip_from_network_metadata(meta: &serde_json::Value) -> Option<core::net::IpAddr> {
    let parsed_json: serde_json::Value;
    let root = match meta {
        serde_json::Value::String(s) => {
            parsed_json = serde_json::from_str(s).ok()?;
            &parsed_json
        }
        other => other,
    };
    for key in [
        "publicIpAddress",
        "PublicIpAddress",
        "privateIpAddress",
        "PrivateIpAddress",
        "ipAddress",
    ] {
        if let Some(s) = root.get(key).and_then(serde_json::Value::as_str)
            && let Ok(ip) = s.trim().parse::<core::net::IpAddr>()
        {
            return Some(ip);
        }
    }
    None
}

fn random_guid_string() -> String {
    let mut b = random::<[u8; 16]>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

#[cfg(windows)]
async fn acquire_gateway_token_via_wam(
    authority: String,
    client_id: String,
    resource: String,
    site_id: Option<String>,
    username_hint: Option<String>,
) -> anyhow::Result<(String, String)> {
    tokio::task::spawn_blocking(move || {
        wam_windows::acquire_token(
            &authority,
            &client_id,
            "",
            &resource,
            site_id.as_deref(),
            Some(r#"{"access_token":{"xms_cc":{"values":["cp1"]}}}"#),
            None,
            username_hint.as_deref(),
        )
    })
    .await
    .context("join WAM gateway token acquisition")?
}

#[cfg(not(windows))]
async fn acquire_gateway_token_via_wam(
    _authority: String,
    _client_id: String,
    _resource: String,
    _site_id: Option<String>,
    _username_hint: Option<String>,
) -> anyhow::Result<(String, String)> {
    Err(anyhow!("AVD gateway WAM authentication requires Windows"))
}

#[cfg(windows)]
async fn acquire_aadj_pop_token_via_wam(
    authority: String,
    client_id: String,
    resource_uri: String,
    req_cnf: String,
    username_hint: Option<String>,
) -> anyhow::Result<String> {
    let (token, _) = tokio::task::spawn_blocking(move || {
        wam_windows::acquire_token(
            &authority,
            &client_id,
            "user_impersonation",
            &resource_uri,
            None,
            None,
            Some(&req_cnf),
            username_hint.as_deref(),
        )
    })
    .await
    .context("join WAM AADJ PoP token acquisition")??;
    Ok(token)
}

#[cfg(not(windows))]
async fn acquire_aadj_pop_token_via_wam(
    _authority: String,
    _client_id: String,
    _resource_uri: String,
    _req_cnf: String,
    _username_hint: Option<String>,
) -> anyhow::Result<String> {
    Err(anyhow!("AVD Entra RDS AAD WAM authentication requires Windows"))
}

#[cfg(windows)]
mod wam_windows {
    use anyhow::{Context as _, anyhow, bail};
    use tracing::{debug, info};
    use windows::Security::Authentication::Web::Core::{
        FindAllWebAccountsStatus, WebAuthenticationCoreManager, WebProviderError, WebTokenRequest,
        WebTokenRequestResult, WebTokenRequestStatus,
    };
    use windows::Security::Credentials::{WebAccount, WebAccountProvider};
    use windows::core::{HSTRING, h};

    #[expect(clippy::too_many_arguments)]
    pub(super) fn acquire_token(
        authority: &str,
        client_id: &str,
        scope: &str,
        resource: &str,
        site_id: Option<&str>,
        claims: Option<&str>,
        req_cnf: Option<&str>,
        username_hint: Option<&str>,
    ) -> anyhow::Result<(String, String)> {
        let provider = WebAuthenticationCoreManager::FindAccountProviderWithAuthorityAsync(
            h!("https://login.windows.net"),
            &HSTRING::from(authority),
        )
        .context("FindAccountProviderWithAuthorityAsync")?
        .join()
        .context("await FindAccountProviderWithAuthorityAsync")?;

        let accounts_result =
            WebAuthenticationCoreManager::FindAllAccountsWithClientIdAsync(&provider, &HSTRING::from(client_id))
                .context("FindAllAccountsWithClientIdAsync")?
                .join()
                .context("await FindAllAccountsWithClientIdAsync")?;

        if accounts_result.Status()? == FindAllWebAccountsStatus::Success {
            let accounts = accounts_result.Accounts()?;
            let count = accounts.Size()?;
            let mut candidates: Vec<WebAccount> = Vec::with_capacity(count as usize);
            for i in 0..count {
                if let Ok(acc) = accounts.GetAt(i) {
                    candidates.push(acc);
                }
            }
            if let Some(hint) = username_hint {
                candidates.sort_by_key(|acc| {
                    let matches = acc
                        .UserName()
                        .ok()
                        .is_some_and(|u| u.to_string().eq_ignore_ascii_case(hint));
                    !matches
                });
            }

            for account in &candidates {
                let acc_user = account.UserName().map(|s| s.to_string()).unwrap_or_default();
                if let Some(hint) = username_hint
                    && !acc_user.eq_ignore_ascii_case(hint)
                    && candidates.iter().any(|a| {
                        a.UserName()
                            .ok()
                            .is_some_and(|u| u.to_string().eq_ignore_ascii_case(hint))
                    })
                {
                    continue;
                }
                let request = build_token_request(
                    &provider,
                    client_id,
                    scope,
                    authority,
                    resource,
                    site_id,
                    claims,
                    req_cnf,
                    Some(&acc_user),
                    true,
                )?;
                let result = WebAuthenticationCoreManager::GetTokenSilentlyWithWebAccountAsync(&request, account)
                    .context("GetTokenSilentlyWithWebAccountAsync")?
                    .join()
                    .context("await GetTokenSilentlyWithWebAccountAsync")?;
                if result.ResponseStatus()? == WebTokenRequestStatus::Success {
                    let token = extract_token(&result)?;
                    debug!(user = %acc_user, resource = %resource, "Acquired WAM token silently");
                    return Ok((token, acc_user));
                }
            }
        }

        info!(resource = %resource, "Silent WAM token acquisition did not succeed; requesting interactive WAM token");
        let request = build_token_request(
            &provider,
            client_id,
            scope,
            authority,
            resource,
            site_id,
            claims,
            req_cnf,
            username_hint,
            false,
        )?;
        let result = WebAuthenticationCoreManager::RequestTokenAsync(&request)
            .context("RequestTokenAsync")?
            .join()
            .context("await RequestTokenAsync")?;
        let status = result.ResponseStatus()?;
        if status != WebTokenRequestStatus::Success {
            let err_msg = result
                .ResponseError()
                .ok()
                .and_then(|e: WebProviderError| Some((e.ErrorCode().ok()?, e.ErrorMessage().ok()?.to_string())))
                .map(|(code, msg)| format!(" (0x{code:08x}: {msg})"))
                .unwrap_or_default();
            bail!("WAM token request failed with status {status:?}{err_msg}");
        }
        let token = extract_token(&result)?;
        let user = result.ResponseData()?.GetAt(0)?.WebAccount()?.UserName()?.to_string();
        Ok((token, user))
    }

    #[expect(clippy::too_many_arguments)]
    fn build_token_request(
        provider: &WebAccountProvider,
        client_id: &str,
        scope: &str,
        authority: &str,
        resource: &str,
        site_id: Option<&str>,
        claims: Option<&str>,
        req_cnf: Option<&str>,
        login_hint: Option<&str>,
        silent: bool,
    ) -> anyhow::Result<WebTokenRequest> {
        let request = WebTokenRequest::Create(provider, &HSTRING::from(scope), &HSTRING::from(client_id))
            .context("WebTokenRequest::Create")?;
        let props = request.Properties().context("WebTokenRequest::Properties")?;
        props.Insert(h!("authority"), &HSTRING::from(authority))?;
        props.Insert(h!("resource"), &HSTRING::from(resource))?;
        if let Some(site_id) = site_id.filter(|s| !s.is_empty()) {
            props.Insert(h!("site_id"), &HSTRING::from(site_id))?;
        }
        if let Some(claims) = claims.filter(|s| !s.is_empty()) {
            props.Insert(h!("claims"), &HSTRING::from(claims))?;
        }
        if let Some(req_cnf) = req_cnf.filter(|s| !s.is_empty()) {
            props.Insert(h!("req_cnf"), &HSTRING::from(req_cnf))?;
        }
        if !silent {
            if let Some(hint) = login_hint.filter(|s| !s.is_empty()) {
                props.Insert(h!("LoginHint"), &HSTRING::from(hint))?;
                props.Insert(h!("prompt"), h!("no_select"))?;
            } else {
                props.Insert(h!("prompt"), h!("select_account"))?;
            }
            props.Insert(h!("display"), h!("popup"))?;
        }
        Ok(request)
    }

    fn extract_token(result: &WebTokenRequestResult) -> anyhow::Result<String> {
        let data = result.ResponseData().context("WebTokenRequestResult::ResponseData")?;
        if data.Size()? == 0 {
            return Err(anyhow!("WAM returned an empty ResponseData vector"));
        }
        let token = data.GetAt(0)?.Token()?.to_string();
        if token.is_empty() {
            return Err(anyhow!("WAM returned an empty token string"));
        }
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

    use super::*;

    #[test]
    fn parse_claims_challenge_header() {
        let header = "Claims:Authority=https://login.windows.net/4c5a5ec1-9ac5-4612-9b4c-6bed178bb65a;Client=a85cf173-4192-42f8-81fa-777a763e6e2c;Redirect=https://www.wvd.microsoft.com/webclient;Resource=https://www.wvd.microsoft.com;Site=501413";
        let challenge = ClaimsChallenge::parse(header).unwrap();
        assert_eq!(
            challenge.authority.as_deref(),
            Some("https://login.windows.net/4c5a5ec1-9ac5-4612-9b4c-6bed178bb65a")
        );
        assert_eq!(
            challenge.client_id.as_deref(),
            Some("a85cf173-4192-42f8-81fa-777a763e6e2c")
        );
        assert_eq!(challenge.resource.as_deref(), Some("https://www.wvd.microsoft.com"));
        assert_eq!(challenge.site_id.as_deref(), Some("501413"));
    }

    #[test]
    fn decode_redirected_cert_and_auth_blob_roundtrip() {
        let cert_der = b"\x30\x82\x01\x0a\x02\x82\x01\x01\x00\xab\xcd\xef";
        let mut container = Vec::new();
        container.extend_from_slice(&ELEMENT_TYPE_CERTIFICATE.to_le_bytes());
        container.extend_from_slice(&ENCODING_TYPE_ASN1_DER.to_le_bytes());
        container.extend_from_slice(&(u32::try_from(cert_der.len()).unwrap()).to_le_bytes());
        container.extend_from_slice(cert_der);
        let inner_b64 = STANDARD.encode(&container);
        let mut utf16_bytes = Vec::new();
        for u in inner_b64.encode_utf16() {
            utf16_bytes.extend_from_slice(&u.to_le_bytes());
        }
        utf16_bytes.extend_from_slice(&[0, 0]);
        let outer_b64 = STANDARD.encode(&utf16_bytes);

        assert_eq!(decode_redirected_server_cert(&outer_b64).unwrap(), cert_der);

        let cookie = b"broker-auth-cookie-bytes";
        let inner_cookie_b64 = STANDARD.encode(cookie);
        let mut cookie_utf16 = Vec::new();
        for u in inner_cookie_b64.encode_utf16() {
            cookie_utf16.extend_from_slice(&u.to_le_bytes());
        }
        let outer_cookie_b64 = STANDARD.encode(&cookie_utf16);
        assert_eq!(decode_redirected_auth_blob(&outer_cookie_b64).unwrap(), cookie);
    }

    #[test]
    fn parse_ice_servers_and_shortpath_network_metadata() {
        let ice_json = serde_json::json!({
            "iceServers": [
                {
                    "urls": ["stun:world.turn.wvd.microsoft.com:3478", "turn:world.turn.wvd.microsoft.com:3478?transport=udp"],
                    "username": "turn-user",
                    "credential": "turn-secret"
                }
            ]
        });
        let servers = parse_ice_servers_config(&ice_json);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].urls.len(), 2);
        assert_eq!(servers[0].username.as_deref(), Some("turn-user"));
        assert_eq!(servers[0].credential.as_deref(), Some("turn-secret"));

        let net_meta = serde_json::json!({ "publicIpAddress": "20.50.100.10" });
        assert_eq!(
            extract_ip_from_network_metadata(&net_meta),
            Some("20.50.100.10".parse().unwrap())
        );
    }

    struct MockAvdTokenProvider;

    impl AvdTokenProvider for MockAvdTokenProvider {
        fn acquire_gateway_token(&self, challenge: &AvdGatewayChallenge) -> anyhow::Result<(String, String)> {
            assert_eq!(challenge.resource, "https://www.wvd.microsoft.com");
            Ok(("mock-gw-token".to_owned(), "user1@downhillpro.xyz".to_owned()))
        }

        fn acquire_host_pop_token(&self, request: &AvdHostPopTokenRequest) -> anyhow::Result<String> {
            assert!(request.resource_uri.starts_with("ms-device-service://"));
            assert!(!request.req_cnf.is_empty());
            Ok("mock-pop-token".to_owned())
        }
    }

    #[test]
    fn custom_avd_token_provider_roundtrip() {
        let provider: Arc<dyn AvdTokenProvider> = Arc::new(MockAvdTokenProvider);
        let (gw_tok, user) = provider
            .acquire_gateway_token(&AvdGatewayChallenge {
                authority: "https://login.windows.net/common".to_owned(),
                client_id: DEFAULT_WVD_CLIENT_ID.to_owned(),
                resource: "https://www.wvd.microsoft.com".to_owned(),
                site_id: Some("501413".to_owned()),
                username_hint: None,
            })
            .unwrap();
        assert_eq!(gw_tok, "mock-gw-token");
        assert_eq!(user, "user1@downhillpro.xyz");
    }
}
