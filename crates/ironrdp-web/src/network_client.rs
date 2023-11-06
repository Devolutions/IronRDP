use std::pin::Pin;

use futures_util::Future;
use ironrdp::connector::sspi::generator::NetworkRequest;
use ironrdp::connector::sspi::network_client::NetworkProtocol;
use ironrdp::connector::{general_err, reason_err, ConnectorResult};
use ironrdp_futures::AsyncNetworkClient;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::ReadableStream;

#[derive(Debug)]
pub(crate) struct WasmNetworkClient {
    kdc_url: String,
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
                    let body = js_sys::Uint8Array::from(&network_request.data[..]);

                    let stream = gloo_net::http::Request::post(&self.kdc_url)
                        .header("keep-alive", "true")
                        .body(body)
                        .map_err(|e| reason_err!("Error send KDC request", "{}", e))?
                        .send()
                        .await
                        .map_err(|e| reason_err!("Error send KDC request", "{}", e))?
                        .body()
                        .ok_or(general_err!("No body in response"))?;
                    let res = read_stream(stream).await?;

                    Ok(res)
                }
                _ => Err(general_err!("KDC Url must always start with HTTP/HTTPS for Web")),
            }
        })
    }
}

impl WasmNetworkClient {
    pub(crate) fn new(kdc_url: String) -> Self {
        Self { kdc_url }
    }
}

pub async fn read_stream(stream: ReadableStream) -> ConnectorResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let reader = web_sys::ReadableStreamDefaultReader::new(&stream).map_err(|_| general_err!("error create reader"))?;

    loop {
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|_e| general_err!("error read stream"))?;

        // Cast the result into an object and check if the stream is done
        let result_obj = result.dyn_into::<js_sys::Object>().unwrap();
        let done = js_sys::Reflect::get(&result_obj, &"done".into())
            .map_err(|_| general_err!("error read stream"))?
            .as_bool()
            .ok_or(general_err!("error resolve reader promise proerty: done"))?;
        if done {
            break;
        }

        // Extract the value (the chunk) from the result
        let value = js_sys::Reflect::get(&result_obj, &"value".into()).unwrap();

        // Convert value to Uint8Array, then to Vec<u8> and append to bytes
        let chunk = js_sys::Uint8Array::new(&value);
        bytes.extend_from_slice(&chunk.to_vec());
    }

    Ok(bytes)
}
