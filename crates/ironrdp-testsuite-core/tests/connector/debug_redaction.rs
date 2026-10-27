use ironrdp_connector::{Credentials, SmartCardIdentity};

// oracle: mutant Credentials::UsernamePassword Debug formatting exposing the password or omitting the username
#[test]
fn credentials_debug_redacts_password() {
    let creds = Credentials::UsernamePassword {
        username: "Administrator".to_owned(),
        password: "dummy-password".to_owned(),
    };

    let rendered = format!("{creds:?}");
    assert!(rendered.contains("Credentials::UsernamePassword"));
    assert!(rendered.contains("Administrator"));
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("dummy-password"));
}

// oracle: mutant Credentials::SmartCard or SmartCardIdentity Debug formatting exposing PIN or key material instead of byte lengths
#[test]
fn credentials_debug_redacts_pin_and_identity_material() {
    let identity = SmartCardIdentity {
        certificate: b"certsecret".to_vec(),
        reader_name: "Reader0".to_owned(),
        container_name: "Container".to_owned(),
        csp_name: "CSP".to_owned(),
        private_key: b"keysecret".to_vec(),
    };

    let creds = Credentials::SmartCard {
        pin: "1234".to_owned(),
        config: Some(identity),
    };

    let rendered = format!("{creds:?}");
    assert!(rendered.contains("Credentials::SmartCard"));
    assert!(rendered.contains("<redacted>"));
    assert!(rendered.contains("certificate_len: 10"));
    assert!(rendered.contains("private_key_len: 9"));
    assert!(rendered.contains("Reader0"));
    assert!(rendered.contains("Container"));
    assert!(rendered.contains("CSP"));
    assert!(!rendered.contains("1234"));
    assert!(!rendered.contains("certsecret"));
    assert!(!rendered.contains(&format!("{:?}", b"certsecret")));
    assert!(!rendered.contains("keysecret"));
    assert!(!rendered.contains(&format!("{:?}", b"keysecret")));
}
