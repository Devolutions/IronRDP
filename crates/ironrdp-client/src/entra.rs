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
const MAX_AUTHORITY_RESPONSE_SIZE_U64: u64 = 64 * 1024;
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
    fn uri(&self) -> anyhow::Result<String> {
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

        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("create Entra HTTPS client")?;

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

        let req_cnf_json = serde_json::to_vec(&serde_json::json!({ "kid": self.key.key_id }))?;
        let req_cnf = URL_SAFE_NO_PAD.encode(req_cnf_json);
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
        ensure!(
            token.token_type.eq_ignore_ascii_case("pop"),
            "Entra returned a non-PoP RDP access token"
        );
        ensure!(!token.access_token.is_empty(), "Entra returned an empty access token");
        ensure!(token.expires_in > 0, "Entra returned an expired access token");
        if let Some(scope) = &token.scope {
            ensure!(
                scope.eq_ignore_ascii_case(&self.resource_uri),
                "Entra token response scope did not match the requested RDP resource"
            );
        }
        let expires_at = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(token.expires_in))
            .context("Entra token lifetime is out of range")?;

        Ok(EntraAuthSession {
            access_token: Zeroizing::new(token.access_token.clone()),
            expires_at,
            http: self.http,
            key: self.key,
            resource_uri: self.resource_uri,
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
    /// Acquire the fresh AAD nonce required for an RDS AAD authentication handshake.
    ///
    /// Call this before the TLS handshake, as required by the RDS AAD connection sequence.
    pub async fn acquire_aad_nonce(&self) -> anyhow::Result<String> {
        ensure!(
            tokio::time::Instant::now() < self.expires_at,
            "Entra RDP access token has expired; start a new authorization flow"
        );
        self.request_aad_nonce().await
    }

    /// Sign an RDP assertion for the server's TLS-protected nonce and a fresh AAD nonce.
    pub async fn assertion(&self, server_nonce: &str, aad_nonce: &str) -> anyhow::Result<EntraAssertion> {
        ensure!(
            tokio::time::Instant::now() < self.expires_at,
            "Entra RDP access token has expired; start a new authorization flow"
        );
        ensure!(!server_nonce.is_empty(), "RDP server nonce is empty");
        ensure!(!aad_nonce.is_empty(), "AAD nonce is empty");
        self.key
            .sign_assertion(&self.access_token, &self.resource_uri, server_nonce, aad_nonce)
    }

    async fn request_aad_nonce(&self) -> anyhow::Result<String> {
        let response = self
            .http
            .post(format!("{AUTHORITY}/oauth2/token"))
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

struct EntraPopKey {
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
    fn generate() -> anyhow::Result<Self> {
        let private_key = PrivateKey::generate_rsa(2048).context("generate Entra RSA PoP key")?;
        let public_key = private_key.to_public_key().context("derive Entra PoP public key")?;
        let public_jwk = Jwk::from_public_key(&public_key).context("encode Entra PoP public JWK")?;
        let public_jwk_json = serde_json::to_value(&public_jwk)?;
        let exponent = public_jwk_json
            .get("e")
            .and_then(serde_json::Value::as_str)
            .context("generated Entra JWK has no RSA public exponent")?;
        let modulus = public_jwk_json
            .get("n")
            .and_then(serde_json::Value::as_str)
            .context("generated Entra JWK has no RSA modulus")?;
        let thumbprint_input = format!(r#"{{"e":"{exponent}","kty":"RSA","n":"{modulus}"}}"#);
        let key_id = URL_SAFE_NO_PAD.encode(HashAlgorithm::SHA2_256.digest(thumbprint_input.as_bytes()));

        Ok(Self {
            private_key,
            public_jwk,
            key_id,
        })
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
        let claims = RdpAssertionClaims {
            ts: timestamp.to_string(),
            at: access_token,
            u: resource_uri,
            nonce: server_nonce,
            cnf: Confirmation { jwk: &self.public_jwk },
            client_claims,
        };
        let payload = Zeroizing::new(serde_json::to_vec(&claims)?);
        let mut jws = Jws::new(JwsAlg::RS256, payload.to_vec());
        jws.header.kid = Some(self.key_id.clone());
        let encoded = jws.encode(&self.private_key);
        jws.payload.zeroize();
        Ok(EntraAssertion(Zeroizing::new(encoded?)))
    }
}

#[derive(Serialize)]
struct ClientClaims<'a> {
    aad_nonce: &'a str,
}

#[derive(Serialize)]
struct Confirmation<'a> {
    jwk: &'a Jwk,
}

#[derive(Serialize)]
struct RdpAssertionClaims<'a> {
    ts: String,
    at: &'a str,
    u: &'a str,
    nonce: &'a str,
    cnf: Confirmation<'a>,
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
            length <= MAX_AUTHORITY_RESPONSE_SIZE_U64,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn builds_host_and_device_resource_uris_as_single_path_segments() {
        assert_eq!(
            RdpResource::HostName("desktop.example.com".to_owned())
                .uri()
                .expect("valid host resource"),
            "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation"
        );
        assert_eq!(
            RdpResource::DeviceId("device/id".to_owned())
                .uri()
                .expect("device ID is escaped as a path segment"),
            "ms-device-service://termsrv.wvd.microsoft.com/id/device%2Fid/user_impersonation"
        );
        assert!(RdpResource::HostName(String::new()).uri().is_err());
    }

    // oracle: spec MS-RDPBCGR section 2.2.18.2.1.
    #[tokio::test]
    async fn signs_rdp_assertion_with_required_claims_and_rsa_key_id() {
        let key = EntraPopKey::generate().expect("generate PoP key");
        let public_key = key.private_key.to_public_key().expect("derive verification key");
        let auth = EntraAuthSession {
            access_token: Zeroizing::new("pop-access-token".to_owned()),
            expires_at: tokio::time::Instant::now() + Duration::from_secs(60),
            http: reqwest::Client::new(),
            key,
            resource_uri: "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation"
                .to_owned(),
        };
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_secs();
        let assertion = auth
            .assertion("server-nonce", "aad-nonce")
            .await
            .expect("sign assertion");
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_secs();
        let verified = Jws::decode(assertion.as_str(), &public_key).expect("verify assertion signature");
        assert_eq!(verified.header.alg, JwsAlg::RS256);
        assert!(verified.header.kid.is_some(), "the JOSE header must contain kid");

        let claims: serde_json::Value = serde_json::from_slice(&verified.payload).expect("decode assertion claims");
        let timestamp = claims["ts"].as_str().expect("timestamp is a JSON string");
        assert!(timestamp.bytes().all(|byte| byte.is_ascii_digit()));
        let timestamp: u64 = timestamp.parse().expect("timestamp contains whole Unix seconds");
        assert!(before <= timestamp && timestamp <= after);
        let client_claims: serde_json::Value = serde_json::from_str(
            claims["client_claims"]
                .as_str()
                .expect("client_claims is a JSON string"),
        )
        .expect("decode client claims");
        assert_eq!(claims["at"], "pop-access-token");
        assert_eq!(claims["nonce"], "server-nonce");
        assert_eq!(client_claims["aad_nonce"], "aad-nonce");
        assert_eq!(claims["cnf"]["jwk"]["kty"], "RSA");
        assert_eq!(
            claims["u"],
            "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation"
        );
        // oracle: spec RFC 7638 section 3 and MS-RDPBCGR section 4.11.4.
        let jwk = serde_json::to_value(&auth.key.public_jwk).expect("serialize public JWK");
        let thumbprint_input = format!(
            r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#,
            jwk["e"].as_str().expect("JWK exponent"),
            jwk["n"].as_str().expect("JWK modulus")
        );
        assert_eq!(
            auth.key.key_id,
            URL_SAFE_NO_PAD.encode(HashAlgorithm::SHA2_256.digest(thumbprint_input.as_bytes()))
        );
    }

    // oracle: spec RFC 6749 section 4.1.1, RFC 7636 section 4.3, and MS-RDPBCGR section 4.11.4.
    #[tokio::test]
    async fn authorization_request_uses_scope_state_and_pkce() {
        let request = EntraAuthorization::begin(
            "8aa28d12-42ad-4ae8-8ed5-f8a593d77f63",
            Url::parse("https://client.example.com/signin-callback").expect("valid application redirect"),
            RdpResource::HostName("desktop.example.com".to_owned()),
        )
        .await
        .expect("create authorization request");
        let query = request
            .authorization_url()
            .query_pairs()
            .into_owned()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            query.get("client_id").map(String::as_str),
            Some("8aa28d12-42ad-4ae8-8ed5-f8a593d77f63")
        );
        assert_eq!(
            query.get("redirect_uri").map(String::as_str),
            Some("https://client.example.com/signin-callback")
        );
        assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(query.get("response_mode").map(String::as_str), Some("form_post"));
        assert_eq!(query.get("code_challenge_method").map(String::as_str), Some("S256"));
        assert_eq!(
            query.get("scope").map(String::as_str),
            Some("ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation")
        );
        assert_eq!(query.get("state").map(String::as_str), Some(request.state()));
        assert!(query.get("code_challenge").is_some_and(|value| !value.is_empty()));
    }

    // oracle: spec RFC 6749 section 4.1.1 requires the client identifier.
    #[tokio::test]
    async fn authorization_requires_explicit_client_id() {
        let result = EntraAuthorization::begin(
            "",
            Url::parse("https://client.example.com/signin-callback").expect("valid application redirect"),
            RdpResource::HostName("desktop.example.com".to_owned()),
        )
        .await;
        assert!(result.is_err(), "missing client ID must not use a first-party default");
    }

    // oracle: spec RFC 6749 section 3.1.2 forbids fragments in the redirection endpoint URI.
    #[tokio::test]
    async fn authorization_rejects_redirect_fragment() {
        let result = EntraAuthorization::begin(
            "8aa28d12-42ad-4ae8-8ed5-f8a593d77f63",
            Url::parse("https://client.example.com/signin-callback#fragment").expect("valid URL"),
            RdpResource::HostName("desktop.example.com".to_owned()),
        )
        .await;
        assert!(result.is_err(), "OAuth redirect URIs must not contain a fragment");
    }

    #[test]
    fn entra_config_requires_no_password_and_disables_credssp() {
        let key = EntraPopKey::generate().expect("generate PoP key");
        let auth = Arc::new(EntraAuthSession {
            access_token: Zeroizing::new("test-pop-token".to_owned()),
            expires_at: tokio::time::Instant::now() + Duration::from_secs(60),
            http: reqwest::Client::new(),
            key,
            resource_uri: RdpResource::HostName("desktop.example.com".to_owned())
                .uri()
                .expect("valid target resource"),
        });

        let builder = || {
            crate::config::ConfigBuilder::new()
                .with_destination(crate::config::Destination::new("desktop.example.com").expect("valid destination"))
                .with_entra_auth(Arc::clone(&auth))
                .with_client_build(1)
                .with_client_dir("C:\\")
                .with_client_name("client")
                .with_platform(ironrdp_pdu::rdp::capability_sets::MajorPlatformType::WINDOWS)
        };
        let config = builder()
            .build()
            .expect("Entra does not require a reusable username or password");

        assert!(config.connector().enable_tls);
        assert!(!config.connector().enable_credssp);
        assert_eq!(
            config.certificate_validation(),
            ironrdp_tls::CertificateValidation::Strict
        );

        let config = builder()
            .with_certificate_validation(ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate)
            .build();
        assert!(config.is_err(), "Entra must not accept invalid TLS certificates");

        let config = builder().with_tls(false).build();
        assert!(config.is_err(), "Entra must not permit TLS to be disabled");

        let config = builder()
            .with_certificate_validation_callback(Arc::new(|_, _, _| true))
            .build();
        assert!(
            config.is_err(),
            "Entra must not permit certificate-validation overrides"
        );

        let config = builder()
            .with_transport(crate::config::TransportKind::RDCleanPath {
                url: Url::parse("wss://proxy.example.com/").expect("valid proxy URL"),
            })
            .with_rdcleanpath_token("proxy-token")
            .build();
        assert!(config.is_err(), "Entra requires validated TLS to the RDP server");
    }
}
