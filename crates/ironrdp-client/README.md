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

When the `gateway` feature is enabled and `Transport::Gateway` is selected, the client uses the MS-TSGU HTTPS WebSocket transport and falls back to legacy dual-channel HTTP when necessary.
It handles `HTTP_REAUTH_MESSAGE` through a short-lived background reauthentication transport without disrupting the active data transport.
The gateway path does not provide a live RPC-over-HTTP (RPCH) transport or RDG-UDP/DTLS side channel.

For the end-user RDP client binary, see [`ironrdp-viewer`](../ironrdp-viewer).

## Cloud discovery and browser handoff

The optional `cloud` feature exposes `cloud::CloudPcClient` for [Graph Cloud PC discovery], [Graph launch details], and direct-launch URL builders for [Windows App].
This is a browser handoff, **not native AVD or Windows 365 support** in `RdpClient`.
The module does not implement feed subscription, broker allocation, reverse-connect/WebSocket service transport, device bootstrap, or session authentication.

Acquire a delegated Microsoft Graph token with `CloudPC.Read.All` permission through your identity provider, then call `list_page(&token, None)` to discover Cloud PCs attributed to that user.
For a Windows 365 Cloud PC, call `retrieve_launch_detail(&token, &cloud_pc.id)` and open its service-provided `launch_url()` in a supported browser.
The API also reports whether the Cloud PC supports Windows 365 Switch.
The returned URL is browser connection state, not an RDP target or reverse-connect endpoint.
Alternatively, call `cloud_pc.launch_url(tenant_id)` to construct the documented direct-launch URL.
Direct launch requires a provisioned Windows 365 Enterprise or Flex Dedicated Cloud PC.
For AVD desktops and RemoteApps, call `avd_launch_url` with the workspace and resource **object IDs**, obtainable through Azure PowerShell as described in the Windows App documentation.
The resource must be assigned to the user; supply the hosting tenant ID for external identities.

Discovery and launch-detail retrieval use only the global Graph endpoint and return one page or launch result per call.
Pass the previous page's `next_link` to request another page, with a fresh token if necessary; callers control cancellation and page limits.
Requests validate TLS against platform roots, reject redirects and off-collection continuations, time out after 30 seconds, and limit each response to 1 MiB.
HTTP errors, including authentication failures and throttling, are returned without automatic retries.
The Graph bearer token is never stored or added to returned launch URLs, and it is **not** an RDP session credential.
Browser authentication is independent of the discovery token.
Live cloud interoperability still requires a licensed tenant, consent, assigned resources, and service access; offline tests do not establish those prerequisites.

[Graph Cloud PC discovery]: https://learn.microsoft.com/graph/api/user-list-cloudpcs?view=graph-rest-1.0
[Graph launch details]: https://learn.microsoft.com/graph/api/cloudpc-retrievecloudpclaunchdetail?view=graph-rest-1.0
[Windows App]: https://learn.microsoft.com/windows-app/direct-launch-urls

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
