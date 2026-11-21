//! Microsoft Entra ID sign-in (RDS AAD Auth) for the web client.
//!
//! The connector negotiates `PROTOCOL_RDSAAD` and frames the exchange, and the embedding
//! application signs the RDP assertion, because it holds the access token and the
//! proof-of-possession key.
//! See [MS-RDPBCGR] section "5.4.5.4.1 RDS AAD Auth Connection Sequence"[^sequence].
//!
//! [^sequence]: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/8f62058b-c7e5-4244-8f14-ed7d76618cb5>

use anyhow::Context as _;
use ironrdp::connector::ClientConnector;
use ironrdp_core::WriteBuf;
use ironrdp_futures::{Framed, FramedRead, FramedWrite, single_sequence_step};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::error::IronError;

/// Runs the RDS AAD Auth exchange on a stream the gateway has already upgraded to TLS.
///
/// `sign_assertion` receives the server nonce and returns the compact JWS RDP assertion, or a
/// promise of it.
pub(crate) async fn perform_entra_auth<S>(
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    sign_assertion: &js_sys::Function,
) -> Result<(), IronError>
where
    S: FramedRead + FramedWrite,
{
    let mut output = WriteBuf::new();

    while connector.should_perform_entra_auth() {
        if let Some(server_nonce) = connector.entra_server_nonce().map(str::to_owned) {
            let assertion = request_assertion(sign_assertion, &server_nonce).await?;

            let written = connector.submit_entra_assertion(&assertion, &mut output)?;
            if written.size().is_some() {
                let write_result = framed.write_all(output.filled()).await;
                output.clear();
                write_result.context("write RDS AAD authentication request")?;
            }
        } else {
            single_sequence_step(framed, connector, &mut output).await?;
            output.clear();
        }
    }

    Ok(())
}

/// Asks the application to sign the RDP assertion for `server_nonce`.
async fn request_assertion(sign_assertion: &js_sys::Function, server_nonce: &str) -> Result<String, IronError> {
    let returned = sign_assertion
        .call1(&JsValue::NULL, &JsValue::from_str(server_nonce))
        .map_err(|e| anyhow::Error::msg(format!("Entra assertion callback failed: {e:?}")))?;

    // The callback signs with the Web Crypto API, which is asynchronous, so a promise is the
    // usual return value; `Promise.resolve` passes a plain value through.
    let resolved = JsFuture::from(js_sys::Promise::resolve(&returned))
        .await
        .map_err(|e| anyhow::Error::msg(format!("Entra assertion callback failed: {e:?}")))?;

    resolved
        .as_string()
        .context("Entra assertion callback did not return a string")
        .map_err(IronError::from)
}
