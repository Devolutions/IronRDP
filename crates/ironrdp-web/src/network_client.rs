use std::time::Duration;

use gloo_net::http::Request;
use ironrdp::connector::sspi::{
    self,
    network_client::{NetworkClient, NetworkClientFactory},
};
use url::Url;
use wasm_bindgen::JsValue;

#[derive(Debug)]
pub(crate) struct WasmNetworkClientFactory;

impl NetworkClientFactory for WasmNetworkClientFactory {
    fn network_client(&self) -> Box<dyn NetworkClient> {
        Box::new(WasmNetworkClient)
    }

    fn clone(&self) -> Box<dyn NetworkClientFactory> {
        Box::new(WasmNetworkClientFactory)
    }
}

struct WasmNetworkClient;

impl NetworkClient for WasmNetworkClient {
    fn send(&self, _: &Url, _: &[u8]) -> sspi::Result<Vec<u8>> {
        // FIXME: this trait should provide another method to advertise available network methods (in this case, only HTTP is supported)

        Err(sspi::Error::new(
            sspi::ErrorKind::NoAuthenticatingAuthority,
            "raw TCP / UDP sockets are not supported in web",
        ))
    }

    fn send_http(&self, url: &Url, data: &[u8], domain: Option<String>) -> sspi::Result<Vec<u8>> {
        // FIXME: sspi-rs should be updated so that implementer don’t need to re-implement the
        // the ASN.1 DER encoding / decoding dance himself (this logic can be extracted) in sspi-rs itself
        let data = {
            // NOTE: the logic looks like this:
            //
            // let domain = if let Some(domain) = domain {
            //     Some(ExplicitContextTag1::from(KerberosStringAsn1::from(
            //         IA5String::from_string(domain)?,
            //     )))
            // } else {
            //     None
            // };

            // let kdc_proxy_message = KdcProxyMessage {
            //     kerb_message: ExplicitContextTag0::from(OctetStringAsn1::from(data.to_vec())),
            //     target_domain: Optional::from(domain),
            //     dclocator_hint: Optional::from(None),
            // };
            //
            // let data = picky_asn1_der::to_vec(&kdc_proxy_message)?;

            let _ = domain;
            data
        };

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

    fn clone(&self) -> Box<dyn NetworkClient> {
        Box::new(WasmNetworkClient)
    }
}
