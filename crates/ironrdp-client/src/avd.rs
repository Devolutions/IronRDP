//! Azure Virtual Desktop (AVD) connection orchestration, WAM token acquisition, and WebSocket gateway transport.
//!
//! Orchestration and challenge parsers in this module are property-tested in `ironrdp-testsuite-extra`
//! rather than fuzzed in `ironrdp-fuzzing`, which does not depend on `ironrdp-client`.

use core::time::Duration;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, anyhow, bail, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_connector::{ConnectorErrorExt as _, ConnectorResult, Credentials};
use ironrdp_pdu::rdp::server_redirection::{ELEMENT_TYPE_CERTIFICATE, ENCODING_TYPE_ASN1_DER};
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use picky::hash::HashAlgorithm;
use rand::random;
use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT, WWW_AUTHENTICATE};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
#[cfg(windows)]
use tracing::warn;
use tracing::{debug, info};
use url::Url;
use zeroize::Zeroizing;

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
/// Web Account Manager (`WebAuthenticationCoreManager`). On Linux and macOS (or when selected via
/// `ironrdp_avd_auth_method`), IronRDP uses [`OAuthAvdTokenProvider`].
pub trait AvdTokenProvider: Send + Sync {
    /// Acquire the bearer access token and resolved username for the AVD orchestration gateway.
    fn acquire_gateway_token(&self, challenge: &AvdGatewayChallenge) -> anyhow::Result<(String, String)>;

    /// Acquire a Proof-of-Possession (`PoP`) access token for the target AVD session host.
    fn acquire_host_pop_token(&self, request: &AvdHostPopTokenRequest) -> anyhow::Result<String>;
}

/// Interactive OAuth2 flow used by [`OAuthAvdTokenProvider`] when no valid cached refresh token exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvdOAuthMode {
    /// Try system-browser loopback PKCE (`http://localhost:<port>`) first, falling back to Device Code flow if headless.
    #[default]
    Auto,
    /// Require system-browser loopback Authorization Code + PKCE.
    LoopbackPkce,
    /// Use OAuth2 Device Code flow (`/oauth2/v2.0/devicecode`).
    DeviceCode,
}

impl AvdOAuthMode {
    fn from_property(val: Option<&str>) -> Option<Self> {
        match val?.trim().to_ascii_lowercase().as_str() {
            "oauth" | "auto" => Some(Self::Auto),
            "pkce" | "browser" | "loopback" => Some(Self::LoopbackPkce),
            "devicecode" | "device-code" | "device_code" => Some(Self::DeviceCode),
            _ => None,
        }
    }
}

/// Cross-platform OAuth2 token provider for AVD using Refresh Tokens, Loopback PKCE, and Device Code flow.
pub struct OAuthAvdTokenProvider {
    http: reqwest::Client,
    mode: AvdOAuthMode,
    cache_path: Option<PathBuf>,
    memory_cache: Mutex<HashMap<String, (Zeroizing<String>, String)>>,
    allow_loopback_http: bool,
}

impl OAuthAvdTokenProvider {
    /// Create a new cross-platform OAuth2 AVD token provider using in-memory refresh-token caching
    /// (or the path in `IRONRDP_AVD_TOKEN_CACHE` when explicitly set).
    pub fn new(mode: AvdOAuthMode) -> anyhow::Result<Self> {
        Self::with_cache_path(mode, default_token_cache_path())
    }

    /// Create a new cross-platform OAuth2 AVD token provider with an explicit cache file path (or `None` for in-memory only).
    ///
    /// When `cache_path` is `Some`, refresh tokens are persisted as plaintext JSON at that path (with `0600`
    /// owner-only permissions on Unix). Pass `None` to keep refresh tokens strictly in memory.
    pub fn with_cache_path(mode: AvdOAuthMode, cache_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let http = crate::http_client_builder()
            .context("configure AVD OAuth2 HTTP client")?
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("build AVD OAuth2 HTTP client")?;
        let memory_cache = cache_path.as_deref().map(load_token_cache_file).unwrap_or_default();
        Ok(Self {
            http,
            mode,
            cache_path,
            memory_cache: Mutex::new(memory_cache),
            allow_loopback_http: false,
        })
    }

    #[doc(hidden)]
    pub fn new_loopback_test(mode: AvdOAuthMode, cache_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let http = crate::http_client_builder()
            .context("configure AVD OAuth2 test HTTP client")?
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("build AVD OAuth2 test HTTP client")?;
        let memory_cache = cache_path.as_deref().map(load_token_cache_file).unwrap_or_default();
        Ok(Self {
            http,
            mode,
            cache_path,
            memory_cache: Mutex::new(memory_cache),
            allow_loopback_http: true,
        })
    }

