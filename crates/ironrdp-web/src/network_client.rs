use std::pin::Pin;

use base64::Engine;
use futures_util::Future;
use ironrdp::connector::sspi::generator::NetworkRequest;
use ironrdp::connector::sspi::network_client::NetworkProtocol;
use ironrdp::connector::{general_err, reason_err, ConnectorResult};
use ironrdp_futures::AsyncNetworkClient;

#[derive(Debug)]
pub(crate) struct WasmNetworkClient {
    kdc_url: String,
    // client: reqwest::Client,
}
impl AsyncNetworkClient for WasmNetworkClient {
    fn send<'a>(
        &'a mut self,
        network_request: &'a NetworkRequest,
    ) -> Pin<Box<dyn Future<Output = ConnectorResult<Vec<u8>>> + 'a>> {
        Box::pin(async move {
            info!("network requwest = {:?}", &network_request);
            match &network_request.protocol {
                NetworkProtocol::Http | NetworkProtocol::Https => {
                    let body = base64::engine::general_purpose::STANDARD.encode(&network_request.data);
                    let res = gloo_net::http::Request::post(&self.kdc_url)
                        .header("keep-alive", "true")
                        .body(body)
                        .map_err(|e| reason_err!("Error send KDC request", "{}", e))?
                        .send()
                        .await
                        .map_err(|e| reason_err!("Error send KDC request", "{}", e))?
                        .body()
                        .ok_or(general_err!("No body in response"))?
                        .as_string()
                        .ok_or(general_err!("No body in response"))?
                        .into_bytes();

                    // let res = self
                    //     .client
                    //     .post(&self.kdc_url)
                    //     .body(network_request.data.clone())
                    //     .send()
                    //     .await
                    //     .map_err(|e| reason_err!("Error send KDC request", "{}", e))?
                    //     .bytes()
                    //     .await
                    //     .map_err(|e| reason_err!("Error decode KDC response", "{}", e))?
                    //     .to_vec();

                    Ok(res)
                }
                _ => Err(general_err!("KDC Url must always start with HTTP/HTTPS for Web")),
            }
        })
    }
}

impl WasmNetworkClient {
    pub(crate) fn new(kdc_url: String) -> Self {
        Self {
            kdc_url,
            // client: reqwest::Client::new(),
        }
    }
}
