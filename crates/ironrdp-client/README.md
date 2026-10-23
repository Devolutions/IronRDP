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

The graphics pipeline (MS-RDPEGFX) and video optimized remoting share one H.264 decoder: on Windows the one shipped with Windows, through Media Foundation; the `openh264` feature switches the default to a decoder built from the bundled OpenH264 sources, which is the only default elsewhere. `ConfigBuilder::with_h264_decoder` replaces it, or removes it with `None`. The graphics pipeline's decoder starts during the connection sequence and is awaited when the graphics channel opens, so a slow decoder start does not delay connecting; if it fails, no AVC capability set is advertised.
With a decoder, the graphics pipeline is advertised by default with capability versions 10.7 down to 10 (AVC444 and AVC444v2), 8.1 (AVC420) and 8; `ConfigBuilder::with_graphics_pipeline(false)` or the `ironrdp_egfx:i:0` `.rdp` property opts out, as mstsc has no `.rdp` key for it. Without a decoder the graphics pipeline is off unless enabled the same way, and then only offers the codecs that need no H.264 decoder.
The optional `video` feature accepts MS-RDPEVOR video optimized remoting and MS-RDPEGT geometry tracking when the H.264 decoder is available. Without a decoder, or with `videoplaybackmode:i:0` as in mstsc, the client keeps declining these channels.
Windows servers only keep the video channels open when the graphics pipeline is enabled.
Decoded pictures are shown at their presentation time, scaled and clipped to the tracked geometry, and composited into the framebuffer like graphics pipeline updates.

When the `gateway` feature is enabled, `Transport::Gateway` uses the MS-TSGU HTTPS WebSocket transport and falls back to legacy dual-channel HTTP when necessary.
It handles `HTTP_REAUTH_MESSAGE` through a short-lived background reauthentication transport without disrupting the active data transport.
Select `TransportKind::GatewayRpch` explicitly to use the authenticated MS-RPCH v2 and TS Proxy stream.
RPCH verifies RPC packet integrity independently of TLS and HTTP authentication; the legacy gateway selection remains the default, and neither transport falls back to unsigned RPC.
The RPCH stream recycles IN and OUT channels, keeps load-balancer cookies and watches the IN channel for gateway errors.
It accepts gateway consent messages, logs service messages and answers reauthentication on a separate authenticated connection.
Gateway UDP/DTLS remains unsupported.
An RPCH session through one Windows RD Gateway has been tested, including IN-channel recycling with a reduced channel lifetime.
Dedicated TCP-channel tests through that gateway also verified OUT-channel recycling, consent and service messages, and repeated reauthentication during a 1.25 GiB transfer.

The optional `usb` feature redirects local USB devices over MS-RDPEUSB (URBDRC) with the nusb backend from [`ironrdp-rdpeusb-native`](../ironrdp-rdpeusb-native).
Select devices with `ConfigBuilder::with_usb_redirection` or the `.rdp` `usbdevicestoredirect` property.
On Windows, only devices bound to the WinUSB driver can be redirected; see that crate for the coverage limits on each platform.
Through an RD Gateway, USB redirection follows the gateway's Plug and Play device-redirection policy.

For the end-user RDP client binary, see [`ironrdp-viewer`](../ironrdp-viewer).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