    /// Acquire the AVD gateway bearer token asynchronously using cached refresh token, loopback PKCE, or device code.
    pub async fn acquire_gateway_token_async(
        &self,
        challenge: &AvdGatewayChallenge,
    ) -> anyhow::Result<(String, String)> {
        let authority = normalize_oauth_authority_with_options(&challenge.authority, self.allow_loopback_http)?;
        let scope = format!(
            "{} offline_access openid profile",
            resource_default_scope(&challenge.resource)
        );
        let claims = Some(r#"{"access_token":{"xms_cc":{"values":["cp1"]}}}"#);

        if let Some((cached_rt, cached_user)) = self.lookup_cached_refresh_token(
            &authority,
            &challenge.client_id,
            challenge.username_hint.as_deref(),
            false,
        ) {
            match self
                .exchange_refresh_token(&authority, &challenge.client_id, &cached_rt, &scope, claims, None)
                .await
            {
                Ok(resp) => {
                    let user = extract_username_from_tokens(&resp)
                        .or_else(|| challenge.username_hint.clone())
                        .unwrap_or(cached_user);
                    self.store_refresh_token(
                        &authority,
                        &challenge.client_id,
                        resp.refresh_token.as_deref().unwrap_or(&cached_rt),
                        &user,
                    );
                    debug!(user = %user, "Acquired AVD gateway token silently via cached OAuth2 refresh token");
                    return Ok((resp.access_token, user));
                }
                Err(error) => {
                    debug!(%error, "Cached AVD OAuth2 refresh token could not be exchanged; falling back to interactive flow");
                }
            }
        }

        let resp = match self.mode {
            AvdOAuthMode::LoopbackPkce => {
                self.acquire_via_loopback_pkce(
                    &authority,
                    &challenge.client_id,
                    &scope,
                    claims,
                    challenge.username_hint.as_deref(),
                )
                .await?
            }
            AvdOAuthMode::DeviceCode => {
                self.acquire_via_device_code(&authority, &challenge.client_id, &scope, claims)
                    .await?
            }
            AvdOAuthMode::Auto => match self
                .acquire_via_loopback_pkce(
                    &authority,
                    &challenge.client_id,
                    &scope,
                    claims,
                    challenge.username_hint.as_deref(),
                )
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    debug!(%error, "Loopback PKCE flow unavailable; falling back to OAuth2 Device Code flow");
                    self.acquire_via_device_code(&authority, &challenge.client_id, &scope, claims)
                        .await?
                }
            },
        };

