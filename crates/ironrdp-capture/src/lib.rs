//! Offline decoding support for RDP packet captures.

mod error;
mod gateway;
mod gateway_rpch;
mod tls;
mod transport;

pub use error::CaptureError;
pub use gateway::{extract_tunneled_rdp, is_gateway_tunnel};
pub use gateway_rpch::{extract_rpch_from_flows, extract_rpch_tunneled_rdp};
pub use tls::{Plaintext, decrypt_tls};
pub use transport::{Capture, Endpoint, Flow, PacketStream, TlsKeyLog, read_capture};

/// Decrypt the RDP plaintext of a capture.
///
/// This unwraps a supported RD Gateway WebSocket or RPC-over-HTTP tunnel before
/// decrypting its inner TLS session.
pub fn decrypt_rdp(capture: &Capture) -> Result<Plaintext, CaptureError> {
    let primary = tls::decrypt_tls_streams(
        &capture.flow.client_stream,
        &capture.flow.server_stream,
        capture.tls_key_log.as_str(),
    );
    let mut decrypted = Vec::new();
    if let Ok(plaintext) = &primary {
        decrypted.push(plaintext.clone());
    }
    for flow in &capture.gateway_alternates {
        if let Ok(plaintext) =
            tls::decrypt_tls_streams(&flow.client_stream, &flow.server_stream, capture.tls_key_log.as_str())
        {
            decrypted.push(plaintext);
        }
    }

    if let Some(outer) = decrypted.iter().find(|plaintext| is_gateway_tunnel(plaintext)) {
        let tunneled = extract_tunneled_rdp(outer)?;
        return decrypt_tunneled_rdp(capture, &tunneled);
    }
    if let Some(tunneled) = extract_rpch_from_flows(&decrypted)? {
        return decrypt_tunneled_rdp(capture, &tunneled);
    }
    match primary {
        Ok(plaintext) => Ok(decrypted.into_iter().next().unwrap_or(plaintext)),
        Err(error) => decrypted.into_iter().next().ok_or(error),
    }
}

fn decrypt_tunneled_rdp(capture: &Capture, tunneled: &Plaintext) -> Result<Plaintext, CaptureError> {
    tls::decrypt_tls_streams(&tunneled.client, &tunneled.server, capture.tls_key_log.as_str()).map_err(|error| {
        if matches!(error, CaptureError::MissingTlsSecret) {
            CaptureError::MissingTunneledTlsSecret
        } else {
            error
        }
    })
}
