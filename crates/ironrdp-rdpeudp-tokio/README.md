# IronRDP RDP-UDP Tokio

Tokio driver wiring the RDP-UDP transport, TLS and the multitransport tunnel
into a usable async transport.

The client path implements reliable RDP-UDP2 only.
Legacy RDP-UDP v1/v2 data transfer, lossy RDP-UDP-L, FEC, and DTLS are not implemented.

The default `connect_udp` path remains direct UDP and is unchanged.
The optional `ice` feature adds UDP ICE candidate gathering, connectivity checks, nominated-pair selection, and UDP TURN relay support beneath the existing RDPEUDP2 + TLS + RDPEMT transport.
Enable `ice` and one Rustls provider feature, such as `rustls-ring` or `rustls-aws-lc-rs`.
Configure local STUN/TURN servers and TURN credentials, then exchange the gathered local credentials and candidates with the peer through the embedder's signaling system.
Pass the peer's ICE credentials and candidate strings to `connect_udp_with_ice` or `accept_udp_with_ice`.
Existing multitransport clients can use `MultitransportBootstrap::connect_with_ice` to preserve the request/response flow.
IronRDP does not define a signaling protocol or claim interoperability with AVD Shortpath.
The ICE API accepts only UDP candidates for component 1 with IP-literal addresses, STUN URLs, and IPv4 UDP TURN URLs.
Gathering is limited to 8 ICE servers and retains at most 64 candidates, and each ICE operation has a deadline of at most 120 seconds.
Over a TURN relay candidate, the outer network path must carry the 1232-byte RDPEUDP2 SYN datagram plus TURN encapsulation overhead, and server-side TURN allocations expire at their server-assigned lifetime after local shutdown.
The relay-only integration test uses a local UDP TURN fixture and verifies an RDPEUDP2 transfer over a nominated relay-to-relay pair.
Interoperability with a deployed TURN service has not been validated.

For a one-shot STUN probe, use `query_server_reflexive_address` and pass the same bound socket to `connect_udp_with_socket`.
The probe sends one request without retransmission and reports a mapping, not proof that a remote peer can reach that address through its NAT.

This crate is the I/O layer for three sans-I/O pieces:

- [`ironrdp-rdpeudp`](https://docs.rs/ironrdp-rdpeudp): reliable UDP with congestion control
- `tokio-rustls`: encryption over the RDP-UDP byte stream
- [`ironrdp-rdpemt`](https://docs.rs/ironrdp-rdpemt): tunnel negotiation and data framing

## Architecture

```text
Application <-> UdpTransport (channels)
                     |
              Driver task (tokio::spawn)
              owns direct UdpSocket or selected ICE datagram connection
              plus RdpeudpConnection
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

An RDP multitransport request still bootstraps RDPEUDP2 and its RDPEMT tunnel over the selected datagram path ([MS-RDPEUDP2] §3.1.3, [MS-RDPEMT] §3.1.3).
The standalone STUN probe follows [RFC 8489], ICE candidate gathering and checks follow [RFC 8445], and UDP TURN relay behavior follows [RFC 8656].

[RFC 8489]: https://www.rfc-editor.org/rfc/rfc8489
[RFC 8445]: https://www.rfc-editor.org/rfc/rfc8445
[RFC 8656]: https://www.rfc-editor.org/rfc/rfc8656
[MS-RDPEUDP2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp2/
[MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/
