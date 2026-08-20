#![allow(unused_crate_dependencies)]

use ironrdp_mstsgu::http_auth::{AuthStep, GatewayHttpAuth, NtlmHttpAuth, basic_authorization, split_auth_challenge};

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
    assert_eq!(auth.scheme(), "Negotiate");
}
