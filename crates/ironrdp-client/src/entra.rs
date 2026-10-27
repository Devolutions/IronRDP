//! Microsoft Entra RDS AAD authentication using the public MS-RDPBCGR OAuth flow.
//!
//! This module supplies a browser-mediated authorization-code flow and the RDS AAD proof-of-possession
//! assertion.
//! The embedding application opens the returned URL and passes the resulting code and state back.
//! It does not use WAM or acquire credentials from the Windows logon session.
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

use core::time::Duration;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use picky::hash::HashAlgorithm;
use picky::jose::jwk::Jwk;
use picky::jose::jws::{Jws, JwsAlg};
use picky::key::PrivateKey;
use rand::random;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::{Zeroize as _, Zeroizing};

const AUTHORITY: &str = "https://login.microsoftonline.com/common";
const MAX_AUTHORITY_RESPONSE_SIZE: usize = 64 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Identifies the RDP host for which the RDP access token is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpResource {
    /// Use the Azure AD device ID assigned to the RDP host.
    DeviceId(String),
    /// Use the host name expected by the RDP server.
    HostName(String),
}

impl RdpResource {
    /// Format the `ms-device-service://` resource URI for this RDP target.
    pub fn uri(&self) -> anyhow::Result<String> {
        let (kind, value) = match self {
            Self::DeviceId(value) => ("id", value),
            Self::HostName(value) => ("name", value),
        };
        ensure!(
            !value.is_empty() && !value.chars().any(char::is_control),
            "RDP resource identifier must be non-empty and contain no control characters"
        );

        let mut uri = Url::parse("ms-device-service://termsrv.wvd.microsoft.com/")?;
        uri.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("invalid RDP resource URI"))?
            .pop_if_empty()
            .push(kind)
            .push(value)
            .push("user_impersonation");
        Ok(uri.into())
    }
}

/// A browser authorization request with an ephemeral RSA PoP key and PKCE verifier.
///
/// Keep this value until the embedding application receives the matching authorization code and state.
/// Dropping it abandons the attempt and discards the in-memory key and verifier.
pub struct EntraAuthorization {
    authorization_url: Url,
    client_id: String,
    redirect_uri: Url,
    state: String,
    code_verifier: Zeroizing<String>,
    resource_uri: String,
    http: reqwest::Client,
    key: EntraPopKey,
}

impl fmt::Debug for EntraAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntraAuthorization")
            .field("authorization_url", &"[redacted]")
            .field("state", &"[redacted]")
            .field("resource_uri", &self.resource_uri)
            .finish_non_exhaustive()
    }
}

impl EntraAuthorization {
    /// Start an authorization-code flow for an RDP target.
    ///
    /// Supply an application client ID you are authorized to use and its registered redirect URI.
    /// The authentication host must capture the `form_post` response at that URI.
    /// Registration alone does not establish permission to obtain RDP resource tokens.
    ///
    /// Key generation runs on Tokio's blocking pool.
    pub async fn begin(client_id: &str, redirect_uri: Url, resource: RdpResource) -> anyhow::Result<Self> {
        ensure!(!client_id.is_empty(), "Entra client ID is empty");
        ensure!(
            redirect_uri.fragment().is_none(),
            "Entra redirect URI must not contain a fragment"
        );
        let resource_uri = resource.uri()?;
        let key = tokio::task::spawn_blocking(EntraPopKey::generate)
            .await
            .context("join Entra PoP key generation")??;
        let state = URL_SAFE_NO_PAD.encode(random::<[u8; 32]>());
        let code_verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(random::<[u8; 32]>()));
        let code_challenge = URL_SAFE_NO_PAD.encode(HashAlgorithm::SHA2_256.digest(code_verifier.as_bytes()));

        let mut authorization_url = Url::parse(&format!("{AUTHORITY}/oauth2/v2.0/authorize"))?;
        authorization_url
            .query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("response_mode", "form_post")
            .append_pair("scope", &resource_uri)
            .append_pair("redirect_uri", redirect_uri.as_str())
            .append_pair("state", &state)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256");

        let http = entra_http_client()?;

