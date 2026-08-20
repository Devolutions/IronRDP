//! HTTP authentication helpers for the MS-TSGU WebSocket upgrade.
//!
//! Corporate RD Gateways typically challenge with Negotiate and/or NTLM rather than Basic.
//! Negotiate prefers Kerberos (via KDC / `SSPI_KDC_URL`) and falls back to NTLM inside SPNEGO.
//! Pure NTLM is used when the server only offers the NTLM scheme, and for MS-TSGU extended-auth
//! SSPI NTLM blobs after the handshake.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sspi::ntlm::NtlmConfig;
use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, ClientRequestFlags, CredentialUse, Credentials, CredentialsBuffers,
    DataRepresentation, InitializeSecurityContextResult, Negotiate, NegotiateConfig, Ntlm, SecurityBuffer,
    SecurityStatus, Sspi as _, SspiImpl as _, Username,
};

use crate::{Error, GwErrorExt as _, GwErrorKind};

/// Result of consuming one HTTP auth challenge/response.
#[derive(Debug)]
pub(crate) enum AuthStep {
    /// Send another request with this `Authorization` header value.
    Continue(String),
    /// Authentication finished (caller should inspect the final HTTP status).
    Complete,
    /// Server did not offer NTLM/Negotiate; try HTTP Basic instead.
    TryBasic,
}

// SSPI state objects are large value types; boxing would add an indirection per call.
#[expect(clippy::large_enum_variant, reason = "SSPI state objects are large value types")]
enum AuthBackend {
    Ntlm {
        ntlm: Ntlm,
        credentials_handle: Option<AuthIdentityBuffers>,
    },
    Negotiate {
        negotiate: Negotiate,
        credentials_handle: Option<CredentialsBuffers>,
    },
}

/// Multi-leg HTTP auth state for `Authorization: NTLM …` / `Negotiate …`.
pub(crate) struct GatewayHttpAuth {
    backend: AuthBackend,
    /// Scheme used in the Authorization header (`NTLM` or `Negotiate`).
    scheme: &'static str,
    /// Optional SPN / target name (for example `HTTP/rdg.contoso.com`).
    target_name: Option<String>,
    complete: bool,
}

/// Client-side NTLM state for MS-TSGU `HTTP_EXTENDED_AUTH_PACKET` SSPI NTLM blobs.
pub(crate) struct NtlmHttpAuth {
    ntlm: Ntlm,
    credentials_handle: Option<AuthIdentityBuffers>,
    complete: bool,
}

impl GatewayHttpAuth {
    /// Build auth state from a first-round `WWW-Authenticate` challenge set.
    ///
    /// Prefers Negotiate (Kerberos → NTLM SPNEGO) when advertised, otherwise pure NTLM.
    pub(crate) fn from_challenges(
        username: &str,
        password: &str,
        target_name: Option<String>,
        challenges: &[&str],
    ) -> Result<(Self, AuthStep), Error> {
        let mut saw_negotiate = false;
        let mut saw_ntlm = false;
        let mut saw_basic = false;
        let mut negotiate_token: Option<Vec<u8>> = None;
        let mut ntlm_token: Option<Vec<u8>> = None;

        for value in challenges {
            if let Some(rest) = split_auth_challenge(value, "Negotiate") {
                saw_negotiate = true;
                if rest.is_empty() {
                    if negotiate_token.is_none() {
                        negotiate_token = Some(Vec::new());
                    }
                } else {
                    negotiate_token = Some(
                        STANDARD
                            .decode(rest.as_bytes())
                            .map_err(|e| Error::custom("decode negotiate challenge", e))?,
                    );
                }
            } else if let Some(rest) = split_auth_challenge(value, "NTLM") {
                saw_ntlm = true;
                if rest.is_empty() {
                    if ntlm_token.is_none() {
                        ntlm_token = Some(Vec::new());
                    }
                } else {
                    ntlm_token = Some(
                        STANDARD
                            .decode(rest.as_bytes())
                            .map_err(|e| Error::custom("decode ntlm challenge", e))?,
                    );
                }
            } else if split_auth_challenge(value, "Basic").is_some() {
                saw_basic = true;
            }
        }

        if saw_negotiate {
            let mut auth = Self::new_negotiate(username, password, target_name.clone())?;
            let input = negotiate_token.as_deref().filter(|t| !t.is_empty());
            // Kerberos SPNEGO can fail without a reachable KDC (for example a gateway SPN
            // with no ticket path); fall back to plain NTLM, which needs no network.
            match auth.initialize(input) {
                Ok(token) => {
                    let header = auth.format_authorization(&token);
                    return Ok((auth, AuthStep::Continue(header)));
                }
                Err(error) if saw_ntlm => {
                    log::debug!("Negotiate failed ({error}); falling back to NTLM");
                }
                Err(error) => return Err(error),
            }
        }

        if saw_ntlm {
            let mut auth = Self::new_ntlm(username, password, target_name)?;
            let input = ntlm_token.as_deref().filter(|t| !t.is_empty());
            let token = auth.initialize(input)?;
            let header = auth.format_authorization(&token);
            return Ok((auth, AuthStep::Continue(header)));
        }

        if saw_basic {
            // Dummy backend is not used when falling back to Basic.
            let auth = Self::new_ntlm(username, password, target_name)?;
            return Ok((auth, AuthStep::TryBasic));
        }

        Err(Error::new(
            "websocket upgrade auth challenge",
            GwErrorKind::UnsupportedFeature,
        ))
    }

