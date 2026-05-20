use core::fmt;
use std::sync::Arc;

use ironrdp_server::{CredentialDecision, CredentialValidationError, CredentialValidator, Credentials};

fn fixed_creds() -> Credentials {
    Credentials {
        username: "alice".to_owned(),
        password: "hunter2".to_owned(),
        domain: None,
    }
}

struct AlwaysAccept;
impl CredentialValidator for AlwaysAccept {
    fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        Ok(CredentialDecision::Accept)
    }
}

struct AlwaysReject;
impl CredentialValidator for AlwaysReject {
    fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        Ok(CredentialDecision::Reject)
    }
}

#[derive(Debug)]
struct BackendDown;
impl fmt::Display for BackendDown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ldap server unreachable")
    }
}
impl core::error::Error for BackendDown {}

struct AlwaysBackendError;
impl CredentialValidator for AlwaysBackendError {
    fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        Err(CredentialValidationError::new(BackendDown))
    }
}

#[test]
fn validator_accept_returns_accept() {
    let v = AlwaysAccept;
    assert_eq!(v.validate(&fixed_creds()).unwrap(), CredentialDecision::Accept);
}

#[test]
fn validator_reject_returns_reject() {
    let v = AlwaysReject;
    assert_eq!(v.validate(&fixed_creds()).unwrap(), CredentialDecision::Reject);
}

#[test]
fn validator_backend_error_propagates_source() {
    let v = AlwaysBackendError;
    let err = v.validate(&fixed_creds()).expect_err("expected backend error");
    assert_eq!(err.to_string(), "credential validator backend failure");
    let inner = core::error::Error::source(&err).expect("source must be Some");
    assert_eq!(inner.to_string(), "ldap server unreachable");
}

#[test]
fn validator_can_be_held_behind_arc_dyn() {
    // Exercises the Send + Sync + 'static bounds the trait promises through Arc<dyn _>.
    let v: Arc<dyn CredentialValidator> = Arc::new(AlwaysAccept);
    assert_eq!(v.validate(&fixed_creds()).unwrap(), CredentialDecision::Accept);
}
