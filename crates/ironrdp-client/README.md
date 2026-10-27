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

The optional `entra-auth` feature provides an experimental implementation of the [MS-RDPBCGR] RDS AAD authorization-code and proof-of-possession flow.
Call `EntraAuthorization::begin(client_id, redirect_uri, resource)` with an application client ID you are authorized to use and its registered redirect URI; there are no first-party registration defaults.
The method returns a browser URL; the embedder must capture the `form_post` response at its redirect URI and pass the authorization code and state to `complete`.
An app registration and redirect URI alone do not establish permission to obtain RDP resource tokens; third-party registration acceptance has not been verified.
Attach the resulting `EntraAuthSession` with `ConfigBuilder::with_entra_auth` to authenticate the RDP session using RDS AAD over strictly validated TLS, without a CredSSP or password fallback.
The high-level client supports direct and RDS Gateway transports for Entra authentication; it rejects RDCleanPath because that path does not validate the RDP server's TLS certificate.
Entra authentication also rejects certificate-validation override callbacks.
Certificate validation uses platform trust and the server's DNS name; Entra P2P certificate validation using tenant roots and a device ID is not implemented.
The embedder owns browser UI and must choose the target host name or device ID that matches the RDP server.
This flow uses Microsoft's documented public-cloud authority and requires a new authorization flow after the access token expires.
When combined with `gateway`, `gatewaybrokeringtype:i:1` enables Azure Virtual Desktop orchestration over WebSocket, acquiring gateway and session-host PoP tokens through Windows Web Account Manager on Windows, `OAuthAvdTokenProvider` (loopback PKCE, device code, and refresh-token cache) across platforms, or a custom `AvdTokenProvider` set with `ConfigBuilder::with_avd_token_provider`.
Live Entra host interoperability has not been verified.

TLS peer-certificate validation remains disabled by default for compatibility with
existing deployments. `ConfigBuilder` exposes an explicit
`CertificateValidation::Strict` policy for callers that require platform-root and
server-name validation.

Cloud PC, AVD feed, and Entra HTTP requests use platform-root validation and the client's selected TLS backend.

The experimental `remote-guard` feature forwards [Remote Credential Guard][rcg-build] through the Tokio and async drivers.
Select `ConfigBuilder::with_remote_guard_mode` with `RemoteGuardMode::Negotiate`, `Ntlm`, or `Kerberos` from `ironrdp_client::config`, and explicitly select strict TLS validation.
All modes require Kerberos for initial authentication; the mode mirrors CredSSP package selection, with `Negotiate` permitting both Kerberos and outbound NTLMv2 redirection and the other two restricting redirection to one package.
The client retains the CredSSP context and registers MS-RDPEAR on its existing DRDYNVC carrier.
Without an explicit Kerberos configuration, RCG enables DNS KDC discovery using the configured client computer name.
The account realm must be DNS-resolvable; SAM/NetBIOS identities require an explicit KDC URL.
Unsupported builds, invalid policies, insecure transports, RDCleanPath, VMConnect, and automatic reconnect fail rather than downgrading authentication.
The feature is off by default and is not included in `all`.

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

## Cloud discovery and browser handoff

The optional `cloud` feature exposes `cloud::CloudPcClient` for [Graph Cloud PC discovery], [Graph launch details], direct-launch URL builders for [Windows App], AVD RDWeb feed discovery, and PKCS#7 `.rdp` signature verification.
The module does not implement broker allocation, reverse-connect/WebSocket service transport, device bootstrap, or session authentication.

Acquire a delegated Microsoft Graph token with `CloudPC.Read.All` permission through your identity provider, then call `list_page(&token, None)` to discover Cloud PCs attributed to that user.
For a Windows 365 Cloud PC, call `retrieve_launch_detail(&token, &cloud_pc.id)` and open its service-provided `launch_url()` in a supported browser.
The API also reports whether the Cloud PC supports Windows 365 Switch.
The returned URL is browser connection state, not an RDP target or reverse-connect endpoint.
Alternatively, call `cloud_pc.launch_url(tenant_id)` to construct the documented direct-launch URL.
Direct launch requires a provisioned Windows 365 Enterprise or Flex Dedicated Cloud PC.
For AVD desktops and RemoteApps, call `avd_launch_url` with the workspace and resource **object IDs**, obtainable through Azure PowerShell as described in the Windows App documentation.
The resource must be assigned to the user; supply the hosting tenant ID for external identities.

For AVD RDWeb feeds, build the discovery URL with `AvdCloudEnvironment::feed_discovery_url`, call `discover_avd_feeds` and `fetch_avd_workspace_feed` with an RDWeb bearer token, and call `download_avd_rdp_file` or `verify_rdp_signature` to validate the detached PKCS#7 signature and `signscope` sensitive-setting coverage on signed `.rdp` files.
Signature verification loads platform trust anchors through `rustls-native-certs` independently of the HTTP client's selected TLS backend, and skips platform root entries that cannot be decoded as X.509 DER certificates.

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
[MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
[rcg-build]: https://docs.rs/ironrdp-connector#experimental-remote-credential-guard