    fn new_ntlm(username: &str, password: &str, target_name: Option<String>) -> Result<Self, Error> {
        let identity = auth_identity(username, password)?;
        let mut ntlm = Ntlm::new();
        let credentials_handle = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .map_err(|e| Error::custom("acquire ntlm credentials", e))?
            .credentials_handle;

        Ok(Self {
            backend: AuthBackend::Ntlm {
                ntlm,
                credentials_handle,
            },
            scheme: "NTLM",
            target_name,
            complete: false,
        })
    }

    fn new_negotiate(username: &str, password: &str, target_name: Option<String>) -> Result<Self, Error> {
        let identity = auth_identity(username, password)?;
        let credentials = Credentials::AuthIdentity(identity);
        let client_computer_name = client_computer_name();

        // Start as NTLM; Negotiate upgrades to Kerberos when a KDC is discoverable.
        let config = NegotiateConfig {
            protocol_config: Box::new(NtlmConfig::new(client_computer_name.clone())),
            // Prefer Kerberos when available; keep NTLM fallback; skip PKU2U for gateway HTTP.
            package_list: Some("kerberos,ntlm".to_owned()),
            client_computer_name,
        };

        let mut negotiate = Negotiate::new_client(config).map_err(|e| Error::custom("create negotiate package", e))?;
        let credentials_handle = negotiate
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&credentials)
            .execute(&mut negotiate)
            .map_err(|e| Error::custom("acquire negotiate credentials", e))?
            .credentials_handle;

