#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// No need to be as strict as in production libraries
#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

#[cfg(feature = "sound")]
pub mod audio;
#[cfg(all(feature = "gateway", feature = "entra-auth"))]
pub mod avd;
#[cfg(feature = "camera")]
pub mod camera;
#[cfg(feature = "cloud")]
pub mod cloud;
pub mod config;
pub mod egfx_cache;
#[cfg(feature = "entra-auth")]
pub mod entra;
pub mod h264;
pub mod output_channel;
pub mod rail;
pub mod rdp;
#[cfg(feature = "video")]
pub mod video;

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn __test_apply_server_redirection(
    config: &mut config::Config,
    redirection: &ironrdp_pdu::rdp::server_redirection::ServerRedirectionPdu,
) -> ironrdp_connector::ConnectorResult<String> {
    redirection::apply_server_redirection(config, redirection)
}

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn __test_build_connector(
    config: &config::Config,
    auto_reconnect_cookie: Option<&ironrdp_pdu::rdp::session_info::ServerAutoReconnect>,
) -> ironrdp_connector::ConnectorResult<ironrdp_connector::ClientConnector> {
    rdp::__test_build_connector(config, auto_reconnect_cookie)
}

#[cfg(any(feature = "cloud", feature = "entra-auth"))]
fn http_client_builder() -> std::io::Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder();
    #[cfg(feature = "rustls")]
    let builder = builder.use_preconfigured_tls(ironrdp_tls::rustls_client_config(
        ironrdp_tls::CertificateValidation::Strict,
        "",
        None,
    )?);
    #[cfg(all(not(feature = "rustls"), feature = "native-tls"))]
    let builder = builder.use_native_tls();
    #[cfg(not(any(feature = "rustls", feature = "native-tls")))]
    return Err(std::io::Error::other(
        "either the `rustls` or `native-tls` feature must be enabled for HTTPS requests",
    ));
    #[cfg(any(feature = "rustls", feature = "native-tls"))]
    Ok(builder)
}

/// USB device redirection backend (MS-RDPEUSB), re-exported for device selection.
#[cfg(feature = "usb")]
pub use ironrdp_rdpeusb_native as usb;

#[cfg(all(any(windows, target_os = "linux"), feature = "clipboard"))]
mod clipboard;
mod redirection;

#[cfg(feature = "usb")]
mod urbdrc;

mod ws;
