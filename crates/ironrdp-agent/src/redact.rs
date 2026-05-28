//! Credential redaction for any property value surfaced over IPC or logs.
//!
//! [`dump_properties`](crate::SessionEntry::dump_properties) and any other path
//! that ships `PropertySet` values out of the daemon **must** filter values
//! through [`redact_value`] first.
//!
//! The agent is consumed by LLMs: leaking `ClearTextPassword`, gateway
//! credentials, or auth tokens through `DumpProperties`, stdout, or tracing
//! would prompt-inject the model with live secrets.

/// Placeholder substituted for sensitive property values before they leave the
/// daemon.
pub const REDACTED: &str = "***REDACTED***";

/// Returns `true` if a property key holds a credential or token whose value
/// must never be exposed verbatim outside the daemon.
///
/// Matches both known canonical `.rdp` credential keys (e.g. `ClearTextPassword`,
/// `GatewayPassword`) and any key that contains a credential-shaped substring
/// (`password`, `secret`, `token`, `credential`, `cookie`, `apikey`,
/// `passphrase`, `auth`) — case-insensitively, to defend against
/// non-canonical / custom keys an LLM might inject via `set-property`.
pub fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "password",
        "passphrase",
        "secret",
        "token",
        "credential",
        "cookie",
        "apikey",
        "api_key",
        "privatekey",
        "private_key",
    ];
    if NEEDLES.iter().any(|needle| lower.contains(needle)) {
        return true;
    }
    // Catch-all for keys that don't follow the substring pattern but are
    // known to carry credential material.
    matches!(
        lower.as_str(),
        "cleartextpassword" | "gatewayaccesstoken" | "kdcproxyclientcertificate" | "pcb"
    )
}

/// Returns the value to emit for a given key: either the original or the
/// [`REDACTED`] placeholder when [`is_sensitive_key`] holds.
pub fn redact_value<'a>(key: &str, value: &'a str) -> &'a str {
    if is_sensitive_key(key) { REDACTED } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_credential_keys_are_sensitive() {
        for key in [
            "ClearTextPassword",
            "cleartextpassword",
            "Password",
            "password 51",
            "GatewayPassword",
            "ProxyPassword",
            "GatewayAccessToken",
            "KdcProxyClientCertificate",
            "winposstr-secret",
            "my_custom_api_key",
            "AuthToken",
            "OAuthCredential",
            "sessionCookie",
            "private_key_pem",
            "pcb",
        ] {
            assert!(is_sensitive_key(key), "{key} should be sensitive");
        }
    }

    #[test]
    fn non_credential_keys_are_not_sensitive() {
        for key in [
            "full address",
            "username",
            "domain",
            "desktopwidth",
            "desktopheight",
            "gatewayhostname",
            "gatewayusername",
            "agent:state",
            "agent:current_width",
            "compression",
            "audiomode",
        ] {
            assert!(!is_sensitive_key(key), "{key} should not be sensitive");
        }
    }

    #[test]
    fn redact_value_passes_through_safe_keys() {
        assert_eq!(redact_value("username", "alice"), "alice");
        assert_eq!(redact_value("desktopwidth", "1920"), "1920");
    }

    #[test]
    fn redact_value_masks_sensitive_keys() {
        assert_eq!(redact_value("ClearTextPassword", "hunter2"), REDACTED);
        assert_eq!(redact_value("GatewayPassword", "hunter2"), REDACTED);
        assert_eq!(redact_value("GatewayAccessToken", "eyJhbGc..."), REDACTED);
        assert_eq!(redact_value("custom_api_key", "abc"), REDACTED);
    }
}
