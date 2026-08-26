//! Test-only hooks for exercising the gateway HTTP transport without a TLS listener.

use hyper::body::Bytes;
use tokio::io::AsyncWriteExt as _;

use crate::http_auth::basic_authorization;
use crate::packet_io::{
    GatewayTransport as NetworkGatewayTransport, gateway_endpoint_is_valid as endpoint_is_valid,
    open_gateway_transport, open_test_transport,
    parse_proxy_url, proxy_from_values, read_http_connect_response,
};
use crate::{Error, GwClient, GwConnectTarget, GwConsentCallback};

/// In-memory gateway transport used by the registered integration tests.
pub struct GatewayTransport(NetworkGatewayTransport);

impl GatewayTransport {
    /// Connect the client side of the mock OUT and IN HTTP connections.
    pub async fn connect(
        out_stream: tokio::io::DuplexStream,
        in_stream: tokio::io::DuplexStream,
    ) -> Result<Self, String> {
        open_test_transport(out_stream, in_stream)
            .await
            .map(Self)
            .map_err(|error| error.to_string())
    }

    /// Open an HTTPS gateway transport with the production TLS connection path.
    pub async fn connect_tls(
        target: &GwConnectTarget,
        certificate_validation: ironrdp_tls::CertificateValidation,
        certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
    ) -> Result<Self, Error> {
        open_gateway_transport(target, certificate_validation, certificate_validation_callback)
            .await
            .map(|(transport, _)| Self(transport))
    }

    /// Return the authentication negotiated while opening this mock transport.
    pub fn session_authentication(&self) -> crate::GwSessionAuthentication {
        self.0.session_authentication
    }

    /// Send one MS-TSGU packet to the mock gateway.
    pub async fn send_packet(&mut self, packet: &[u8]) -> Result<(), String> {
        self.0.io.send_bytes(packet).await.map_err(|error| error.to_string())
    }

    /// Read one MS-TSGU packet from the mock gateway.
    pub async fn read_packet(&mut self) -> Result<Option<Bytes>, String> {
        self.0.io.read_packet_buf().await.map_err(|error| error.to_string())
    }

    /// Finish the mock gateway IN request body.
    pub async fn close(&mut self) -> Result<(), String> {
        self.0.io.close().await.map_err(|error| error.to_string())
    }

    /// Establish a gateway tunnel through this test transport.
    pub async fn connect_tunnel(
        self,
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        consent_callback: Option<&mut GwConsentCallback<'_>>,
    ) -> Result<GwClient, Error> {
        GwClient::connect_ws(target, client_name, server_port, self.0, consent_callback).await
    }

    /// Establish a mock tunnel with a selected session-authentication mode.
    pub async fn connect_tunnel_with_session_authentication(
        self,
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        session_authentication: crate::GwSessionAuthentication,
    ) -> Result<GwClient, Error> {
        let mut transport = self.0;
        transport.session_authentication = session_authentication;
        GwClient::connect_ws(target, client_name, server_port, transport, None).await
    }

    /// Establish a mock tunnel with a second mock transport reserved for reauthentication.
    pub async fn connect_tunnel_with_reauth(
        self,
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        reauthentication_transport: GatewayTransport,
    ) -> Result<GwClient, Error> {
        let mut reauthentication_transport = Some(reauthentication_transport.0);
        GwClient::connect_ws_with_reauth(
            target,
            client_name,
            server_port,
            self.0,
            Box::new(move || {
                Box::pin(core::future::ready(
                    reauthentication_transport
                        .take()
                        .ok_or_else(|| Error::new("mock reauthentication transport exhausted", crate::GwErrorKind::Connect)),
                ))
            }),
            None,
        )
        .await
    }
}

/// Evaluate a consent message received during tunnel creation.
pub fn evaluate_consent_message(
    consent_message: &[u8],
    consent_callback: Option<&mut GwConsentCallback<'_>>,
) -> Result<(), Error> {
    crate::evaluate_consent_message(consent_message, consent_callback)
}

/// Select a proxy from explicit environment variable values without changing process state.
pub fn proxy_summary(
    gateway_host: &str,
    https_proxy: Option<&str>,
    https_proxy_lowercase: Option<&str>,
    no_proxy: Option<&str>,
    no_proxy_lowercase: Option<&str>,
) -> Result<Option<String>, String> {
    proxy_from_values(
        gateway_host,
        443,
        https_proxy
            .map(str::to_owned)
            .or_else(|| https_proxy_lowercase.map(str::to_owned)),
        no_proxy
            .map(str::to_owned)
            .or_else(|| no_proxy_lowercase.map(str::to_owned)),
    )
    .map(|proxy| proxy.map(|proxy| format!("{}://{}:{}", proxy.scheme.name(), proxy.host, proxy.port)))
    .map_err(|error| error.to_string())
}

/// Validate a gateway endpoint without opening a connection.
pub fn gateway_endpoint_is_valid(endpoint: &str) -> bool {
    endpoint_is_valid(endpoint)
}

/// Verify that a proxy URL can construct a Basic CONNECT authorization header.
pub fn proxy_uses_basic_authorization(proxy_url: &str) -> Result<bool, String> {
    parse_proxy_url(proxy_url)
        .map(|proxy| {
            proxy.credentials.as_ref().is_some_and(|credentials| {
                basic_authorization(&credentials.username, &credentials.password).starts_with("Basic ")
            })
        })
        .map_err(|error| error.to_string())
}

/// Format a proxy configuration with credentials redacted.
pub fn proxy_debug(proxy_url: &str) -> Result<String, String> {
    parse_proxy_url(proxy_url)
        .map(|proxy| format!("{proxy:?}"))
        .map_err(|error| error.to_string())
}

/// Read an HTTP CONNECT response from an in-memory stream.
pub async fn validate_proxy_response(response: &[u8]) -> Result<(), String> {
    let (client, mut server) = tokio::io::duplex(response.len().max(1));
    server.write_all(response).await.map_err(|error| error.to_string())?;
    drop(server);
    read_http_connect_response(Box::new(client))
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}
