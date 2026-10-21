# IronRDP client

Reusable RDP client engine library built on top of the IronRDP crates suite.

This crate is **library-only**: it exposes the `Config`, the `RdpClient`
runtime, input/output event types, the WebSocket transport, and the session driver. It is
consumed by `ironrdp-viewer` (the portable GUI client binary) and by any other embedder
(for example, a headless agent).

The optional `udp` feature enables reliable RDP-UDP2 multitransport for direct enhanced-security connections.
Opt in with `ConfigBuilder::with_udp_transport(true)`.
The client negotiates Soft-Sync and routes only selected dynamic channels over reliable UDP.
Bootstrap failures and sideband loss before channel migration disable UDP and continue over TCP.
After Soft-Sync migrates a channel, sideband loss ends the connection so automatic reconnect can create fresh security, cookie, correlation, and socket state.
Gateway, RDCleanPath, named-pipe, Hyper-V VM Connect, and standard RDP security transports are TCP-only; `prefer_direct` may use UDP only for its direct attempt.
Legacy RDP-UDP v1/v2 data transfer and lossy RDP-UDP-L are not supported.

The library is winit-agnostic. Output events are emitted on a bounded
`tokio::sync::mpsc::Sender<RdpOutputEvent>` channel: the embedder is responsible
for consuming them and dispatching them to whatever event loop or runtime it wishes.

TLS peer-certificate validation remains disabled by default for compatibility with
existing deployments. `ConfigBuilder` exposes an explicit
`CertificateValidation::Strict` policy for callers that require platform-root and
server-name validation.

The optional `camera` feature enables MS-RDPECAM camera redirection, selected with `ConfigBuilder::with_cameras` or the `camerastoredirect` `.rdp` property.
Capture uses Media Foundation and is only available on Windows; cameras attached or removed during the session are advertised or withdrawn as they come and go.
Raw camera frames are encoded to H.264 unless the camera produces H.264 or MJPEG itself or `CameraConfig::encode_video` is off.

The optional `video` feature accepts MS-RDPEVOR video optimized remoting and MS-RDPEGT geometry tracking when an H.264 decoder is configured with `ConfigBuilder::with_h264_decoder`; without one, the client keeps declining these channels.
The `openh264` feature configures a decoder built from the bundled OpenH264 sources by default.
Decoded pictures are shown at their presentation time, scaled and clipped to the tracked geometry, and composited into the framebuffer like graphics pipeline updates.

When the `gateway` feature is enabled and `Transport::Gateway` is selected, the client uses the MS-TSGU HTTPS WebSocket transport and falls back to legacy dual-channel HTTP when necessary.
It handles `HTTP_REAUTH_MESSAGE` through a short-lived background reauthentication transport without disrupting the active data transport.
The gateway path does not provide a live RPC-over-HTTP (RPCH) transport or RDG-UDP/DTLS side channel.

For the end-user RDP client binary, see [`ironrdp-viewer`](../ironrdp-viewer).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
