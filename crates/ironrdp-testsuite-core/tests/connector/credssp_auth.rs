use std::process::Command;

use ironrdp_connector::Credentials;
use ironrdp_connector::credssp::{CredsspAuth, CredsspSequence, KerberosConfig};
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_core::WriteBuf;
use ironrdp_pdu::nego;
use rstest::rstest;

// oracle: spec RFC 4178 / MS-SPNG mechanism OIDs, DER-encoded without tag and length
const NTLM_OID: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0a];
const KRB5_OID: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x12, 0x01, 0x02, 0x02];
const MS_KRB5_OID: &[u8] = &[0x2a, 0x86, 0x48, 0x82, 0xf7, 0x12, 0x01, 0x02, 0x02];
// oracle: spec MS-NLMP 2.2.1.1, the NTLM message signature
const NTLM_SIGNATURE: &[u8] = b"NTLMSSP\0";

/// Set in the child process that runs [`negotiate_without_a_config_child`], to the expected outcome.
const CHILD_EXPECTATION: &str = "IRONRDP_TEST_NEGOTIATE_WITHOUT_CONFIG";

fn first_token(username: &str, auth: CredsspAuth) -> Vec<u8> {
    let (mut sequence, request) = CredsspSequence::init(
        Credentials::UsernamePassword {
            username: username.to_owned(),
            password: "secret".to_owned(),
        },
        None,
        nego::SecurityProtocol::HYBRID,
        "rdp.example.invalid".to_owned().into(),
        vec![0x42; 32],
        auth,
    )
    .unwrap();
    let state = {
        let mut generator = sequence.process_ts_request(request);
        let GeneratorState::Completed(state) = generator.start() else {
            panic!("the first token was expected without a network request");
        };
        state.unwrap()
    };
    let mut buf = WriteBuf::new();
    sequence.handle_process_result(state, &mut buf).unwrap();
    buf.filled().to_vec()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

fn offers_kerberos(token: &[u8]) -> bool {
    contains(token, KRB5_OID) || contains(token, MS_KRB5_OID)
}

fn explicit_kdc() -> KerberosConfig {
    KerberosConfig::new_with_kdc_url(Some("tcp://127.0.0.1:1".to_owned()), "client".to_owned()).unwrap()
}

// oracle: rustdoc CredsspAuth::Ntlm, NTLM only
#[test]
fn ntlm_sends_a_bare_ntlm_negotiate_message() {
    let token = first_token("alice", CredsspAuth::Ntlm);
    assert!(contains(&token, NTLM_SIGNATURE));
    assert!(!offers_kerberos(&token));
}

// oracle: reference sspi 0.23.1 negotiate/mod.rs `Negotiate::new`, the protocol config is the first mechanism; NTLM stays enabled without a package list
#[test]
fn negotiate_with_a_kerberos_config_starts_with_kerberos_and_still_offers_ntlm() {
    let token = first_token("alice@EXAMPLE.INVALID", CredsspAuth::Negotiate(Some(explicit_kdc())));
    assert!(offers_kerberos(&token));
    assert!(contains(&token, NTLM_OID));
}

// oracle: reference sspi 0.23.1 negotiate/mod.rs `PackageListConfig::parse`, a package list of "kerberos" disables NTLM
#[rstest]
#[case::kdc_from_realm(None)]
#[case::explicit_kdc(Some(explicit_kdc()))]
fn kerberos_never_offers_ntlm(#[case] config: Option<KerberosConfig>) {
    let token = first_token("alice@EXAMPLE.INVALID", CredsspAuth::Kerberos(config));
    assert!(offers_kerberos(&token));
    assert!(!contains(&token, NTLM_OID));
    assert!(!contains(&token, NTLM_SIGNATURE));
}

// oracle: reference sspi 0.23.1 negotiate/mod.rs `negotiate_protocol`, NTLM until `detect_kdc_url` finds a KDC for the realm; kdc.rs `detect_kdc_hosts`, `SSPI_KDC_URL` is consulted first
//
// sspi's discovery reads the process environment, so each case runs in a child process whose environment is pinned:
// no `SSPI_KDC_URL*` variables except the case's own, and a krb5.conf path that does not exist. EXAMPLE.INVALID is
// reserved (RFC 2606), so neither the Windows registry nor DNS knows it.
#[rstest]
#[case::no_kdc_found(None, "ntlm")]
#[case::kdc_found(Some("tcp://127.0.0.1:1"), "kerberos")]
fn negotiate_without_a_config_uses_kerberos_only_when_a_kdc_is_found(
    #[case] kdc_url: Option<&str>,
    #[case] expected: &str,
) {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "connector::credssp_auth::negotiate_without_a_config_child",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD_EXPECTATION, expected)
        .env("KRB5_CONFIG", "/nonexistent/krb5.conf");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SSPI_KDC_URL") {
            child.env_remove(name);
        }
    }
    if let Some(kdc_url) = kdc_url {
        child.env("SSPI_KDC_URL", kdc_url);
    }

    let output = child.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("1 passed"), "the child test did not run:\n{stdout}");
}

#[test]
fn negotiate_without_a_config_child() {
    let Ok(expected) = std::env::var(CHILD_EXPECTATION) else {
        return;
    };
    let token = first_token("alice@EXAMPLE.INVALID", CredsspAuth::Negotiate(None));
    match expected.as_str() {
        "ntlm" => {
            assert!(contains(&token, NTLM_SIGNATURE));
            assert!(!offers_kerberos(&token));
        }
        "kerberos" => {
            assert!(offers_kerberos(&token));
            assert!(contains(&token, NTLM_OID));
        }
        other => panic!("unexpected expectation {other}"),
    }
}
