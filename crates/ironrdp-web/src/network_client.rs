use std::time::Duration;

use gloo_net::http::Request;
use ironrdp::connector::sspi::{
    self,
    network_client::{NetworkClient, NetworkClientFactory, NetworkProtocol},
};
use url::Url;
use wasm_bindgen::JsValue;

#[derive(Debug)]
pub(crate) struct WasmNetworkClientFactory;

impl NetworkClientFactory for WasmNetworkClientFactory {
    fn network_client(&self) -> Box<dyn NetworkClient> {
        Box::new(WasmNetworkClient)
    }

    fn box_clone(&self) -> Box<dyn NetworkClientFactory> {
        Box::new(WasmNetworkClientFactory)
    }
}

struct WasmNetworkClient;

impl WasmNetworkClient {
    const NAME: &str = "Wasm";
    const SUPPORTED_PROTOCOLS: &[NetworkProtocol] = &[NetworkProtocol::Http, NetworkProtocol::Https];
}

impl NetworkClient for WasmNetworkClient {
    fn send(&self, _protocol: NetworkProtocol, url: &Url, data: &[u8]) -> sspi::Result<Vec<u8>> {
        let length = JsValue::from_f64(data.len() as f64);
        let payload = js_sys::Uint8Array::new(&length);
        payload.copy_from(data);

        let fut = Request::new(url.as_str()).body(payload).send();

        let (tx, rx) = std::sync::mpsc::sync_channel(0); // rendezvous channel

        wasm_bindgen_futures::spawn_local(async move {
            match fut.await {
                Ok(response) => {
                    let result = response.binary().await;
                    let _ = tx.send(result);
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                }
            }
        });

        let response = rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| sspi::Error::new(sspi::ErrorKind::InternalError, e.to_string()))?
            .map_err(|e| sspi::Error::new(sspi::ErrorKind::NoAuthenticatingAuthority, e.to_string()))?;

        Ok(response)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn supported_protocols(&self) -> &[sspi::network_client::NetworkProtocol] {
        Self::SUPPORTED_PROTOCOLS
    }

    fn box_clone(&self) -> Box<dyn NetworkClient> {
        Box::new(WasmNetworkClient)
    }
}
