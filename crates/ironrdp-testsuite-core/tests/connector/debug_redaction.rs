use ironrdp_connector::{Credentials, SmartCardIdentity};

// oracle: mutant Credentials::UsernamePassword Debug output exposing the password or omitting the username.
#[test]
fn credentials_debug_redacts_password() {
    let creds = Credentials::UsernamePassword {
        username: "Administrator".to_owned(),
        password: "dummy-password".to_owned(),
    };

    let rendered = format!("{creds:?}");
    assert!(rendered.contains("Administrator"));
    assert!(!rendered.contains("dummy-password"));
}

// oracle: mutant Credentials::SmartCard Debug output exposing the PIN.
#[test]
fn credentials_debug_redacts_pin() {
    let creds = Credentials::SmartCard {
        pin: "1234".to_owned(),
        config: None,
    };

    let rendered = format!("{creds:?}");
    assert!(!rendered.contains("1234"));
}

// oracle: mutant SmartCardIdentity Debug output exposing certificate or private-key bytes or omitting reader, container, or CSP names.
#[test]
fn smart_card_identity_debug_redacts_certificate_and_private_key() {
    let identity = SmartCardIdentity {
        certificate: b"certsecret".to_vec(),
        reader_name: "Reader0".to_owned(),
        container_name: "Container".to_owned(),
        csp_name: "CSP".to_owned(),
        private_key: b"keysecret".to_vec(),
    };

    let rendered = format!("{identity:?}");
    assert!(rendered.contains("Reader0"));
    assert!(rendered.contains("Container"));
    assert!(rendered.contains("CSP"));
    assert!(!rendered.contains("certsecret"));
    assert!(!rendered.contains(&format!("{:?}", b"certsecret")));
    assert!(!rendered.contains("keysecret"));
    assert!(!rendered.contains(&format!("{:?}", b"keysecret")));
}