        let user = extract_username_from_tokens(&resp)
            .or_else(|| challenge.username_hint.clone())
            .unwrap_or_default();
        if let Some(rt) = resp.refresh_token.as_deref().filter(|s| !s.is_empty()) {
            self.store_refresh_token(&authority, &challenge.client_id, rt, &user);
        }
        Ok((resp.access_token, user))
    }

    /// Acquire the RDS AAD host PoP token asynchronously by exchanging the refresh token with `req_cnf`.
    pub async fn acquire_host_pop_token_async(&self, request: &AvdHostPopTokenRequest) -> anyhow::Result<String> {
        let authority = normalize_oauth_authority_with_options(&request.authority, self.allow_loopback_http)?;
        let scope = format!("{} offline_access", host_impersonation_scope(&request.resource_uri));
        let (refresh_token, matched_user) = self
            .lookup_cached_refresh_token(&authority, &request.client_id, request.username_hint.as_deref(), true)
            .ok_or_else(|| anyhow!("no OAuth2 refresh token available to mint AVD host PoP token"))?;

        let resp = self
            .exchange_refresh_token(
                &authority,
                &request.client_id,
                &refresh_token,
                &scope,
                None,
                Some(&request.req_cnf),
            )
            .await
            .context("exchange refresh token for AVD host PoP token")?;

        if let Some(new_rt) = resp.refresh_token.as_deref().filter(|s| !s.is_empty()) {
            let user = extract_username_from_tokens(&resp)
                .or_else(|| request.username_hint.clone())
                .unwrap_or(matched_user);
            self.store_refresh_token(&authority, &request.client_id, new_rt, &user);
        }

        Ok(resp.access_token)
    }

    fn cache_key(authority: &str, client_id: &str, username_hint: Option<&str>) -> String {
        format!(
            "{}|{}|{}",
            authority.to_ascii_lowercase(),
            client_id.to_ascii_lowercase(),
            username_hint.unwrap_or("").to_ascii_lowercase()
        )
    }

    /// Look up a cached refresh token for `(authority, client_id, username_hint)`.
    ///
    /// Gateway authentication (`allow_cross_tenant_host_exchange == false`) requires an exact `authority`
    /// match (and an exact `username_hint` match when non-empty).
    ///
    /// Host PoP token minting (`allow_cross_tenant_host_exchange == true`) may redeem a multi-resource
    /// refresh token acquired against `/common` (or the user's home tenant) at the orchestration-returned
    /// `/{deviceTenantId}` endpoint on the same validated Entra login host for the same `client_id` and account.
    fn lookup_cached_refresh_token(
        &self,
        authority: &str,
        client_id: &str,
        username_hint: Option<&str>,
        allow_cross_tenant_host_exchange: bool,
    ) -> Option<(Zeroizing<String>, String)> {
        let guard = self.memory_cache.lock().ok()?;
        let hint = username_hint.map(str::trim).filter(|s| !s.is_empty());
        let exact_key = Self::cache_key(authority, client_id, hint);
        if let Some(found) = guard.get(&exact_key) {
            return Some(found.clone());
        }
        let target_host = authority_host(authority)?;
        let client_lower = client_id.to_ascii_lowercase();
        let authority_lower = authority.to_ascii_lowercase();

        if let Some(hint) = hint {
            if !allow_cross_tenant_host_exchange {
                return None;
            }
            let hint_lower = hint.to_ascii_lowercase();
            if let Some((_, v)) = guard.iter().find(|(k, _)| {
                let mut parts = k.splitn(3, '|');
                let (Some(entry_auth), Some(entry_client), Some(entry_user)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    return false;
                };
                entry_client == client_lower
                    && entry_user == hint_lower
                    && authority_host(entry_auth).as_deref() == Some(target_host.as_str())
            }) {
                return Some(v.clone());
            }
            return guard
                .iter()
                .find(|(k, _)| {
                    let mut parts = k.splitn(3, '|');
                    let (Some(entry_auth), Some(entry_client), Some(entry_user)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        return false;
                    };
                    entry_client == client_lower
                        && entry_user.is_empty()
                        && authority_host(entry_auth).as_deref() == Some(target_host.as_str())
                })
                .map(|(_, v)| v.clone());
        }

        let same_authority_prefix = format!("{authority_lower}|{client_lower}|");
        if let Some((_, v)) = guard.iter().find(|(k, _)| k.starts_with(&same_authority_prefix)) {
            return Some(v.clone());
        }
        if allow_cross_tenant_host_exchange {
            return guard
                .iter()
                .find(|(k, _)| {
                    let mut parts = k.splitn(3, '|');
                    let (Some(entry_auth), Some(entry_client), _) = (parts.next(), parts.next(), parts.next()) else {
                        return false;
                    };
                    entry_client == client_lower && authority_host(entry_auth).as_deref() == Some(target_host.as_str())
                })
                .map(|(_, v)| v.clone());
        }
        None
    }

    #[doc(hidden)]
    pub fn store_refresh_token_for_test(&self, authority: &str, client_id: &str, refresh_token: &str, username: &str) {
        self.store_refresh_token(authority, client_id, refresh_token, username);
    }

    #[doc(hidden)]
    pub fn lookup_cached_refresh_token_for_test(
        &self,
        authority: &str,
        client_id: &str,
        username_hint: Option<&str>,
        allow_cross_tenant_host_exchange: bool,
    ) -> Option<(String, String)> {
        self.lookup_cached_refresh_token(authority, client_id, username_hint, allow_cross_tenant_host_exchange)
            .map(|(rt, u)| ((*rt).clone(), u))
    }

    fn store_refresh_token(&self, authority: &str, client_id: &str, refresh_token: &str, username: &str) {
        if let Ok(mut guard) = self.memory_cache.lock() {
            if !username.is_empty() {
                let empty_key = Self::cache_key(authority, client_id, None);
                guard.remove(&empty_key);
            }
            let key = Self::cache_key(authority, client_id, Some(username));
            guard.insert(key, (Zeroizing::new(refresh_token.to_owned()), username.to_owned()));
            if let Some(path) = &self.cache_path {
                save_token_cache_file(path, &guard);
            }
        }
    }

    async fn post_token_request(
        &self,
        authority: &str,
        form_body: &str,
        context_msg: &'static str,
    ) -> anyhow::Result<OAuthTokenResponse> {
        let (status, bytes) = self.send_token_form(authority, form_body, context_msg).await?;
        ensure!(
            status.is_success(),
            "OAuth2 token endpoint returned HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        Self::parse_token_response(&bytes)
    }

    async fn send_token_form(
        &self,
        authority: &str,
        form_body: &str,
        context_msg: &'static str,
    ) -> anyhow::Result<(reqwest::StatusCode, Vec<u8>)> {
        let resp = self
            .http
            .post(format!("{authority}/oauth2/v2.0/token"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form_body.as_bytes().to_vec())
            .send()
            .await
            .context(context_msg)?;
        let status = resp.status();
        let bytes = resp.bytes().await.context("read OAuth2 token response")?;
        Ok((status, bytes.to_vec()))
    }

    fn parse_token_response(bytes: &[u8]) -> anyhow::Result<OAuthTokenResponse> {
        let parsed: OAuthTokenResponse = serde_json::from_slice(bytes).context("decode OAuth2 token response JSON")?;
        ensure!(!parsed.access_token.is_empty(), "OAuth2 returned an empty access_token");
        Ok(parsed)
    }

    async fn exchange_refresh_token(
        &self,
        authority: &str,
        client_id: &str,
        refresh_token: &str,
        scope: &str,
        claims: Option<&str>,
        req_cnf: Option<&str>,
    ) -> anyhow::Result<OAuthTokenResponse> {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("client_id", client_id)
            .append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", refresh_token)
            .append_pair("scope", scope);
        if let Some(claims) = claims {
            form.append_pair("claims", claims);
        }
        if let Some(req_cnf) = req_cnf {
            form.append_pair("req_cnf", req_cnf).append_pair("token_type", "pop");
        }
        let body = Zeroizing::new(form.finish());
        self.post_token_request(authority, &body, "send OAuth2 refresh_token request")
            .await
    }

    async fn acquire_via_loopback_pkce(
        &self,
        authority: &str,
        client_id: &str,
        scope: &str,
        claims: Option<&str>,
        login_hint: Option<&str>,
    ) -> anyhow::Result<OAuthTokenResponse> {
        let v4_listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind loopback OAuth2 redirect listener")?;
        let port = v4_listener.local_addr()?.port();
        let v6_listener = TcpListener::bind(("::1", port)).await.ok();
        let redirect_uri = format!("http://localhost:{port}");

        let state = URL_SAFE_NO_PAD.encode(random::<[u8; 32]>());
        let code_verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(random::<[u8; 32]>()));
        let code_challenge = URL_SAFE_NO_PAD.encode(HashAlgorithm::SHA2_256.digest(code_verifier.as_bytes()));

        let mut auth_url = Url::parse(&format!("{authority}/oauth2/v2.0/authorize"))?;
        {
            let mut q = auth_url.query_pairs_mut();
            q.append_pair("client_id", client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("response_mode", "query")
                .append_pair("scope", scope)
                .append_pair("state", &state)
                .append_pair("code_challenge", &code_challenge)
                .append_pair("code_challenge_method", "S256");
            if let Some(hint) = login_hint.filter(|s| !s.is_empty()) {
                q.append_pair("login_hint", hint);
            }
            if let Some(claims) = claims.filter(|s| !s.is_empty()) {
                q.append_pair("claims", claims);
            }
        }

        open_system_browser(auth_url.as_str()).context("launch system browser for AVD OAuth2 login")?;
        info!(redirect_uri = %redirect_uri, "Waiting for browser OAuth2 callback");

        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(180), async {
            match v6_listener {
                Some(v6) => {
                    tokio::select! {
                        res = v4_listener.accept() => res,
                        res = v6.accept() => res,
                    }
                }
                None => v4_listener.accept().await,
            }
        })
        .await
        .context("timed out waiting for OAuth2 browser redirect")?
        .context("accept OAuth2 loopback callback")?;

        let mut req_buf = Vec::with_capacity(8192);
        let mut header_complete = false;
        tokio::time::timeout(HTTP_TIMEOUT, async {
            let mut chunk = [0u8; 2048];
            while req_buf.len() < 16_384 {
                let n = socket
                    .read(&mut chunk)
                    .await
                    .context("read OAuth2 loopback HTTP request")?;
                if n == 0 {
                    break;
                }
                req_buf.extend_from_slice(&chunk[..n]);
                if req_buf.windows(4).any(|w| w == b"\r\n\r\n") || req_buf.windows(2).any(|w| w == b"\n\n") {
                    header_complete = true;
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("timed out reading OAuth2 loopback HTTP request")??;
        ensure!(
            header_complete || req_buf.contains(&b'\n'),
            "OAuth2 loopback callback request line was incomplete or exceeded size limit"
        );
        let req_str = String::from_utf8_lossy(&req_buf);
        let request_line = req_str.lines().next().unwrap_or("");
        let path_and_query = request_line.split_whitespace().nth(1).unwrap_or("/");
        let callback_url =
            Url::parse(&format!("http://localhost{path_and_query}")).context("parse loopback callback URL")?;

        let mut code = None;
        let mut returned_state = None;
        let mut oauth_err = None;
        for (k, v) in callback_url.query_pairs() {
            match k.as_ref() {
                "code" => code = Some(v.into_owned()),
                "state" => returned_state = Some(v.into_owned()),
                "error" => oauth_err = Some(v.into_owned()),
                _ => {}
            }
        }

        let html = if oauth_err.is_some() || code.is_none() {
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body><h3>Authentication failed. You may close this window.</h3></body></html>"
        } else {
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body><h3>Authentication complete. You may close this tab and return to IronRDP.</h3></body></html>"
        };
        let _ = socket.write_all(html.as_bytes()).await;
        let _ = socket.shutdown().await;

        if let Some(err) = oauth_err {
            bail!("OAuth2 authorization failed: {err}");
        }
        let code = code.context("OAuth2 callback did not include an authorization code")?;
        ensure!(
            returned_state.as_deref() == Some(state.as_str()),
            "OAuth2 callback state mismatch"
        );

        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("client_id", client_id)
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", &code)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", scope)
            .append_pair("code_verifier", &code_verifier);
        if let Some(claims) = claims {
            form.append_pair("claims", claims);
        }
        let body = Zeroizing::new(form.finish());
        self.post_token_request(authority, &body, "exchange OAuth2 authorization code")
            .await
    }

    async fn acquire_via_device_code(
        &self,
        authority: &str,
        client_id: &str,
        scope: &str,
        claims: Option<&str>,
    ) -> anyhow::Result<OAuthTokenResponse> {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("client_id", client_id).append_pair("scope", scope);
        let resp = self
            .http
            .post(format!("{authority}/oauth2/v2.0/devicecode"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form.finish().into_bytes())
            .send()
            .await
            .context("send OAuth2 devicecode request")?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        ensure!(
            status.is_success(),
            "OAuth2 devicecode endpoint returned HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        let dc: DeviceCodeResponse = serde_json::from_slice(&bytes).context("decode OAuth2 devicecode response")?;

        info!(
            verification_uri = %dc.verification_uri,
            user_code = %dc.user_code,
            "{}",
            dc.message
        );
        #[expect(
            clippy::print_stderr,
            reason = "device-code prompt must be visible to interactive CLI users"
        )]
        {
            eprintln!("{}", dc.message);
        }

        let mut poll_interval = Duration::from_secs(dc.interval.max(1));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in.max(30));

        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(poll_interval).await;

            let mut poll_form = url::form_urlencoded::Serializer::new(String::new());
            poll_form
                .append_pair("client_id", client_id)
                .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:device_code")
                .append_pair("device_code", &dc.device_code);
            if let Some(claims) = claims {
                poll_form.append_pair("claims", claims);
            }
            let poll_body = poll_form.finish();
            let (poll_status, poll_bytes) = self
                .send_token_form(authority, &poll_body, "poll OAuth2 devicecode token endpoint")
                .await?;
            if poll_status.is_success() {
                return Self::parse_token_response(&poll_bytes);
            }

            if let Ok(err) = serde_json::from_slice::<OAuthErrorResponse>(&poll_bytes) {
                match err.error.as_str() {
                    "authorization_pending" => continue,
                    "slow_down" => {
                        poll_interval += Duration::from_secs(5);
                        continue;
                    }
                    other => bail!(
                        "OAuth2 device-code authentication failed ({other}): {}",
                        err.error_description.unwrap_or_default()
                    ),
                }
            }
            bail!(
                "OAuth2 device-code token polling returned HTTP {poll_status}: {}",
                String::from_utf8_lossy(&poll_bytes)
            );
        }

        bail!("OAuth2 device-code authentication timed out")
    }
}

impl AvdTokenProvider for OAuthAvdTokenProvider {
    fn acquire_gateway_token(&self, challenge: &AvdGatewayChallenge) -> anyhow::Result<(String, String)> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build OAuth2 token runtime")?;
        rt.block_on(self.acquire_gateway_token_async(challenge))
    }

    fn acquire_host_pop_token(&self, request: &AvdHostPopTokenRequest) -> anyhow::Result<String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build OAuth2 token runtime")?;
        rt.block_on(self.acquire_host_pop_token_async(request))
    }
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "default_device_code_interval")]
    interval: u64,
    message: String,
}

