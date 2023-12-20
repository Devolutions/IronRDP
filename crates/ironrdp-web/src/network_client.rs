use std::pin::Pin;

use futures_util::Future;
use ironrdp::connector::sspi::generator::NetworkRequest;
use ironrdp::connector::sspi::network_client::NetworkProtocol;
use ironrdp::connector::{custom_err, reason_err, ConnectorResult};
use ironrdp_futures::AsyncNetworkClient;

#[derive(Debug)]
pub(crate) struct WasmNetworkClient;

impl AsyncNetworkClient for WasmNetworkClient {
    fn send<'a>(
        &'a mut self,
        network_request: &'a NetworkRequest,
    ) -> Pin<Box<dyn Future<Output = ConnectorResult<Vec<u8>>> + 'a>> {
        Box::pin(async move {
            debug!(?network_request.protocol, ?network_request.url);

            match &network_request.protocol {
                NetworkProtocol::Http | NetworkProtocol::Https => {
                    let body = js_sys::Uint8Array::from(&network_request.data[..]);

                    let response = gloo_net::http::Request::post(network_request.url.as_str())
                        .header("keep-alive", "true")
                        .body(body)
                        .map_err(|e| custom_err!("failed to send KDC request", e))?
                        .send()
                        .await
                        .map_err(|e| custom_err!("failed to send KDC request", e))?;

                    if !response.ok() {
                        return Err(reason_err!(
                            "KdcProxy",
                            "HTTP status error ({} {})",
                            response.status(),
                            response.status_text(),
                        ));
                    }

                    let body = response
                        .binary()
                        .await
                        .map_err(|e| custom_err!("failed to retrieve HTTP response", e))?;

                    Ok(body)
                }
                unsupported => Err(reason_err!("CredSSP", "unsupported protocol: {unsupported:?}")),
            }
        })
    }
}