        Ok(Self {
            backend: AuthBackend::Negotiate {
                negotiate,
                credentials_handle,
            },
            scheme: "Negotiate",
            target_name,
            complete: false,
        })
    }

    /// Consume a subsequent `WWW-Authenticate` challenge and produce the next step.
    pub(crate) fn step_www_authenticate<'a, I>(&mut self, challenges: I) -> Result<AuthStep, Error>
    where
        I: IntoIterator<Item = &'a str>,
    {
        if self.complete {
            return Ok(AuthStep::Complete);
        }

        let mut saw_basic = false;
        let mut token: Option<Vec<u8>> = None;

        for value in challenges {
            let preferred = match self.scheme {
                "Negotiate" => split_auth_challenge(value, "Negotiate"),
                _ => split_auth_challenge(value, "NTLM").or_else(|| split_auth_challenge(value, "Negotiate")),
            };

            if let Some(rest) = preferred {
                if rest.is_empty() {
                    if token.is_none() {
                        token = Some(Vec::new());
                    }
                } else {
                    token = Some(
                        STANDARD
                            .decode(rest.as_bytes())
                            .map_err(|e| Error::custom("decode auth challenge", e))?,
                    );
                }
            } else if split_auth_challenge(value, "Basic").is_some() {
                saw_basic = true;
            }
        }

        match token {
            Some(raw) => {
                let input = if raw.is_empty() { None } else { Some(raw.as_slice()) };
                let next = self.initialize(input)?;
                if self.complete && next.is_empty() {
                    Ok(AuthStep::Complete)
                } else {
                    Ok(AuthStep::Continue(self.format_authorization(&next)))
                }
            }
            None if saw_basic => Ok(AuthStep::TryBasic),
            None => Err(Error::new(
                "websocket upgrade auth challenge",
                GwErrorKind::UnsupportedFeature,
            )),
        }
    }

    fn format_authorization(&self, token: &[u8]) -> String {
        format!("{} {}", self.scheme, STANDARD.encode(token))
    }

    fn initialize(&mut self, input: Option<&[u8]>) -> Result<Vec<u8>, Error> {
        let mut input_token = [SecurityBuffer::new(
            input.map(<[u8]>::to_vec).unwrap_or_default(),
            BufferType::Token,
        )];
        let mut output_token = [SecurityBuffer::new(Vec::with_capacity(1024), BufferType::Token)];

        let status = match &mut self.backend {
            AuthBackend::Ntlm {
                ntlm,
                credentials_handle,
            } => {
                let mut builder = ntlm
                    .initialize_security_context()
                    .with_credentials_handle(credentials_handle)
                    .with_context_requirements(default_context_flags())
                    .with_target_data_representation(DataRepresentation::Native)
                    .with_input(&mut input_token)
                    .with_output(&mut output_token);
                builder.target_name = self.target_name.as_deref();

                let InitializeSecurityContextResult { status, .. } = ntlm
                    .initialize_security_context_impl(&mut builder)
                    .map_err(|e| Error::custom("ntlm initialize security context", e))?
                    .resolve_to_result()
                    .map_err(|e| Error::custom("ntlm initialize security context", e))?;
                status
            }
            AuthBackend::Negotiate {
                negotiate,
                credentials_handle,
            } => {
                let mut builder = negotiate
                    .initialize_security_context()
                    .with_credentials_handle(credentials_handle)
                    .with_context_requirements(default_context_flags())
                    .with_target_data_representation(DataRepresentation::Native)
                    .with_input(&mut input_token)
                    .with_output(&mut output_token);
                builder.target_name = self.target_name.as_deref();

                let mut generator = negotiate
                    .initialize_security_context_impl(&mut builder)
                    .map_err(|e| Error::custom("negotiate initialize security context", e))?;

                // Kerberos may suspend for KDC traffic; NTLM completes without network.
                let InitializeSecurityContextResult { status, .. } = generator
                    .resolve_with_default_network_client()
                    .map_err(|e| Error::custom("negotiate initialize security context", e))?;
                status
            }
        };

        match status {
            SecurityStatus::Ok => {
                self.complete = true;
            }
            SecurityStatus::ContinueNeeded => {}
            other => {
                return Err(Error::new("http auth security status", GwErrorKind::Connect)
                    .with_source(std::io::Error::other(format!("unexpected auth status: {other:?}"))));
            }
        }

        Ok(core::mem::take(&mut output_token[0].buffer))
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "kept for post-handshake SSPI NTLM extended auth")
)]
impl NtlmHttpAuth {
    pub(crate) fn new(username: &str, password: &str) -> Result<Self, Error> {
        let identity = auth_identity(username, password)?;
        let mut ntlm = Ntlm::new();
        let credentials_handle = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .map_err(|e| Error::custom("acquire ntlm credentials", e))?
            .credentials_handle;

        Ok(Self {
            ntlm,
            credentials_handle,
            complete: false,
        })
    }

    /// Produce the next NTLM token for extended-auth packet exchange.
    ///
    /// Returns `(token, complete)`.
    pub(crate) fn step_token(&mut self, input: Option<&[u8]>) -> Result<(Vec<u8>, bool), Error> {
        let mut input_token = [SecurityBuffer::new(
            input.map(<[u8]>::to_vec).unwrap_or_default(),
            BufferType::Token,
        )];
        let mut output_token = [SecurityBuffer::new(Vec::with_capacity(1024), BufferType::Token)];

        let mut builder = self
            .ntlm
            .initialize_security_context()
            .with_credentials_handle(&mut self.credentials_handle)
            .with_context_requirements(default_context_flags())
            .with_target_data_representation(DataRepresentation::Native)
            .with_input(&mut input_token)
            .with_output(&mut output_token);

        let InitializeSecurityContextResult { status, .. } = self
            .ntlm
            .initialize_security_context_impl(&mut builder)
            .map_err(|e| Error::custom("ntlm initialize security context", e))?
            .resolve_to_result()
            .map_err(|e| Error::custom("ntlm initialize security context", e))?;

        match status {
            SecurityStatus::Ok => {
                self.complete = true;
            }
            SecurityStatus::ContinueNeeded => {}
            other => {
                return Err(Error::new("ntlm security status", GwErrorKind::Connect)
                    .with_source(std::io::Error::other(format!("unexpected ntlm status: {other:?}"))));
            }
        }

        Ok((core::mem::take(&mut output_token[0].buffer), self.complete))
    }
}