fn default_device_code_interval() -> u64 {
    5
}

#[derive(Deserialize)]
struct OAuthErrorResponse {
    error: String,
    error_description: Option<String>,
}

/// STUN or TURN server descriptor parsed from an AVD orchestration `iceServersConfig` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Parsed `WWW-Authenticate: Claims:...` challenge parameters from an AVD orchestration gateway.
///
/// See [RFC 7235] section 4.1 ("WWW-Authenticate").
///
/// [RFC 7235]: https://datatracker.ietf.org/doc/html/rfc7235#section-4.1
#[doc(hidden)]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClaimsChallenge {
    pub authority: Option<String>,
    pub client_id: Option<String>,
    pub resource: Option<String>,
    pub site_id: Option<String>,
}

impl ClaimsChallenge {
    /// Parses a `Claims:Authority=...;Client=...;Resource=...;Site=...` challenge header value.
    ///
    /// See [RFC 7235] section 4.1 ("WWW-Authenticate").
    ///
    /// [RFC 7235]: https://datatracker.ietf.org/doc/html/rfc7235#section-4.1
    #[doc(hidden)]
    pub fn parse(header_val: &str) -> Option<Self> {
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
    config: &mut Config,
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

    config.destination = outcome.avd_config.destination.clone();
    config.use_redirection_server_name = outcome.avd_config.use_redirection_server_name;
    config.redirection.clone_from(&outcome.avd_config.redirection);
    config.connector.credentials = outcome.avd_config.connector.credentials.clone();
    config.entra_auth.clone_from(&outcome.avd_config.entra_auth);

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
    let gw_stream = SyncGatewayStream(Mutex::new(crate::ws::websocket_compat(ws)));

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
    let sso_logon_token = config
        .avd_sso_logon_token
        .as_deref()
        .map(String::as_str)
        .filter(|s| !s.is_empty());
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
            req = req.bearer_auth(token);
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
        _ => (!gw.username.is_empty()).then(|| gw.username.clone()),
    };

