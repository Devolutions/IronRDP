use anyhow::{bail, ensure};
use ironrdp_mstsgu::GwSmartCardCredentials;
use ironrdp_mstsgu::http_auth::GatewayHttpAuth;

#[cfg(feature = "smartcard")]
pub(crate) fn check_authentication_errors() -> anyhow::Result<()> {
    // oracle: regression Devolutions/IronRDP#1743.
    let mut credentials = credentials();
    credentials.username.clear();
    let error = authentication_error(&credentials, &["Negotiate"])?;
    ensure!(
        error.contains("username"),
        "missing smartcard username was not rejected: {error}"
    );

    let credentials = self::credentials();
    let error = authentication_error(&credentials, &["NTLM"])?;
    ensure!(
        error.contains("Negotiate"),
        "smartcard authentication accepted an NTLM challenge: {error}"
    );

    let error = authentication_error(&credentials, &["Negotiate !"])?;
    ensure!(
        error.contains("decode negotiate challenge"),
        "malformed challenge did not fail before certificate processing: {error}"
    );

    let error = authentication_error(&credentials, &[r#"NTLM, Negotiate !, Basic realm="RDG""#])?;
    ensure!(
        error.contains("decode negotiate challenge"),
        "combined challenge did not select Negotiate before certificate processing: {error}"
    );
    Ok(())
}

pub(crate) fn check_disabled_feature() -> anyhow::Result<()> {
    // oracle: regression Devolutions/IronRDP#1743.
    ensure!(
        !cfg!(feature = "smartcard"),
        "smartcard-disabled requires a build without smartcard support"
    );
    let error = authentication_error(&credentials(), &["Negotiate"])?;
    ensure!(
        error.contains("smart-card") && error.contains("smartcard"),
        "missing-feature error was not explicit: {error}"
    );
    Ok(())
}

fn authentication_error(credentials: &GwSmartCardCredentials, challenges: &[&str]) -> anyhow::Result<String> {
    let Err(error) = GatewayHttpAuth::from_challenges("alice", "password", Some(credentials), None, challenges) else {
        bail!("invalid smartcard credentials unexpectedly authenticated");
    };
    let error = error.to_string();
    for secret in ["sensitive-pin", "sensitive-certificate", "sensitive-private-key"] {
        ensure!(
            !error.contains(secret),
            "smartcard authentication error disclosed a secret"
        );
    }
    Ok(error)
}

fn credentials() -> GwSmartCardCredentials {
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
