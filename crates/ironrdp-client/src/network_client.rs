use sspi::{Error, ErrorKind};
use std::net::{IpAddr, Ipv4Addr};
use std::{future::Future, pin::Pin};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use ironrdp::connector::{custom_err, general_err, ConnectorResult};
use reqwest::Client;
use url::Url;

use ironrdp_tokio::AsyncNetworkClient;
pub(crate) struct AsyncTokioNetworkClient {
    client: Option<Client>,
}
impl AsyncNetworkClient for AsyncTokioNetworkClient {
    fn send<'a>(
        &'a mut self,
        request: &'a sspi::generator::NetworkRequest,
    ) -> Pin<Box<dyn Future<Output = ConnectorResult<Vec<u8>>> + 'a>> {
        Box::pin(async move {
            match &request.protocol {
                sspi::network_client::NetworkProtocol::Tcp => self.send_tcp(&request.url, &request.data).await,
                sspi::network_client::NetworkProtocol::Udp => self.send_udp(&request.url, &request.data).await,
                sspi::network_client::NetworkProtocol::Http | sspi::network_client::NetworkProtocol::Https => {
                    self.send_http(&request.url, &request.data).await
                }
            }
        })
    }

    fn box_clone(&self) -> Box<dyn AsyncNetworkClient> {
        return Box::new(AsyncTokioNetworkClient {
            client: self.client.clone(),
        });
    }
}

impl AsyncTokioNetworkClient {
    pub(crate) fn new() -> Self {
        Self { client: None }
    }
}

impl AsyncTokioNetworkClient {
    async fn send_tcp(&self, url: &Url, data: &[u8]) -> ConnectorResult<Vec<u8>> {
        let addr = format!("{}:{}", url.host_str().unwrap_or_default(), url.port().unwrap_or(88));
        let mut stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{:?}", e)))
            .map_err(|e| custom_err!("sending KDC request over TCP ", e))?;

        stream
            .write(data)
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{:?}", e)))
            .map_err(|e| custom_err!("Sending KDC request over TCP ", e))?;

        let len = stream
            .read_u32()
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{:?}", e)))
            .map_err(|e| custom_err!("Sending KDC request over TCP ", e))?;

        let mut buf = vec![0; len as usize + 4];
        buf[0..4].copy_from_slice(&(len.to_be_bytes()));

        stream
            .read_exact(&mut buf[4..])
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{:?}", e)))
            .map_err(|e| custom_err!("Sending KDC request over TCP ", e))?;

        Ok(buf)
    }

    async fn send_udp(&self, url: &Url, data: &[u8]) -> ConnectorResult<Vec<u8>> {
        let udp_socket = UdpSocket::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .map_err(|e| custom_err!("Cannot bind udp socket", e))?;

        let addr = format!("{}:{}", url.host_str().unwrap_or_default(), url.port().unwrap_or(88));
        udp_socket
            .send_to(data, addr)
            .await
            .map_err(|e| custom_err!("Error sending udp request", e))?;

        // 48 000 bytes: default maximum token len in Windows
        let mut buf = vec![0; 0xbb80];

        let n = udp_socket
            .recv(&mut buf)
            .await
            .map_err(|e| custom_err!("Error receiving UDP request", e))?;

        let mut reply_buf = Vec::with_capacity(n + 4);
        reply_buf.extend_from_slice(&(n as u32).to_be_bytes());
        reply_buf.extend_from_slice(&buf[0..n]);

        Ok(reply_buf)
    }

    async fn send_http(&mut self, url: &Url, data: &[u8]) -> ConnectorResult<Vec<u8>> {
        if self.client.is_none() {
            self.client = Some(Client::new()); // dont drop the cllient, keep-alive
        }
        let result_bytes = self
            .client
            .as_ref()
            .ok_or_else(|| general_err!("Missing HTTP client, should never happen"))?
            .post(url.clone())
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| custom_err!("Sending KDC request over proxy", e))?
            .bytes()
            .await
            .map_err(|e| custom_err!("Receving KDC response", e))?
            .to_vec();

        Ok(result_bytes)
    }
}
