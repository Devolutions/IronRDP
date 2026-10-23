use core::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironrdp_server::{
    CredentialDecision, CredentialOrigin, CredentialValidationError, CredentialValidator, Credentials,
};

fn fixed_creds() -> Credentials {
    Credentials {
        username: "alice".to_owned(),
        password: "hunter2".to_owned(),
        domain: None,
    }
}

struct AlwaysAccept;
#[async_trait]
impl CredentialValidator for AlwaysAccept {
    async fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        Ok(CredentialDecision::Accept)
    }
}

struct AlwaysReject;
#[async_trait]
impl CredentialValidator for AlwaysReject {
    async fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
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
#[async_trait]
impl CredentialValidator for AlwaysBackendError {
    async fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        Err(CredentialValidationError::new(BackendDown))
    }
}

struct CredSspOnly;
#[async_trait]
impl CredentialValidator for CredSspOnly {
    async fn validate(&self, _: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        Ok(CredentialDecision::Reject)
    }

    async fn validate_received(
        &self,
        _: &Credentials,
        origin: CredentialOrigin,
    ) -> Result<CredentialDecision, CredentialValidationError> {
        Ok(if origin == CredentialOrigin::CredSspDelegated {
            CredentialDecision::Accept
        } else {
            CredentialDecision::Reject
        })
    }
}

#[tokio::test]
async fn validator_accept_returns_accept() {
    let v = AlwaysAccept;
    assert_eq!(v.validate(&fixed_creds()).await.unwrap(), CredentialDecision::Accept);
}

#[tokio::test]
async fn validator_reject_returns_reject() {
    let v = AlwaysReject;
    assert_eq!(v.validate(&fixed_creds()).await.unwrap(), CredentialDecision::Reject);
}

#[tokio::test]
async fn validator_backend_error_propagates_source() {
    let v = AlwaysBackendError;
    let err = v.validate(&fixed_creds()).await.expect_err("expected backend error");
    assert_eq!(err.to_string(), "credential validator backend failure");
    let inner = core::error::Error::source(&err).expect("source must be Some");
    assert_eq!(inner.to_string(), "ldap server unreachable");
}

#[tokio::test]
async fn validator_can_be_held_behind_arc_dyn() {
    // Exercises the Send + Sync + 'static bounds the trait promises through Arc<dyn _>.
    let v: Arc<dyn CredentialValidator> = Arc::new(AlwaysAccept);
    assert_eq!(v.validate(&fixed_creds()).await.unwrap(), CredentialDecision::Accept);
}

#[tokio::test]
async fn validator_can_override_origin_aware_policy() {
    let v = CredSspOnly;
    assert_eq!(
        v.validate_received(&fixed_creds(), CredentialOrigin::ClientInfo)
            .await
            .unwrap(),
        CredentialDecision::Reject
    );
    assert_eq!(
        v.validate_received(&fixed_creds(), CredentialOrigin::CredSspDelegated)
            .await
            .unwrap(),
        CredentialDecision::Accept
    );
}
