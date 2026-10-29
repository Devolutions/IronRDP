use core::time::Duration;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ironrdp_client::config::{ClipboardType, ConfigBuilder, Destination, TransportKind};
use ironrdp_client::entra::{
    __test_rsa_jwk_thumbprint, __test_validate_token_response, EntraAuthSession, EntraAuthorization, EntraPopKey,
    RdpResource,
};
use ironrdp_client::output_channel::output_channel;
use ironrdp_client::rdp::{RdpClient, RdpOutputEvent};
use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest, ResponseFlags, SecurityProtocol};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::server_redirection::{ServerRedirectionPdu, TargetCertificateContainer};
use ironrdp_pdu::x224::X224;
use picky::jose::jwk::Jwk;
use picky::jose::jws::{Jws, JwsAlg};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use url::Url;

// oracle: spec MS-RDPBCGR §2.2.18.2.1; spec MS-RDPBCGR §4.11.4; spec RFC 3986 §3.3
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

// oracle: spec RFC 7638 §3.1
#[test]
fn rsa_jwk_thumbprint_matches_rfc7638_vector() {
    let rfc7638_jwk = r#"{
        "kty": "RSA",
        "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
        "e": "AQAB",
        "alg": "RS256",
        "use": "sig",
        "kid": "2011-04-29"
    }"#;
    assert_eq!(
        __test_rsa_jwk_thumbprint(rfc7638_jwk).expect("compute RFC 7638 thumbprint"),
        "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
    );
}

// oracle: spec MS-RDPBCGR §2.2.18.2.1; spec MS-RDPBCGR §4.11.4
#[tokio::test]
async fn signs_rdp_assertion_with_required_claims_and_rsa_key_id() {
    let key = EntraPopKey::generate().expect("generate PoP key");
    let expected_req_cnf = key.req_cnf().expect("encode req_cnf");
    let auth = EntraAuthSession::from_pop_token(
        "pop-access-token".to_owned(),
        Duration::from_secs(60),
        key,
        "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation".to_owned(),
        Some("00000000-0000-0000-0000-000000000000"),
    )
    .expect("construct session from PoP token");
    assert!(!auth.is_expired());
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

    let payload_segment = assertion
        .as_str()
        .split('.')
        .nth(1)
        .expect("compact JWS payload segment");
    let payload_bytes = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload_segment)
        .expect("decode Base64URL payload");
    let unverified_claims: serde_json::Value = serde_json::from_slice(&payload_bytes).expect("decode assertion JSON");
    let jwk: Jwk = serde_json::from_value(unverified_claims["cnf"]["jwk"].clone()).expect("decode cnf.jwk");
    let public_key = jwk.to_public_key().expect("derive public key from cnf.jwk");

    let verified = Jws::decode(assertion.as_str(), &public_key).expect("verify assertion signature");
    assert_eq!(verified.header.alg, JwsAlg::RS256);
    assert_eq!(verified.header.kid.as_deref(), Some(expected_req_cnf.as_str()));

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
    assert_eq!(claims["cnf"]["jwk"]["alg"], "RS256");
    assert_eq!(
        claims["u"],
        "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation"
    );

    assert!(
        EntraAuthSession::from_pop_token(
            String::new(),
            Duration::from_secs(60),
            EntraPopKey::generate().expect("generate PoP key"),
            "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation".to_owned(),
            None,
        )
        .is_err()
    );
    assert!(
        EntraAuthSession::from_pop_token(
            "pop-access-token".to_owned(),
            Duration::ZERO,
            EntraPopKey::generate().expect("generate PoP key"),
            "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation".to_owned(),
            None,
        )
        .is_err()
    );
    assert!(
        EntraAuthSession::from_pop_token(
            "pop-access-token".to_owned(),
            Duration::from_secs(60),
            EntraPopKey::generate().expect("generate PoP key"),
            "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation".to_owned(),
            Some("tenant/../../other"),
        )
        .is_err()
    );
}

// oracle: spec RFC 6749 §3.3; spec RFC 6749 §5.1; spec MS-RDPBCGR §4.11.4
#[test]
fn token_response_accepts_space_delimited_scope_list_and_rejects_mismatched_scope() {
    let resource_uri = "ms-device-service://termsrv.wvd.microsoft.com/name/desktop.example.com/user_impersonation";
    let multi_scope = serde_json::json!({
        "access_token": "pop-access-token",
        "token_type": "PoP",
        "expires_in": 3600,
        "scope": format!("openid profile {resource_uri}"),
    });
    assert_eq!(
        __test_validate_token_response(&multi_scope.to_string(), resource_uri).expect("accept multi-token scope"),
        Duration::from_secs(3600)
    );

    let wrong_scope = serde_json::json!({
        "access_token": "pop-access-token",
        "token_type": "PoP",
        "expires_in": 3600,
        "scope": "openid profile ms-device-service://termsrv.wvd.microsoft.com/name/other.example.com/user_impersonation",
    });
    assert!(__test_validate_token_response(&wrong_scope.to_string(), resource_uri).is_err());

    let expired = serde_json::json!({
        "access_token": "pop-access-token",
        "token_type": "PoP",
        "expires_in": 0,
        "scope": resource_uri,
    });
    assert!(__test_validate_token_response(&expired.to_string(), resource_uri).is_err());
}

// oracle: spec RFC 6749 §4.1.1; spec RFC 7636 §4.3; spec MS-RDPBCGR §4.11.4
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

// oracle: spec RFC 6749 §4.1.1
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

// oracle: spec RFC 6749 §3.1.2
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

