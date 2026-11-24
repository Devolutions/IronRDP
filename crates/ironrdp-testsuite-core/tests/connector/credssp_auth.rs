use ironrdp_connector::Credentials;
use ironrdp_connector::credssp::{CredsspAuth, CredsspSequence, KerberosConfig};
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_core::WriteBuf;
use ironrdp_pdu::nego;

// RFC 4178 / MS-SPNG mechanism OIDs, DER-encoded without tag and length.
const NTLM_OID: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0a];
const KRB5_OID: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x12, 0x01, 0x02, 0x02];
const MS_KRB5_OID: &[u8] = &[0x2a, 0x86, 0x48, 0x82, 0xf7, 0x12, 0x01, 0x02, 0x02];
// MS-NLMP 2.2.1.1
const NTLM_SIGNATURE: &[u8] = b"NTLMSSP\0";

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

#[test]
fn ntlm_sends_a_bare_ntlm_negotiate_message() {
    let token = first_token("alice", CredsspAuth::Ntlm);
    assert!(contains(&token, NTLM_SIGNATURE));
    assert!(!offers_kerberos(&token));
}

#[test]
fn negotiate_without_a_findable_kdc_starts_with_ntlm() {
    let token = first_token("alice@EXAMPLE.INVALID", CredsspAuth::Negotiate(None));
    assert!(contains(&token, NTLM_SIGNATURE));
}

#[test]
fn negotiate_with_a_kerberos_config_starts_with_kerberos_and_still_offers_ntlm() {
    let config = KerberosConfig::new_with_kdc_url(Some("tcp://127.0.0.1:1".to_owned()), "client".to_owned()).unwrap();
    let token = first_token("alice@EXAMPLE.INVALID", CredsspAuth::Negotiate(Some(config)));
    assert!(offers_kerberos(&token));
    assert!(contains(&token, NTLM_OID));
}

#[test]
fn kerberos_never_offers_ntlm() {
    for config in [
        None,
        Some(KerberosConfig::new_with_kdc_url(Some("tcp://127.0.0.1:1".to_owned()), "client".to_owned()).unwrap()),
    ] {
        let token = first_token("alice@EXAMPLE.INVALID", CredsspAuth::Kerberos(config));
        assert!(offers_kerberos(&token));
        assert!(!contains(&token, NTLM_OID));
        assert!(!contains(&token, NTLM_SIGNATURE));
    }
}
