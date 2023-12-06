#![doc = include_str!("../README.md")]

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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
pub(crate) fn extract_tls_server_public_key(cert: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io;

    use x509_cert::der::Decode as _;

    let cert = x509_cert::Certificate::from_der(cert).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let server_public_key = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "subject public key BIT STRING is not aligned"))?
        .to_owned();

    Ok(server_public_key)
}
