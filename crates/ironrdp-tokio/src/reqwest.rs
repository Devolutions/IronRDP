use core::future::Future;
use core::net::{IpAddr, Ipv4Addr};
use core::pin::Pin;

use ironrdp_connector::{custom_err, ConnectorResult};
use reqwest::Client;
use sspi::{Error, ErrorKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use url::Url;

use crate::AsyncNetworkClient;

pub struct ReqwestNetworkClient {
    client: Option<Client>,
}

impl AsyncNetworkClient for ReqwestNetworkClient {
    fn send<'a>(
        &'a mut self,
        request: &'a sspi::generator::NetworkRequest,
    ) -> Pin<Box<dyn Future<Output = ConnectorResult<Vec<u8>>> + 'a>> {
        ReqwestNetworkClient::send(self, request)
    }
}

impl ReqwestNetworkClient {
    pub fn new() -> Self {
        Self { client: None }
    }
}

impl Default for ReqwestNetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestNetworkClient {
    pub fn send<'a>(
        &'a mut self,
        request: &'a sspi::generator::NetworkRequest,
    ) -> Pin<Box<dyn Future<Output = ConnectorResult<Vec<u8>>> + Send + 'a>> {
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

    async fn send_tcp(&self, url: &Url, data: &[u8]) -> ConnectorResult<Vec<u8>> {
        let addr = format!("{}:{}", url.host_str().unwrap_or_default(), url.port().unwrap_or(88));

        let mut stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{e:?}")))
            .map_err(|e| custom_err!("failed to send KDC request over TCP", e))?;

        stream
            .write(data)
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{e:?}")))
            .map_err(|e| custom_err!("failed to send KDC request over TCP", e))?;

        let len = stream
            .read_u32()
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{e:?}")))
            .map_err(|e| custom_err!("failed to send KDC request over TCP", e))?;

        let mut buf = vec![0; len as usize + 4];
        buf[0..4].copy_from_slice(&(len.to_be_bytes()));

        stream
            .read_exact(&mut buf[4..])
            .await
            .map_err(|e| Error::new(ErrorKind::NoAuthenticatingAuthority, format!("{e:?}")))
            .map_err(|e| custom_err!("failed to send KDC request over TCP", e))?;

        Ok(buf)
    }

    async fn send_udp(&self, url: &Url, data: &[u8]) -> ConnectorResult<Vec<u8>> {
        let udp_socket = UdpSocket::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .map_err(|e| custom_err!("cannot bind UDP socket", e))?;

        let addr = format!("{}:{}", url.host_str().unwrap_or_default(), url.port().unwrap_or(88));

        udp_socket
            .send_to(data, addr)
            .await
            .map_err(|e| custom_err!("failed to send UDP request", e))?;

        // 48 000 bytes: default maximum token len in Windows
        let mut buf = vec![0; 0xbb80];

        let n = udp_socket
            .recv(&mut buf)
            .await
            .map_err(|e| custom_err!("failed to receive UDP request", e))?;
        let buf = &buf[0..n];

        let mut reply_buf = Vec::with_capacity(n + 4);
        let n = u32::try_from(n).map_err(|e| custom_err!("invalid length", e))?;
        reply_buf.extend_from_slice(&n.to_be_bytes());
        reply_buf.extend_from_slice(buf);

        Ok(reply_buf)
    }

    async fn send_http(&mut self, url: &Url, data: &[u8]) -> ConnectorResult<Vec<u8>> {
        let client = self.client.get_or_insert_with(Client::new);

        let response = client
            .post(url.clone())
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| custom_err!("failed to send KDC request over proxy", e))?
            .error_for_status()
            .map_err(|e| custom_err!("KdcProxy", e))?;

        let body = response
            .bytes()
            .await
            .map_err(|e| custom_err!("failed to receive KDC response", e))?;

        // The type bytes::Bytes has a special From implementation for Vec<u8>.
        let body = Vec::from(body);

        Ok(body)
    }
}
