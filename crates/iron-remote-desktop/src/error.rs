//! Error handling types and traits for iron-remote-desktop.
//!
//! # Example: Handling RDCleanPath errors
//!
//! ```no_run
//! # use iron_remote_desktop::*;
//! # fn handle_error(error: impl IronError) {
//! match error.kind() {
//!     IronErrorKind::RDCleanPath => {
//!         if let Some(details) = error.rdcleanpath_details() {
//!             // Check for specific HTTP errors
//!             if details.http_status_code() == Some(403) {
//!                 // Handle forbidden/VNET deleted case
//!             }
//!             // Check for WSA errors
//!             if details.wsa_error_code() == Some(10013) {
//!                 // Handle permission denied
//!             }
//!         }
//!     }
//!     _ => {}
//! }
//! # }
//! ```

use wasm_bindgen::prelude::*;

pub trait IronError {
    fn backtrace(&self) -> String;

    fn kind(&self) -> IronErrorKind;

    fn rdcleanpath_details(&self) -> Option<RDCleanPathDetails>;
}

#[derive(Clone, Copy)]
#[wasm_bindgen]
pub enum IronErrorKind {
    /// Catch-all error kind
    General,
    /// Incorrect password used
    WrongPassword,
    /// Unable to login to machine
    LogonFailure,
    /// Insufficient permission, server denied access
    AccessDenied,
    /// Something wrong happened when sending or receiving the RDCleanPath message
    RDCleanPath,
    /// Couldn't connect to proxy
    ProxyConnect,
    /// Protocol negotiation failed
    NegotiationFailure,
}

/// Detailed error information for RDCleanPath errors.
///
/// When an RDCleanPath error occurs, this structure provides granular details
/// about the underlying cause, including HTTP status codes, Windows Socket errors,
/// and TLS alert codes.
#[derive(Clone, Copy, Debug)]
#[wasm_bindgen]
pub struct RDCleanPathDetails {
    http_status_code: Option<u16>,
    wsa_error_code: Option<u16>,
    tls_alert_code: Option<u8>,
}

// NOTE: multiple impl blocks required because wasm-bindgen doesn't support
// non-exported constructors in #[wasm_bindgen] impl blocks
#[wasm_bindgen]
impl RDCleanPathDetails {
    /// HTTP status code if the error originated from an HTTP response.
    ///
    /// Common values:
    /// - 403: Forbidden (e.g., deleted VNET, insufficient permissions)
    /// - 404: Not Found
    /// - 500: Internal Server Error
    /// - 502: Bad Gateway
    /// - 503: Service Unavailable
    #[wasm_bindgen(getter, js_name = httpStatusCode)]
    pub fn http_status_code(&self) -> Option<u16> {
        self.http_status_code
    }

    /// Windows Socket API (WSA) error code.
    ///
    /// Common values:
    /// - 10013: Permission denied (WSAEACCES) - often indicates deleted/invalid VNET
    /// - 10060: Connection timed out (WSAETIMEDOUT)
    /// - 10061: Connection refused (WSAECONNREFUSED)
    /// - 10051: Network is unreachable (WSAENETUNREACH)
    /// - 10065: No route to host (WSAEHOSTUNREACH)
    #[wasm_bindgen(getter, js_name = wsaErrorCode)]
    pub fn wsa_error_code(&self) -> Option<u16> {
        self.wsa_error_code
    }

    /// TLS alert code if the error occurred during TLS handshake.
    ///
    /// Common values:
    /// - 40: Handshake failure
    /// - 42: Bad certificate
    /// - 45: Certificate expired
    /// - 48: Unknown CA
    /// - 112: Unrecognized name
    #[wasm_bindgen(getter, js_name = tlsAlertCode)]
    pub fn tls_alert_code(&self) -> Option<u8> {
        self.tls_alert_code
    }
}

#[expect(
    clippy::allow_attributes,
    reason = "Unfortunately, expect attribute doesn't work with clippy::multiple_inherent_impl lint"
)]
#[allow(
    clippy::multiple_inherent_impl,
    reason = "We don't want to expose the constructor to JS"
)]
impl RDCleanPathDetails {
    pub fn new(http_status_code: Option<u16>, wsa_error_code: Option<u16>, tls_alert_code: Option<u8>) -> Self {
        Self {
            http_status_code,
            wsa_error_code,
            tls_alert_code,
        }
    }
}
