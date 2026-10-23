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
pub use transport::{Capture, Endpoint, Flow, PacketStream, TlsKeyLog, flatten, read_capture};

struct NonEmptyPlaintexts(Vec<Plaintext>);

impl NonEmptyPlaintexts {
    fn new(first: Plaintext) -> Self {
        Self(vec![first])
    }

    fn push(&mut self, plaintext: Plaintext) {
        self.0.push(plaintext);
    }

    fn as_slice(&self) -> &[Plaintext] {
        &self.0
    }

    fn into_first(mut self) -> Plaintext {
        self.0.remove(0)
    }
}

/// Decrypt the RDP plaintext of a capture.
///
/// This unwraps a supported RD Gateway WebSocket or RPC-over-HTTP tunnel before
/// decrypting its inner TLS session.
pub fn decrypt_rdp(capture: &Capture) -> Result<Plaintext, CaptureError> {
    let mut decrypted = match tls::decrypt_tls_streams(
        &capture.flow.client_stream,
        &capture.flow.server_stream,
        capture.tls_key_log.as_str(),
    ) {
        Ok(plaintext) => Ok(NonEmptyPlaintexts::new(plaintext)),
        Err(error) => Err(error),
    };
    for flow in &capture.gateway_alternates {
        if let Ok(plaintext) =
            tls::decrypt_tls_streams(&flow.client_stream, &flow.server_stream, capture.tls_key_log.as_str())
        {
            match &mut decrypted {
                Ok(plaintexts) => plaintexts.push(plaintext),
                Err(_) => decrypted = Ok(NonEmptyPlaintexts::new(plaintext)),
            }
        }
    }

    let decrypted = decrypted?;
    if let Some(outer) = decrypted
        .as_slice()
        .iter()
        .find(|plaintext| is_gateway_tunnel(plaintext))
    {
        let tunneled = extract_tunneled_rdp(outer)?;
        return decrypt_tunneled_rdp(capture, &tunneled);
    }
    if let Some(tunneled) = extract_rpch_from_flows(decrypted.as_slice())? {
        return decrypt_tunneled_rdp(capture, &tunneled);
    }
    Ok(decrypted.into_first())
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