        Ok(Self {
            authorization_url,
            client_id: client_id.to_owned(),
            redirect_uri,
            state,
            code_verifier,
            resource_uri,
            http,
            key,
        })
    }

    /// Authorization URL for the embedding application to open in a system browser.
    pub fn authorization_url(&self) -> &Url {
        &self.authorization_url
    }

    /// State value that the embedding application must match against the authorization response.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Exchange the matching authorization code for an RDP PoP access token.
    ///
    /// This consumes the request so that its private key and PKCE verifier cannot be reused for
    /// another authorization response.
    pub async fn complete(self, code: &str, returned_state: &str) -> anyhow::Result<EntraAuthSession> {
        ensure!(!code.is_empty(), "Entra authorization code is empty");
        ensure!(self.state == returned_state, "Entra authorization state did not match");

        let req_cnf = self.key.req_cnf()?;
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("client_id", &self.client_id)
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("scope", &self.resource_uri)
            .append_pair("req_cnf", &req_cnf)
            .append_pair("redirect_uri", self.redirect_uri.as_str())
            .append_pair("code_verifier", &self.code_verifier);
        let form_body = Zeroizing::new(form.finish());

        let response = self
            .http
            .post(format!("{AUTHORITY}/oauth2/v2.0/token"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form_body.as_bytes().to_vec())
            .send()
            .await
            .context("send Entra authorization-code token request")?;
        let body = authority_response_body(response).await?;
        let token: TokenResponse = serde_json::from_slice(&body).context("decode Entra token response")?;
        let expires_in = validate_token_response(&token, &self.resource_uri)?;
        let expires_at = tokio::time::Instant::now()
            .checked_add(expires_in)
            .context("Entra token lifetime is out of range")?;

        Ok(EntraAuthSession {
            access_token: Zeroizing::new(token.access_token.clone()),
            expires_at,
            http: self.http,
            key: self.key,
            resource_uri: self.resource_uri,
            authority: AUTHORITY.to_owned(),
        })
    }
}

/// A PoP-bound Entra RDP access token and its in-memory signing key.
///
/// Clone this through `Arc` when sharing it with an RDP client configuration.
/// The access token is zeroized when this value is dropped, but the token is not refreshed
/// automatically; start a new authorization flow after it expires.
pub struct EntraAuthSession {
    access_token: Zeroizing<String>,
    expires_at: tokio::time::Instant,
    http: reqwest::Client,
    key: EntraPopKey,
    resource_uri: String,
    authority: String,
}

impl fmt::Debug for EntraAuthSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntraAuthSession")
            .field("access_token", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .field("resource_uri", &self.resource_uri)
            .field("key", &self.key)
            .finish()
    }
}

