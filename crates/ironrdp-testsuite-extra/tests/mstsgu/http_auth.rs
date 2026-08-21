use ironrdp_mstsgu::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, split_auth_challenge};

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
        GatewayHttpAuth::from_challenges(r"CONTOSO\alice", "secret", None, &["NTLM"]).expect("ntlm init");
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
        GatewayHttpAuth::from_challenges("alice", "secret", None, &["Basic realm=\"RDG\""]).expect("basic");
    assert!(auth.is_none());
    assert!(matches!(step, AuthStep::TryBasic));
}

#[test]
fn prefer_negotiate_over_ntlm() {
    let (auth, _) =
        GatewayHttpAuth::from_challenges("alice", "secret", None, &["NTLM", "Negotiate", "Basic realm=\"x\""])
            .expect("init");
    assert_eq!(auth.expect("negotiate backend").scheme(), "Negotiate");
}

#[test]
fn combined_www_authenticate_prefers_negotiate() {
    let (auth, step) =
        GatewayHttpAuth::from_challenges("alice", "secret", None, &[r#"Negotiate, NTLM, Basic realm="RDG""#])
            .expect("init");
    assert_eq!(auth.expect("negotiate backend").scheme(), "Negotiate");
    assert!(matches!(step, AuthStep::Continue(_)));
}

#[test]
fn quoted_comma_in_basic_realm_is_one_challenge() {
    let (auth, step) =
        GatewayHttpAuth::from_challenges("alice", "secret", None, &[r#"Basic realm="a, b""#]).expect("basic");
    assert!(auth.is_none());
    assert!(matches!(step, AuthStep::TryBasic));
}
