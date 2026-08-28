# IronRDP RDP-UDP Tokio

Tokio driver wiring the RDP-UDP transport, TLS and the multitransport tunnel
into a usable async transport.

The client path implements reliable RDP-UDP2 only.
Legacy RDP-UDP v1/v2 data transfer, lossy RDP-UDP-L, FEC, and DTLS are not implemented.

This crate is the I/O layer for three sans-I/O pieces:

- [`ironrdp-rdpeudp`](../ironrdp-rdpeudp): reliable UDP with congestion control
- `tokio-rustls`: encryption over the RDP-UDP byte stream
- [`ironrdp-rdpemt`](../ironrdp-rdpemt): tunnel negotiation and data framing

## Architecture

```text
Application <-> UdpTransport (channels)
                     |
              Driver task (tokio::spawn)
              owns UdpSocket + RdpeudpConnection
                     |
              RdpeudpStream (AsyncRead/AsyncWrite adapter)
                     |
              TLS (tokio-rustls)
                     |
              multitransport tunnel (data framing)
```

The driver task owns the socket and the connection state machine. It is also
the only place in the stack that reads a clock: `RdpeudpConnection` takes the
current instant as an argument and reports the deadline it next wants, so the
state machine stays testable and free of ambient time.

## Usage

```rust,ignore
use ironrdp_rdpemt::TunnelConfig;
use ironrdp_rdpeudp_tokio::{UdpTransportConfig, connect_udp};
use ironrdp_tls::CertificateValidation;

let mut config = UdpTransportConfig::new(
    server_addr,
    "rdp-server.example.com".into(),
    TunnelConfig { request_id, security_cookie },
);
config.tls.certificate_validation = CertificateValidation::Strict;

let mut transport = connect_udp(config).await?;

// Send higher-layer data.
transport.send(dvc_frame).await?;

// Receive higher-layer data.
while let Some(data) = transport.recv().await {
    process_dvc_frame(&data);
}

transport.shutdown().await?;
```