impl EntraAuthSession {
    /// Construct an Entra RDS AAD session from a pre-acquired PoP access token, its remaining lifetime, and its signing key.
    pub fn from_pop_token(
        access_token: String,
        expires_in: Duration,
        key: EntraPopKey,
        resource_uri: String,
        tenant_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        ensure!(!access_token.is_empty(), "Entra returned an empty access token");
        ensure!(Duration::ZERO < expires_in, "Entra returned an expired access token");
        let http = entra_http_client()?;
        let authority = match tenant_id.filter(|id| !id.is_empty()) {
            Some(tenant_id) => {
                ensure!(
                    !tenant_id
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '/' | '\\' | '?' | '#')),
                    "Entra tenant ID contains invalid URL characters"
                );
                let mut url = Url::parse("https://login.microsoftonline.com/")?;
                url.path_segments_mut()
                    .map_err(|()| anyhow::anyhow!("invalid Entra authority URL"))?
                    .pop_if_empty()
                    .push(tenant_id);
                url.into()
            }
            None => AUTHORITY.to_owned(),
        };
        let expires_at = tokio::time::Instant::now()
            .checked_add(expires_in)
            .context("Entra token lifetime is out of range")?;
        Ok(Self {
            access_token: Zeroizing::new(access_token),
            expires_at,
            http,
            key,
            resource_uri,
            authority,
        })
    }

    /// Whether the PoP access token has reached its expiration timestamp.
    pub fn is_expired(&self) -> bool {
        self.expires_at <= tokio::time::Instant::now()
    }

    fn ensure_not_expired(&self) -> anyhow::Result<()> {
        ensure!(
            !self.is_expired(),
            "Entra RDP access token has expired; start a new authorization flow"
        );
        Ok(())
    }

    /// Acquire the fresh AAD nonce required for an RDS AAD authentication handshake.
    ///
    /// Call this once per RDS AAD connection handshake before signing the RDP assertion.
    pub async fn acquire_aad_nonce(&self) -> anyhow::Result<String> {
        self.ensure_not_expired()?;
        self.request_aad_nonce().await
    }

    /// Sign an RDP assertion for the server's TLS-protected nonce and a fresh AAD nonce.
    pub async fn assertion(&self, server_nonce: &str, aad_nonce: &str) -> anyhow::Result<EntraAssertion> {
        self.ensure_not_expired()?;
        ensure!(!server_nonce.is_empty(), "RDP server nonce is empty");
        ensure!(!aad_nonce.is_empty(), "AAD nonce is empty");
        self.key
            .sign_assertion(&self.access_token, &self.resource_uri, server_nonce, aad_nonce)
    }

    async fn request_aad_nonce(&self) -> anyhow::Result<String> {
        let response = self
            .http
            .post(format!("{}/oauth2/token", self.authority))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body("grant_type=srv_challenge")
            .send()
            .await
            .context("send Entra server-challenge nonce request")?;
        let body = authority_response_body(response).await?;
        let nonce: NonceResponse = serde_json::from_slice(&body).context("decode Entra nonce response")?;
        ensure!(!nonce.nonce.is_empty(), "Entra returned an empty AAD nonce");
        Ok(nonce.nonce)
    }
}

/// Signed RDP assertion whose token-bearing contents are cleared on drop.
pub struct EntraAssertion(Zeroizing<String>);

impl EntraAssertion {
    /// Compact JWS assertion to submit to the RDS AAD Auth exchange.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EntraAssertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EntraAssertion([redacted])")
    }
}

/// Ephemeral 2048-bit RSA proof-of-possession key for an Entra RDP access token.
pub struct EntraPopKey {
    private_key: PrivateKey,
    public_jwk: Jwk,
    key_id: String,
}

impl fmt::Debug for EntraPopKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntraPopKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl EntraPopKey {
    /// Generate a new 2048-bit RSA proof-of-possession key.
    pub fn generate() -> anyhow::Result<Self> {
        let private_key = PrivateKey::generate_rsa(2048).context("generate Entra RSA PoP key")?;
        let public_key = private_key.to_public_key().context("derive Entra PoP public key")?;
        let public_jwk = Jwk::from_public_key(&public_key).context("encode Entra PoP public JWK")?;
        let public_jwk_json = serde_json::to_value(&public_jwk)?;
        let key_id = rsa_jwk_thumbprint(&public_jwk_json)?;

        Ok(Self {
            private_key,
            public_jwk,
            key_id,
        })
    }

    /// Encode the Base64URL `req_cnf` parameter binding a token request to this key.
    pub fn req_cnf(&self) -> anyhow::Result<String> {
        let req_cnf_json = serde_json::to_vec(&serde_json::json!({ "kid": self.key_id }))?;
        Ok(URL_SAFE_NO_PAD.encode(req_cnf_json))
    }

    fn sign_assertion(
        &self,
        access_token: &str,
        resource_uri: &str,
        server_nonce: &str,
        aad_nonce: &str,
    ) -> anyhow::Result<EntraAssertion> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates the Unix epoch")?
            .as_secs();
        let client_claims = serde_json::to_string(&ClientClaims { aad_nonce })?;
        let mut jwk = serde_json::to_value(&self.public_jwk)?;
        if let Some(obj) = jwk.as_object_mut() {
            obj.insert("alg".to_owned(), serde_json::Value::String("RS256".to_owned()));
        }
        let claims = RdpAssertionClaims {
            ts: timestamp.to_string(),
            at: access_token,
            u: resource_uri,
            nonce: server_nonce,
            cnf: Confirmation { jwk },
            client_claims,
        };
        let payload = Zeroizing::new(serde_json::to_vec(&claims)?);
        let mut jws = Jws::new(JwsAlg::RS256, payload.to_vec());
        jws.header.kid = Some(self.req_cnf()?);
        let encoded = jws.encode(&self.private_key);
        jws.payload.zeroize();
        Ok(EntraAssertion(Zeroizing::new(encoded?)))
    }
}