// oracle: spec MS-RDPBCGR §5.4.5.4; spec MS-RDPBCGR §5.4.5.4.1
#[test]
fn entra_config_requires_no_password_and_disables_credssp() {
    let key = EntraPopKey::generate().expect("generate PoP key");
    let auth = Arc::new(
        EntraAuthSession::from_pop_token(
            "test-pop-token".to_owned(),
            Duration::from_secs(60),
            key,
            RdpResource::HostName("desktop.example.com".to_owned())
                .uri()
                .expect("valid target resource"),
            None,
        )
        .expect("valid Entra session"),
    );

    let builder = || {
        ConfigBuilder::new()
            .with_destination(Destination::new("desktop.example.com").expect("valid destination"))
            .with_entra_auth(Arc::clone(&auth))
            .with_client_build(1)
            .with_client_dir("C:\\")
            .with_client_name("client")
            .with_platform(MajorPlatformType::WINDOWS)
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

    // oracle: mutant Entra TLS validation policy guard in ConfigBuilder::build
    let config = builder()
        .with_certificate_validation(ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate)
        .build();
    assert!(config.is_err(), "Entra must not accept invalid TLS certificates");

    let config = builder().with_tls(false).build();
    assert!(config.is_err(), "Entra must not permit TLS to be disabled");

    // oracle: mutant Entra certificate-validation override guard in ConfigBuilder::build
    let config = builder()
        .with_certificate_validation_callback(Arc::new(|_, _, _| true))
        .build();
    assert!(
        config.is_err(),
        "Entra must not permit certificate-validation overrides"
    );

    // oracle: mutant Entra RDCleanPath transport guard in ConfigBuilder::build
    let config = builder()
        .with_transport(TransportKind::RDCleanPath {
            url: Url::parse("wss://proxy.example.com/").expect("valid proxy URL"),
        })
        .with_rdcleanpath_token("proxy-token")
        .build();
    assert!(config.is_err(), "Entra requires validated TLS to the RDP server");
}

// oracle: spec MS-RDPBCGR §5.4.5.4.1
#[tokio::test]
async fn entra_redirected_connection_rejects_self_signed_target_certificate() {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("generate self-signed cert");
    let cert_der = certified.cert.der().to_vec();
    let key_der = tokio_rustls::rustls::pki_types::PrivateKeyDer::Pkcs8(
        tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()),
    );
    let server_config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certified.cert.der().clone()], key_der)
        .expect("build TLS server config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let addr = listener.local_addr().expect("listener address");

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept client connection");
        let mut req_buf = [0u8; 256];
        let n = stream.read(&mut req_buf).await.expect("read X.224 request");
        let request = decode::<X224<ConnectionRequest>>(&req_buf[..n])
            .expect("decode X.224 request")
            .0;
        assert_eq!(request.protocol, SecurityProtocol::RDSAAD);

        let confirm = encode_vec(&X224(ConnectionConfirm::Response {
            flags: ResponseFlags::empty(),
            protocol: SecurityProtocol::RDSAAD,
        }))
        .expect("encode X.224 confirm");
        stream.write_all(&confirm).await.expect("write X.224 confirm");

        // The client must reject the self-signed certificate during the TLS handshake even when
        // the Server Redirection PDU reported that certificate.
        if let Ok(mut tls_stream) = acceptor.accept(stream).await {
            let _ = tls_stream.write_all(b"{\"ts_nonce\":\"nonce\"}\0").await;
            let mut buf = [0u8; 64];
            let read = tls_stream.read(&mut buf).await.unwrap_or(0);
            assert_eq!(read, 0, "the client must not send an assertion over untrusted TLS");
        }
    });

    let key = EntraPopKey::generate().expect("generate PoP key");
    let auth = Arc::new(
        EntraAuthSession::from_pop_token(
            "test-pop-token".to_owned(),
            Duration::from_secs(60),
            key,
            RdpResource::HostName("localhost".to_owned())
                .uri()
                .expect("valid target resource"),
            None,
        )
        .expect("valid Entra session"),
    );

    let mut config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("127.0.0.1", addr.port()))
        .with_entra_auth(auth)
        .with_client_build(1)
        .with_client_dir("C:\\")
        .with_client_name("client")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_clipboard(ClipboardType::Disable)
        .build()
        .expect("build Entra config");

    ironrdp_client::__test_apply_server_redirection(
        &mut config,
        &ServerRedirectionPdu {
            session_id: 1,
            target_net_address: Some("127.0.0.1".to_owned()),
            target_fqdn: Some("localhost".to_owned()),
            target_certificate: Some(TargetCertificateContainer::from_certificate_der(cert_der)),
            ..ServerRedirectionPdu::default()
        },
    )
    .expect("apply server redirection");

    let (output_tx, mut output_rx) = output_channel(16);
    let client = RdpClient::new(config, output_tx);
    let local = tokio::task::LocalSet::new();
    let failure = local
        .run_until(async move {
            let client_task = tokio::task::spawn_local(client.run());
            let failure = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    match output_rx.recv().await.expect("client output event") {
                        RdpOutputEvent::ConnectionFailure(error) => return format!("{error:?}"),
                        RdpOutputEvent::Connected => {
                            panic!("Entra client connected to an untrusted redirected certificate")
                        }
                        RdpOutputEvent::Terminated(result) => {
                            panic!("session terminated without ConnectionFailure: {result:?}")
                        }
                        _ => {}
                    }
                }
            })
            .await
            .expect("client fails promptly");
            let _ = tokio::time::timeout(Duration::from_secs(5), client_task).await;
            failure
        })
        .await;

    assert!(
        failure.contains("TLS upgrade"),
        "expected TLS upgrade failure, got: {failure}"
    );
    server_task.await.expect("server task completed");
}
