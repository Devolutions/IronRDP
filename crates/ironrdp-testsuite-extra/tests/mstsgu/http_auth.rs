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

#[test]
fn smart_card_debug_redacts_secrets() {
    let debug = format!("{:?}", smart_card_credentials());
    assert!(!debug.contains("alice@contoso.com"));
    assert!(!debug.contains("sensitive-pin"));
    assert!(!debug.contains("sensitive-certificate"));
    assert!(!debug.contains("sensitive-private-key"));
}

#[cfg(not(feature = "smartcard"))]
#[test]
fn smart_card_without_feature_is_an_explicit_error() {
    let smart_card = smart_card_credentials();
    let result = GatewayHttpAuth::from_challenges("alice", "password", Some(&smart_card), None, &["Negotiate"]);
    let Err(error) = result else {
        panic!("smart-card authentication needs its feature");
    };
    let error = error.to_string();

    assert!(error.contains("smart-card"));
    assert!(error.contains("smartcard"));
    assert!(!error.contains("sensitive-pin"));
    assert!(!error.contains("sensitive-certificate"));
    assert!(!error.contains("sensitive-private-key"));
}

#[cfg(feature = "smartcard")]
#[test]
fn smart_card_requires_username() {
    let mut smart_card = smart_card_credentials();
    smart_card.username.clear();
    let result = GatewayHttpAuth::from_challenges("alice", "password", Some(&smart_card), None, &["Negotiate"]);
    let Err(error) = result else {
        panic!("smart-card authentication requires a username");
    };

    assert!(error.to_string().contains("username"));
}

#[cfg(feature = "smartcard")]
#[test]
fn smart_card_requires_negotiate() {
    let smart_card = smart_card_credentials();
    let result = GatewayHttpAuth::from_challenges("alice", "password", Some(&smart_card), None, &["NTLM"]);
    let Err(error) = result else {
        panic!("smart-card authentication requires Negotiate");
    };
    let error = error.to_string();

    assert!(error.contains("Negotiate"));
    assert!(!error.contains("sensitive-pin"));
    assert!(!error.contains("sensitive-certificate"));
    assert!(!error.contains("sensitive-private-key"));
}

#[cfg(feature = "smartcard")]
#[test]
fn smart_card_rejects_malformed_negotiate_token() {
    let smart_card = smart_card_credentials();
    let result = GatewayHttpAuth::from_challenges("alice", "password", Some(&smart_card), None, &["Negotiate !"]);
    let Err(error) = result else {
        panic!("malformed Negotiate token must fail before PKINIT initialization");
    };
    let error = error.to_string();

    assert!(error.contains("decode negotiate challenge"));
    assert!(!error.contains("sensitive-pin"));
    assert!(!error.contains("sensitive-certificate"));
    assert!(!error.contains("sensitive-private-key"));
}

#[cfg(feature = "smartcard")]
#[test]
fn smart_card_recognizes_combined_negotiate_challenge() {
    let smart_card = smart_card_credentials();
    let result = GatewayHttpAuth::from_challenges(
        "alice",
        "password",
        Some(&smart_card),
        None,
        &[r#"NTLM, Negotiate !, Basic realm="RDG""#],
    );
    let Err(error) = result else {
        panic!("malformed Negotiate token must fail before PKINIT initialization");
    };
    let error = error.to_string();

    assert!(error.contains("decode negotiate challenge"));
    assert!(!error.contains("sensitive-pin"));
    assert!(!error.contains("sensitive-certificate"));
    assert!(!error.contains("sensitive-private-key"));
}