fn entra_http_client() -> anyhow::Result<reqwest::Client> {
    crate::http_client_builder()
        .context("configure Entra HTTPS client")?
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("create Entra HTTPS client")
}

fn rsa_jwk_thumbprint(public_jwk_json: &serde_json::Value) -> anyhow::Result<String> {
    ensure!(
        public_jwk_json.get("kty").and_then(serde_json::Value::as_str) == Some("RSA"),
        "Entra JWK key type must be RSA"
    );
    let exponent = public_jwk_json
        .get("e")
        .and_then(serde_json::Value::as_str)
        .context("generated Entra JWK has no RSA public exponent")?;
    let modulus = public_jwk_json
        .get("n")
        .and_then(serde_json::Value::as_str)
        .context("generated Entra JWK has no RSA modulus")?;
    let thumbprint_input = format!(r#"{{"e":"{exponent}","kty":"RSA","n":"{modulus}"}}"#);
    Ok(URL_SAFE_NO_PAD.encode(HashAlgorithm::SHA2_256.digest(thumbprint_input.as_bytes())))
}

fn validate_token_response(token: &TokenResponse, expected_resource_uri: &str) -> anyhow::Result<Duration> {
    ensure!(
        token.token_type.eq_ignore_ascii_case("pop"),
        "Entra returned a non-PoP RDP access token"
    );
    ensure!(!token.access_token.is_empty(), "Entra returned an empty access token");
    ensure!(0 < token.expires_in, "Entra returned an expired access token");
    if let Some(scope) = &token.scope {
        ensure!(
            scope
                .split_ascii_whitespace()
                .any(|granted| granted.eq_ignore_ascii_case(expected_resource_uri)),
            "Entra token response scope did not include the requested RDP resource"
        );
    }
    Ok(Duration::from_secs(token.expires_in))
}

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn __test_rsa_jwk_thumbprint(jwk_json: &str) -> anyhow::Result<String> {
    let value: serde_json::Value = serde_json::from_str(jwk_json)?;
    rsa_jwk_thumbprint(&value)
}

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn __test_validate_token_response(response_json: &str, expected_resource_uri: &str) -> anyhow::Result<Duration> {
    let token: TokenResponse = serde_json::from_str(response_json)?;
    validate_token_response(&token, expected_resource_uri)
}

#[derive(Serialize)]
struct ClientClaims<'a> {
    aad_nonce: &'a str,
}

#[derive(Serialize)]
struct Confirmation {
    jwk: serde_json::Value,
}

#[derive(Serialize)]
struct RdpAssertionClaims<'a> {
    ts: String,
    at: &'a str,
    u: &'a str,
    nonce: &'a str,
    cnf: Confirmation,
    client_claims: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
}

impl Drop for TokenResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
    }
}

#[derive(Deserialize)]
struct NonceResponse {
    #[serde(rename = "Nonce")]
    nonce: String,
}

async fn authority_response_body(response: reqwest::Response) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    ensure!(
        response.status().is_success(),
        "Entra authority returned HTTP status {}",
        response.status()
    );
    if let Some(length) = response.content_length() {
        ensure!(
            usize::try_from(length).is_ok_and(|length| length <= MAX_AUTHORITY_RESPONSE_SIZE),
            "Entra authority response exceeds the size limit"
        );
    }

    let mut body = Zeroizing::new(Vec::new());
    let mut response = response;
    while let Some(chunk) = response.chunk().await.context("read Entra authority response")? {
        ensure!(
            body.len().saturating_add(chunk.len()) <= MAX_AUTHORITY_RESPONSE_SIZE,
            "Entra authority response exceeds the size limit"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
