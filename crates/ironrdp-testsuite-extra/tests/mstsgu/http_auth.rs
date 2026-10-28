use ironrdp_mstsgu::GwSmartCardCredentials;
use ironrdp_mstsgu::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, split_auth_challenge};

fn smart_card_credentials() -> GwSmartCardCredentials {
    GwSmartCardCredentials {
        username: "alice@contoso.com".to_owned(),
        pin: "sensitive-pin".to_owned(),
        certificate: b"sensitive-certificate".to_vec(),
        private_key: Some(b"sensitive-private-key".to_vec()),
        reader_name: "Reader 0".to_owned(),
        card_name: Some("Card".to_owned()),
        container_name: Some("Container".to_owned()),
        csp_name: Some("Provider".to_owned()),
    }
}

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
        None,
        Some("HTTP/rdg.contoso.com".to_owned()),
        &["Negotiate"],
    )
    .expect("negotiate init");
    let auth = auth.expect("negotiate backend");
    assert_eq!(auth.scheme(), "Negotiate");
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
        GatewayHttpAuth::from_challenges(r"CONTOSO\alice", "secret", None, None, &["NTLM"]).expect("ntlm init");
    let auth = auth.expect("ntlm backend");
    assert_eq!(auth.scheme(), "NTLM");
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
    let (auth, step) =
        GatewayHttpAuth::from_challenges("alice", "secret", None, None, &["Basic realm=\"RDG\""]).expect("basic");
    assert!(auth.is_none());
    assert!(matches!(step, AuthStep::TryBasic));
}

#[test]
fn prefer_negotiate_over_ntlm() {
    let (auth, _) = GatewayHttpAuth::from_challenges(
        "alice",
        "secret",
        None,
        None,
        &["NTLM", "Negotiate", "Basic realm=\"x\""],
    )
    .expect("init");
    assert_eq!(auth.expect("negotiate backend").scheme(), "Negotiate");
}

#[test]
fn combined_www_authenticate_prefers_negotiate() {
    let (auth, step) = GatewayHttpAuth::from_challenges(
        "alice",
        "secret",
        None,
        None,
        &[r#"Negotiate, NTLM, Basic realm="RDG""#],
    )
    .expect("init");
    assert_eq!(auth.expect("negotiate backend").scheme(), "Negotiate");
    assert!(matches!(step, AuthStep::Continue(_)));
}

#[test]
fn quoted_comma_in_basic_realm_is_one_challenge() {
    let (auth, step) =
        GatewayHttpAuth::from_challenges("alice", "secret", None, None, &[r#"Basic realm="a, b""#]).expect("basic");
    assert!(auth.is_none());
    assert!(matches!(step, AuthStep::TryBasic));
}

// oracle: regression Devolutions/IronRDP#1743.
#[test]
fn smart_card_debug_redacts_secrets() {
    let debug = format!("{:?}", smart_card_credentials());
    assert!(!debug.contains("alice@contoso.com"));
    assert!(!debug.contains("sensitive-pin"));
    assert!(!debug.contains("sensitive-certificate"));
    assert!(!debug.contains("sensitive-private-key"));
}

// oracle: regression Devolutions/IronRDP#1743.
#[tokio::test]
async fn smart_card_rejects_invalid_credentials_and_challenges() {
    super::process::run("smartcard-auth", None, None)
        .await
        .expect("isolated smartcard authentication checks");
}

// oracle: regression Devolutions/IronRDP#1743.
#[tokio::test]
async fn smart_card_without_feature_is_an_explicit_error() {
    super::process::run("smartcard-disabled", None, None)
        .await
        .expect("isolated missing-smartcard-feature check");
}
