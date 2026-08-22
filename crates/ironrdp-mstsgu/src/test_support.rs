//! Test-only hooks for exercising the gateway HTTP transport without a TLS listener.

use hyper::body::Bytes;

use crate::packet_io::{PacketIo, open_test_transport};

/// In-memory gateway transport used by the registered integration tests.
pub struct GatewayTransport(PacketIo);

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

    /// Send one MS-TSGU packet to the mock gateway.
    pub async fn send_packet(&mut self, packet: &[u8]) -> Result<(), String> {
        self.0.send_bytes(packet).await.map_err(|error| error.to_string())
    }

    /// Read one MS-TSGU packet from the mock gateway.
    pub async fn read_packet(&mut self) -> Result<Option<Bytes>, String> {
        self.0.read_packet_buf().await.map_err(|error| error.to_string())
    }

    /// Finish the mock gateway IN request body.
    pub async fn close(&mut self) -> Result<(), String> {
        self.0.close().await.map_err(|error| error.to_string())
    }
}
