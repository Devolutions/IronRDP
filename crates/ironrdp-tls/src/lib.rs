#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[cfg(feature = "rustls")]
#[path = "rustls.rs"]
mod impl_;

#[cfg(feature = "native-tls")]
#[path = "native_tls.rs"]
mod impl_;

#[cfg(feature = "stub")]
#[path = "stub.rs"]
mod impl_;

#[cfg(any(
    not(any(feature = "stub", feature = "native-tls", feature = "rustls")),
    all(feature = "stub", feature = "native-tls"),
    all(feature = "stub", feature = "rustls"),
    all(feature = "rustls", feature = "native-tls"),
))]
compile_error!("a TLS backend must be selected by enabling a single feature out of: `rustls`, `native-tls`, `stub`");

// The whole public API of this crate.
#[cfg(any(feature = "stub", feature = "native-tls", feature = "rustls"))]
pub use impl_::{upgrade, TlsStream};

pub fn extract_tls_server_public_key(cert: &x509_cert::Certificate) -> Option<&[u8]> {
    cert.tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
}
