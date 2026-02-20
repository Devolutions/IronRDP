use ironrdp_connector::{Credentials, SmartCardIdentity};

use ironrdp_core as _;
use ironrdp_error as _;
use ironrdp_pdu as _;
use ironrdp_svc as _;
use picky as _;
use picky_asn1_der as _;
use picky_asn1_x509 as _;
use rand as _;
use sspi as _;
use tracing as _;
use url as _;

#[test]
fn credentials_debug_redacts_password() {
    let creds = Credentials::UsernamePassword {
        username: "Administrator".to_owned(),
        password: "dummy-password".to_owned(),
    };

    let rendered = format!("{creds:?}");
    assert!(rendered.contains("Credentials::UsernamePassword"));
    assert!(rendered.contains("Administrator"));
    assert!(!rendered.contains("dummy-password"));
}

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
    assert!(!rendered.contains("1234"));
    assert!(!rendered.contains("certsecret"));
    assert!(!rendered.contains("keysecret"));
}
