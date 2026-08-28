# IronRDP client

Reusable RDP client engine library built on top of the IronRDP crates suite.

This crate is **library-only**: it exposes the `Config`, the `RdpClient`
runtime, input/output event types, the WebSocket transport, and the session driver. It is
consumed by `ironrdp-viewer` (the portable GUI client binary) and by any other embedder
(for example, a headless agent).

The library is winit-agnostic. Output events are emitted on a bounded
`tokio::sync::mpsc::Sender<RdpOutputEvent>` channel: the embedder is responsible
for consuming them and dispatching them to whatever event loop or runtime it wishes.

TLS peer-certificate validation remains disabled by default for compatibility with
existing deployments. `ConfigBuilder` exposes an explicit
`CertificateValidation::Strict` policy for callers that require platform-root and
server-name validation.

With the `gateway` feature, the client uses the MS-TSGU HTTPS WebSocket transport and falls back to legacy dual-channel HTTP when necessary.
It handles `HTTP_REAUTH_MESSAGE` through a short-lived background reauthentication transport without disrupting the active data transport.
The gateway path does not provide a live RPC-over-HTTP (RPCH) transport or RDG-UDP/DTLS side channel.

For the end-user RDP client binary, see [`ironrdp-viewer`](../ironrdp-viewer).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