/// Build a Basic authorization header value.
pub(crate) fn basic_authorization(username: &str, password: &str) -> String {
    let token = STANDARD.encode(format!("{username}:{password}"));
    format!("Basic {token}")
}

fn auth_identity(username: &str, password: &str) -> Result<AuthIdentity, Error> {
    let username = Username::parse(username)
        .or_else(|_| Username::new(username, None))
        .map_err(|e| Error::custom("parse gateway username", e))?;

    Ok(AuthIdentity {
        username,
        password: password.to_owned().into(),
    })
}

fn client_computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "IRONRDP".to_owned())
}

fn default_context_flags() -> ClientRequestFlags {
    ClientRequestFlags::ALLOCATE_MEMORY
        | ClientRequestFlags::CONFIDENTIALITY
        | ClientRequestFlags::INTEGRITY
        | ClientRequestFlags::MUTUAL_AUTH
}

/// Case-insensitive scheme match; returns the remainder after the scheme (may be empty).
fn split_auth_challenge<'a>(header_value: &'a str, scheme: &str) -> Option<&'a str> {
    let header_value = header_value.trim();
    let scheme_len = scheme.len();
    if header_value.len() < scheme_len {
        return None;
    }
    if !header_value.as_bytes()[..scheme_len].eq_ignore_ascii_case(scheme.as_bytes()) {
        return None;
    }
    // The scheme is ASCII, so this byte offset is always a char boundary.
    let rest = header_value.get(scheme_len..)?;
    if rest.is_empty() {
        return Some("");
    }
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(rest.trim())
}

/// Collect all `WWW-Authenticate` header values as strings.
pub(crate) fn www_authenticate_values(headers: &hyper::HeaderMap) -> Vec<&str> {
    headers
        .get_all(hyper::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_auth_challenge_parses_schemes() {
        assert_eq!(split_auth_challenge("NTLM TlRMTVNTUA==", "NTLM"), Some("TlRMTVNTUA=="));
        assert_eq!(split_auth_challenge("ntlm", "NTLM"), Some(""));
        assert_eq!(split_auth_challenge("Negotiate abc", "Negotiate"), Some("abc"));
        assert_eq!(
            split_auth_challenge("Basic realm=\"rdg\"", "Basic"),
            Some("realm=\"rdg\"")
        );
        assert_eq!(split_auth_challenge("Digest qop=auth", "NTLM"), None);
    }

    #[test]
    fn basic_authorization_format() {
        let value = basic_authorization("user", "pass");
        assert_eq!(value, "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn negotiate_type1_from_challenge() {
        let (auth, step) = GatewayHttpAuth::from_challenges(
            r"CONTOSO\alice",
            "secret",
            Some("HTTP/rdg.contoso.com".to_owned()),
            &["Negotiate"],
        )
        .expect("negotiate init");
        assert_eq!(auth.scheme, "Negotiate");
        match step {
            AuthStep::Continue(header) => {
                assert!(header.starts_with("Negotiate "));
                assert!(header.len() > "Negotiate ".len());
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[test]
    fn ntlm_type1_from_challenge() {
        let (auth, step) =
            GatewayHttpAuth::from_challenges(r"CONTOSO\alice", "secret", None, &["NTLM"]).expect("ntlm init");
        assert_eq!(auth.scheme, "NTLM");
        match step {
            AuthStep::Continue(header) => {
                assert!(header.starts_with("NTLM "));
                assert!(header.len() > "NTLM ".len());
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[test]
    fn try_basic_when_only_basic_offered() {
        let (_auth, step) =
            GatewayHttpAuth::from_challenges("alice", "secret", None, &["Basic realm=\"RDG\""]).expect("basic");
        assert!(matches!(step, AuthStep::TryBasic));
    }

    #[test]
    fn extended_auth_ntlm_type1_non_empty() {
        let mut auth = NtlmHttpAuth::new(r"CONTOSO\alice", "secret").expect("ntlm init");
        let (token, complete) = auth.step_token(None).expect("type1");
        assert!(!token.is_empty());
        assert!(!complete);
    }

    #[test]
    fn prefer_negotiate_over_ntlm() {
        let (auth, _) =
            GatewayHttpAuth::from_challenges("alice", "secret", None, &["NTLM", "Negotiate", "Basic realm=\"x\""])
                .expect("init");
        assert_eq!(auth.scheme, "Negotiate");
    }
}