    let gw_challenge = AvdGatewayChallenge {
        authority: authority.clone(),
        client_id: client_id.clone(),
        resource,
        site_id,
        username_hint: username_hint.clone(),
    };

    let oauth_mode = AvdOAuthMode::from_property(ps.get::<&str>("ironrdp_avd_auth_method"));
    let mut oauth_provider: Option<OAuthAvdTokenProvider> = None;

    let (gw_token, wam_username) = if let Some(prop_token) = config
        .avd_gateway_token
        .as_deref()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
    {
        (prop_token.to_owned(), username_hint.clone().unwrap_or_default())
    } else if let Some(provider) = config.avd_token_provider.as_ref() {
        let provider = Arc::clone(provider);
        let gw_challenge_clone = gw_challenge.clone();
        tokio::task::spawn_blocking(move || provider.acquire_gateway_token(&gw_challenge_clone))
            .await
            .context("join custom AVD gateway token provider")??
    } else {
        #[cfg(windows)]
        let wam_result = if oauth_mode.is_none() {
            match acquire_gateway_token_via_wam(
                gw_challenge.authority.clone(),
                gw_challenge.client_id.clone(),
                gw_challenge.resource.clone(),
                gw_challenge.site_id.clone(),
                gw_challenge.username_hint.clone(),
            )
            .await
            {
                Ok(res) => Some(res),
                Err(error) => {
                    warn!(%error, "WAM gateway token acquisition unavailable; falling back to cross-platform OAuth2 provider");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(not(windows))]
        let wam_result: Option<(String, String)> = None;

        if let Some(res) = wam_result {
            res
        } else {
            let provider = OAuthAvdTokenProvider::new(oauth_mode.unwrap_or_default())?;
            let res = provider.acquire_gateway_token_async(&gw_challenge).await?;
            oauth_provider = Some(provider);
            res
        }
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
        let pop_token = if let Some(provider) = config.avd_token_provider.as_ref() {
            let provider = Arc::clone(provider);
            tokio::task::spawn_blocking(move || provider.acquire_host_pop_token(&pop_req))
                .await
                .context("join custom AVD host PoP token provider")??
        } else if let Some(provider) = oauth_provider.as_ref() {
            provider.acquire_host_pop_token_async(&pop_req).await?
        } else {
            #[cfg(windows)]
            let wam_pop = match acquire_aadj_pop_token_via_wam(
                pop_req.authority.clone(),
                pop_req.client_id.clone(),
                pop_req.resource_uri.clone(),
                pop_req.req_cnf.clone(),
                pop_req.username_hint.clone(),
            )
            .await
            {
                Ok(tok) => Some(tok),
                Err(error) => {
                    warn!(%error, "WAM host PoP token acquisition unavailable; falling back to cross-platform OAuth2 provider");
                    None
                }
            };
            #[cfg(not(windows))]
            let wam_pop: Option<String> = None;

            if let Some(tok) = wam_pop {
                tok
            } else {
                let provider = OAuthAvdTokenProvider::new(oauth_mode.unwrap_or_default())?;
                provider.acquire_host_pop_token_async(&pop_req).await?
            }
        };
        let expires_in = jwt_remaining_lifetime(&pop_token)?.unwrap_or(Duration::from_secs(3600));
        let entra_session =
            EntraAuthSession::from_pop_token(pop_token, expires_in, pop_key, rds_resource_uri, tenant_for_auth)?;
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

/// Decodes an AVD `redirectedServerCert` field into a DER-encoded X.509 certificate.
///
/// The outer Base64 payload encodes a UTF-16LE Base64 string whose decoded bytes follow the
/// [MS-RDPBCGR] section 2.2.13.1.2.1 ("Target Certificate Container") layout.
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[doc(hidden)]
pub fn decode_redirected_server_cert(outer_b64: &str) -> Option<Vec<u8>> {
    const ELEMENT_HEADER_SIZE: usize = 4 /* Type */ + 4 /* Encoding */ + 4 /* Size */;

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
    while ELEMENT_HEADER_SIZE <= rest.len() {
        let element_type = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let encoding = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
        let element_size = usize::try_from(u32::from_le_bytes([rest[8], rest[9], rest[10], rest[11]])).ok()?;
        rest = &rest[ELEMENT_HEADER_SIZE..];
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

/// Decodes an AVD `redirectedAuthBlob` field into the redirection password cookie bytes.
///
/// See [MS-RDPBCGR] section 2.2.13.1.1 ("Server Redirection Packet (RDP_SERVER_REDIRECTION_PACKET)").
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[doc(hidden)]
pub fn decode_redirected_auth_blob(outer_b64: &str) -> Option<Vec<u8>> {
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

#[doc(hidden)]
pub fn parse_ice_servers_config(val: &serde_json::Value) -> Vec<AvdIceServer> {
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

#[doc(hidden)]
pub fn extract_ip_from_network_metadata(meta: &serde_json::Value) -> Option<core::net::IpAddr> {
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

fn authority_host(authority: &str) -> Option<String> {
    Url::parse(authority).ok()?.host_str().map(str::to_ascii_lowercase)
}

#[doc(hidden)]
pub fn normalize_oauth_authority(authority: &str) -> anyhow::Result<String> {
    normalize_oauth_authority_with_options(authority, false)
}

fn normalize_oauth_authority_with_options(authority: &str, allow_loopback_http: bool) -> anyhow::Result<String> {
    let trimmed = authority.trim().trim_end_matches('/');
    let rewritten = if let Some(rest) = trimmed.strip_prefix("https://login.windows.net/") {
        format!("https://login.microsoftonline.com/{rest}")
    } else if trimmed.eq_ignore_ascii_case("https://login.windows.net") {
        "https://login.microsoftonline.com/common".to_owned()
    } else {
        trimmed.to_owned()
    };

    let url = Url::parse(&rewritten).context("parse AVD OAuth2 authority URL")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("AVD OAuth2 authority URL has no host"))?;
    if allow_loopback_http && url.scheme() == "http" && matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Ok(rewritten);
    }
    ensure!(url.scheme() == "https", "AVD OAuth2 authority URL must use HTTPS");
    ensure!(
        matches!(
            host.to_ascii_lowercase().as_str(),
            "login.microsoftonline.com"
                | "login.windows.net"
                | "login.microsoft.com"
                | "login.partner.microsoftonline.cn"
                | "login.microsoftonline.us"
                | "login.usgovcloudapi.net"
        ),
        "untrusted AVD OAuth2 authority host: {host}"
    );
    Ok(rewritten)
}

fn scope_with_suffix(resource: &str, suffix: &str) -> String {
    let trimmed = resource.trim().trim_end_matches('/');
    if trimmed.ends_with("/.default") || trimmed.ends_with("/user_impersonation") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/{suffix}")
    }
}

fn resource_default_scope(resource: &str) -> String {
    scope_with_suffix(resource, ".default")
}

fn host_impersonation_scope(resource_uri: &str) -> String {
    scope_with_suffix(resource_uri, "user_impersonation")
}

fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| STANDARD.decode(payload_b64))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_remaining_lifetime(token: &str) -> anyhow::Result<Option<Duration>> {
    let Some(json) = decode_jwt_payload(token) else {
        return Ok(None);
    };
    let Some(exp) = json.get("exp").and_then(serde_json::Value::as_u64) else {
        return Ok(None);
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_secs();
    ensure!(now < exp, "AVD host PoP access token has already expired");
    Ok(Some(Duration::from_secs(exp - now)))
}

fn extract_username_from_tokens(resp: &OAuthTokenResponse) -> Option<String> {
    resp.id_token
        .as_deref()
        .and_then(extract_username_from_jwt)
        .or_else(|| extract_username_from_jwt(&resp.access_token))
}

fn extract_username_from_jwt(jwt: &str) -> Option<String> {
    let json = decode_jwt_payload(jwt)?;
    for key in ["preferred_username", "upn", "unique_name", "email"] {
        if let Some(user) = json.get(key).and_then(serde_json::Value::as_str)
            && !user.is_empty()
        {
            return Some(user.to_owned());
        }
    }
    None
}

fn open_system_browser(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .context("spawn rundll32 browser handler")?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .context("spawn macOS open command")?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .context("spawn xdg-open command")?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedTokenCache {
    entries: HashMap<String, PersistedTokenEntry>,
}

#[derive(Serialize, Deserialize)]
struct PersistedTokenEntry {
    refresh_token: String,
    username: String,
}

fn default_token_cache_path() -> Option<PathBuf> {
    let custom = std::env::var_os("IRONRDP_AVD_TOKEN_CACHE")?;
    let path = PathBuf::from(custom);
    (!path.as_os_str().is_empty()).then_some(path)
}

fn load_token_cache_file(path: &std::path::Path) -> HashMap<String, (Zeroizing<String>, String)> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(parsed) = serde_json::from_slice::<PersistedTokenCache>(&bytes) else {
        return HashMap::new();
    };
    parsed
        .entries
        .into_iter()
        .map(|(k, v)| (k, (Zeroizing::new(v.refresh_token), v.username)))
        .collect()
}

fn save_token_cache_file(path: &std::path::Path, map: &HashMap<String, (Zeroizing<String>, String)>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let cache = PersistedTokenCache {
        entries: map
            .iter()
            .map(|(k, (rt, user))| {
                (
                    k.clone(),
                    PersistedTokenEntry {
                        refresh_token: (**rt).clone(),
                        username: user.clone(),
                    },
                )
            })
            .collect(),
    };
    let Ok(bytes) = serde_json::to_vec(&cache) else {
        return;
    };
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            && file.set_permissions(std::fs::Permissions::from_mode(0o600)).is_ok()
        {
            let _ = file.write_all(&bytes);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(path, bytes);
    }
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
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::System::WinRT::IWebAuthenticationCoreManagerInterop;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    use windows::core::{HSTRING, h};

    fn resolve_wam_parent_hwnd() -> HWND {
        // SAFETY: `GetForegroundWindow` and `GetConsoleWindow` are simple Win32 queries with no pointer arguments.
        let fg = unsafe { GetForegroundWindow() };
        if !fg.is_invalid() {
            return fg;
        }
        // SAFETY: `GetConsoleWindow` has no preconditions or lifetime dependencies.
        unsafe { GetConsoleWindow() }
    }

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
            let mut candidates: Vec<WebAccount> = Vec::with_capacity(usize::try_from(count).unwrap_or_default());
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
        let hwnd = resolve_wam_parent_hwnd();
        let async_op = if !hwnd.is_invalid() {
            let interop: IWebAuthenticationCoreManagerInterop =
                windows::core::factory::<WebAuthenticationCoreManager, IWebAuthenticationCoreManagerInterop>()
                    .context("factory::<WebAuthenticationCoreManager, IWebAuthenticationCoreManagerInterop>")?;
            // SAFETY: `hwnd` is a valid window handle and `request` is a live WinRT token request.
            unsafe { interop.RequestTokenForWindowAsync(hwnd, &request) }.context("RequestTokenForWindowAsync")?
        } else {
            WebAuthenticationCoreManager::RequestTokenAsync(&request).context("RequestTokenAsync")?
        };
        let result = async_op.join().context("await interactive WAM token request")?;
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
