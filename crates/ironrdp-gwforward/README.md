# ironrdp-gwforward

Generic TCP forwarding and SOCKS5 proxying over a Microsoft RD Gateway.

The RD Gateway tunneling protocol ([MS-TSGU]) relays a TCP byte stream to any reachable
target host and port; it is not limited to RDP. This crate builds on
[`ironrdp-mstsgu`] to expose that capability to ordinary, non-RDP programs.

## What it provides

- **Local port forward** ([`run_port_forward`]) — SSH `-L`-style. Listen on a local port
  and relay each inbound connection to a fixed `host:port` through the gateway.
- **SOCKS5 proxy** ([`run_socks5`]) — a local SOCKS5 server (CONNECT, no auth). Any
  SOCKS5-capable client (for example `curl --socks5`) names its destination per
  connection and is tunnelled there through the gateway.

Each inbound connection opens an independent gateway tunnel and relays bytes
bidirectionally with `tokio::io::copy_bidirectional`.

## Contract

Callers supply a [`GatewayTunnelConfig`] (gateway endpoint, credentials, transport, and
TLS certificate policy) plus a local listen address. The crate owns the accept loops and
the per-connection tunnel setup; it does not manage RDP sessions.

The default transport is the modern WebSocket path; [`GatewayTransport::Rpc`] selects the
legacy RPC-over-HTTP transport for gateways without WebSocket support.

## Example

```rust,ignore
use ironrdp_gwforward::{GatewayTunnelConfig, run_socks5};

let config = GatewayTunnelConfig {
    gateway_endpoint: "rdg.contoso.com:443".into(),
    username: "CONTOSO\\alice".into(),
    password: "...".into(),
    ..Default::default()
};
run_socks5(config, "127.0.0.1:1080").await?;
```

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/
[`ironrdp-mstsgu`]: ../ironrdp-mstsgu/README.md
[`run_port_forward`]: ./src/forward.rs
[`run_socks5`]: ./src/forward.rs
[`GatewayTunnelConfig`]: ./src/tunnel.rs
[`GatewayTransport::Rpc`]: ./src/tunnel.rs
