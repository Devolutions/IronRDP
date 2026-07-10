# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-client-v0.1.0)] - 2026-07-10

### <!-- 0 -->Security

- Add DVC COM plugin loader for native Windows DVC client plugins ([9c987bcb40](https://github.com/Devolutions/IronRDP/commit/9c987bcb40a12712fa649e1087f6cb922f9bb75c)) 

  Implements support for loading and using native Windows Dynamic Virtual
  Channel (DVC) client plugin DLLs through the COM-based IWTSPlugin API.
  This enables IronRDP to leverage existing Windows DVC plugins such as
  webauthn.dll for hardware security key support via RDP.
  
  New crate: ironrdp-dvc-com-plugin
  - Implements IWTSVirtualChannelManager and IWTSVirtualChannel COM interfaces
  - Manages plugin lifecycle on dedicated COM worker thread
  - Handles channel open/close/reopen cycles with per-instance write callbacks
  - Properly bridges between COM synchronous calls and IronRDP's async runtime
  
  Client integration:
  - Add --dvc-plugin CLI argument to ironrdp-client
  - Load plugins in both TCP and WebSocket connection paths
  - Windows-only conditional compilation for cross-platform builds
  
  Additional fixes:
  - Fix pre-existing crash in ironrdp-tokio KDC handler on 64-bit Windows
    (usize to u32 conversion in reqwest.rs)
  - Add proper error handling using try_from instead of unsafe as casts
  - All changes pass cargo fmt and cargo clippy with strict pedantic lints
  
  Tested with: C:\Windows\System32\webauthn.dll

- Add alternate_shell and work_dir configuration support ([#1095](https://github.com/Devolutions/IronRDP/issues/1095)) ([a33d27fe67](https://github.com/Devolutions/IronRDP/commit/a33d27fe6771a5a155161ef40a04de88803dd84c)) 

  Add support for configuring `alternate_shell` and `work_dir` fields in
  ClientInfoPdu, which are used by:
    - CyberArk PSM (Privileged Session Manager) for session tokens
    - Remote application scenarios (RemoteApp)
    - Custom shell configurations

- Dispatch multitransport PDUs on IO channel ([#1096](https://github.com/Devolutions/IronRDP/issues/1096)) ([7853e3cc6f](https://github.com/Devolutions/IronRDP/commit/7853e3cc6f26acaf3da000c6177ca3cef6ef85fd)) 

  `decode_io_channel()` assumes all IO channel PDUs begin with
  a`ShareControlHeader`. Multitransport Request PDUs use a
  `BasicSecurityHeader` with `SEC_TRANSPORT_REQ` instead ([MS-RDPBCGR]
  2.2.15.1).
  
  This adds a peek-based dispatch: check the first `u16`
  for`TRANSPORT_REQ`, decode as `MultitransportRequestPdu` if set,
  otherwise fall through to the existing `decode_share_control()` path
  unchanged.
  
  The new variant is propagated through `ProcessorOutput` and
  'ActiveStageOutput` so applications can handle multitransport requests.
  Client and web consumers log the request (no UDP transport yet).

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 1 -->Features

- Make client_codecs_capabilities() configurable ([783702962a](https://github.com/Devolutions/IronRDP/commit/783702962a2e842f9d5046ac706048ba124e1401)) 

- Add --codecs ([3d1762c777](https://github.com/Devolutions/IronRDP/commit/3d1762c777e756b1deb4941de5534662b19d4799)) 

- Support for hardware cursor ([#804](https://github.com/Devolutions/IronRDP/issues/804)) ([1236a9be99](https://github.com/Devolutions/IronRDP/commit/1236a9be994c1c947bf1ea82a4c340885b531170)) 

- Add DVC named pipe proxy support ([#791](https://github.com/Devolutions/IronRDP/issues/791)) ([5482365655](https://github.com/Devolutions/IronRDP/commit/5482365655e5c171cd967eda401b01161a9f6602)) 

- Inital support for .RDP files ([#862](https://github.com/Devolutions/IronRDP/issues/862)) ([c710909a3c](https://github.com/Devolutions/IronRDP/commit/c710909a3cb64808bfc024bbe3f326565268871e)) 

  This is paving the way for .rdp file support.

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a
  similar size of PNG, while offering 20x-50x faster encoding and 3x-4x
  faster decoding.
  
  Add a new QOI codec (UUID 4dae9af8-b399-4df6-b43a-662fd9c0f5d6) for
  SetSurface command. The PDU data contains the QOI header (14 bytes) +
  data "chunks" and the end marker (8 bytes).
  
  Some benchmarks showing interesting results (using ironrdp/perfenc)

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec (UUID 229cc6dc-a860-4b52-b4d8-053a22b3892b) for
  SetSurface command. The PDU data contains the same data as the QOI
  codec, with zstd compression.
  
  Some benchmarks showing interesting results (using ironrdp/perfenc)

- Add MS-TSGU (Microsoft RD Gateway) support ([#913](https://github.com/Devolutions/IronRDP/issues/913)) ([7d28ef83a6](https://github.com/Devolutions/IronRDP/commit/7d28ef83a67afa8f69a7170ff47f4547a5d01b1e)) 

  This adds a working state to connect with the ironrdp-client CLI against
  a server behind a microsoft remote desktop gateway.
  During my testing this was robust enough to work with sessions for more
  than 30 minutes.
  
  CLI Flags and prompts are implemented and can be mixed, so the following
  would prompt only for 2 passwords:
  > ironrdp-client --gw-user username@domain --gw-endpoint rdp.gw.host:443
  -u username@domain rdp.internal.host
  
  [MS-TSGU]
  https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
  * This implements a MVP (in terms of recentness) state needed to connect
  through microsoft rdp gateway.
  * This only supports the HTTPS protocol with Websocket (and not the
  legacy HTTP, HTTP-RPC or UDP protocols).
  * This does not implement reconnection/reauthentication.
  * This only supports basic auth.
  
  Mostly looking for rough initial feedback (e.g. in terms of if there are
  parts that dont align with projects architecture or other areas needing
  major rework) as well as if the implemented scope would be considered
  complete enough to land this in the first place.

- Add an option to specify a timezone ([#917](https://github.com/Devolutions/IronRDP/issues/917)) ([6fab9f8228](https://github.com/Devolutions/IronRDP/commit/6fab9f8228578b3c78db131b3c2e0526352116a9)) 

  Allows to pass a timezone to the remote desktop.

- Preserve RDP negotiation failure details in RDCleanPath error responses ([#930](https://github.com/Devolutions/IronRDP/issues/930)) ([ca11e338d7](https://github.com/Devolutions/IronRDP/commit/ca11e338d7231c86f60a110627a5d864377d8594)) 

  * Both web and desktop clients check for X.224 negotiation failure data
  in RDCleanPath error responses before falling back to generic errors
  * When X.224 Connection Confirm failure is found, convert to specific
  NegotiationFailure error type instead of generic RDCleanPath error
  * Enable clients to show meaningful error messages like "CredSSP
  authentication required" instead of generic connection failures
  * Maintain backward compatibility - existing proxies sending empty
  x224_connection_pdu continue working as before
  * Helper for proxies creating an RDCleanPath error with server response

- [**breaking**] Return x509_cert::Certificate from upgrade() ([#1054](https://github.com/Devolutions/IronRDP/issues/1054)) ([bd2aed7686](https://github.com/Devolutions/IronRDP/commit/bd2aed76867f4038c32df9a0d24532ee40d2f14c)) 

  This allows client applications to verify details of the certificate,
  possibly with the user, when connecting to a server using TLS.

- Add clipboard data locking methods ([#1064](https://github.com/Devolutions/IronRDP/issues/1064)) ([58c3df84bb](https://github.com/Devolutions/IronRDP/commit/58c3df84bb9cafc8669315834cead35a71483c34)) 

  Per [MS-RDPECLIP sections 2.2.4.6 and 2.2.4.7][lock-spec], the Local
  Clipboard
  Owner may lock the Shared Clipboard Owner's clipboard data before
  requesting
  file contents to ensure data stability during multi-request transfers.
  
  This enables server implementations to safely request file data from
  clients
  when handling clipboard paste operations.
  
  ---------

- Add request_file_contents method ([#1065](https://github.com/Devolutions/IronRDP/issues/1065)) ([c30fc35a28](https://github.com/Devolutions/IronRDP/commit/c30fc35a28d6218603c1662e98e8b3053bea3aa5)) 

  Per [MS-RDPECLIP section 2.2.5.3][file-contents-spec], the Local
  Clipboard Owner
  sends File Contents Request PDU to retrieve file data from the Shared
  Clipboard
  Owner during paste operations.
  
  This enables server implementations to request file contents from
  clients,
  completing the bidirectional file transfer capability.

- Add SendFileContentsResponse message variant ([#1066](https://github.com/Devolutions/IronRDP/issues/1066)) ([25f81337aa](https://github.com/Devolutions/IronRDP/commit/25f81337aa494af9a21f55f12ec27fd946465cbe)) 

  Adds `SendFileContentsResponse` to `ClipboardMessage` enum, enabling
  clipboard
  backends to signal when file data is ready to send via
  `submit_file_contents()`.
  
  This provides the message-based interface pattern used consistently by
  server
  implementations for clipboard operations.

- Add bulk compression and wire negotiation ([ebf5da5f33](https://github.com/Devolutions/IronRDP/commit/ebf5da5f3380a3355f6c95814d669f8190425ded)) 

  - add ironrdp-bulk crate with MPPC/NCRUSH/XCRUSH, bitstream, benches, and metrics
  - advertise compression in Client Info and plumb compression_type through connector
  - decode compressed FastPath/ShareData updates using BulkCompressor
  - update CLI to numeric compression flags (enabled by default, level 0-3)
  - extend screenshot example with compression options and negotiated logging
  - refresh tests, FFI/web configs, typos, and Cargo.lock

- Advertise multitransport channel in GCC blocks ([#1092](https://github.com/Devolutions/IronRDP/issues/1092)) ([4f5fdd3628](https://github.com/Devolutions/IronRDP/commit/4f5fdd3628f4d0d2c2a4116e4e45269d802740f1)) 

  Add multitransport_flags config option to populate the
  MultiTransportChannelData GCC block during connection negotiation.
  When None (the default), behavior is unchanged.

- Implement ECHO virtual channel ([#1109](https://github.com/Devolutions/IronRDP/issues/1109)) ([6f6496ad29](https://github.com/Devolutions/IronRDP/commit/6f6496ad29395099563d50417d6dfff623914ee6)) 

- Handle Auto-Detect Request PDUs from server ([#1178](https://github.com/Devolutions/IronRDP/issues/1178)) ([4dcad09980](https://github.com/Devolutions/IronRDP/commit/4dcad09980e4f5354e4e435a134cc0956e2fcf9e)) 

  Fixes a crash when the server sends Auto-Detect Request PDUs during an
  active session. After #1176 added ShareDataPdu::AutoDetectReq routing,
  these PDUs decode correctly but hit the catch-all error path in the x224
  processor: "unhandled PDU: Auto-Detect Request PDU".

- Add --scale-desktop CLI option for display scaling factor ([#1200](https://github.com/Devolutions/IronRDP/issues/1200)) ([e5ab22246f](https://github.com/Devolutions/IronRDP/commit/e5ab22246ff3c01cddc66e49fd86b68c6f09265a)) 

  Adds a new CLI flag to control the initial RDP “desktop scale factor” sent during connection setup, enabling display scaling (useful for HiDPI/4K setups) similarly to FreeRDP’s scaling option.

- Implement clipboard file transfer support ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9)) 

  Add end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel per MS-RDPECLIP.

- Improved RDP file support ([#1220](https://github.com/Devolutions/IronRDP/issues/1220)) ([d0e18e3693](https://github.com/Devolutions/IronRDP/commit/d0e18e3693f313e83c769fb37fa991965d2a39d3)) 

- Add --prevent-session-lock CLI option ([#1207](https://github.com/Devolutions/IronRDP/issues/1207)) ([2d4980463e](https://github.com/Devolutions/IronRDP/commit/2d4980463e280dd880c5fd850eb35daf220db88a)) 

  Adds a new CLI flag to prevent remote session locking by injecting fake
  mouse movement events when the connection is idle, similarly to
  FreeRDP's equivalent option.

- Add --desktop-width and --desktop-height CLI options ([#1307](https://github.com/Devolutions/IronRDP/issues/1307)) ([879ffed866](https://github.com/Devolutions/IronRDP/commit/879ffed866c32748d30d26b54f9d667ad001c51c)) 

  Allow specifying the desired desktop resolution for the RDP session
  directly from the command line, mapping to the `desktopwidth` and
  `desktopheight` property set entries.

- Split ironrdp-client into a reusable library + ironrdp-viewer binary ([#1309](https://github.com/Devolutions/IronRDP/issues/1309)) ([361bdc2fe8](https://github.com/Devolutions/IronRDP/commit/361bdc2fe87739b7ffb8a2eb1705d5014c9f209b)) 

- Gate native backends behind Cargo features ([#1338](https://github.com/Devolutions/IronRDP/issues/1338)) ([f7e6106e0f](https://github.com/Devolutions/IronRDP/commit/f7e6106e0f293c1e0f8129be82aa2d86737ba92a)) 

  ironrdp (meta crate):
  - Added:    client, client-all, client-sound, client-clipboard,
              client-rdpdr, client-smartcard, client-gateway,
              client-dvc-pipe-proxy, client-dvc-com-plugin, and
              top-level rustls / native-tls (forwarded to ironrdp-client)
  - Modified: qoi, qoiz now also gate ironrdp-client's codec

- Dispatch initiate_file_copy via ClipboardMessage ([#1388](https://github.com/Devolutions/IronRDP/issues/1388)) ([b6325f9ea6](https://github.com/Devolutions/IronRDP/commit/b6325f9ea6900a84643b4415f9ebc7b1010cf3cd)) 

  Extends the CLIPRDR backend-facing API to properly support offering clipboard file lists (so later FileContentsRequests can be serviced) by introducing ClipboardMessage::SendInitiateFileCopy(Vec<FileDescriptor>) and wiring it through the in-tree ClipboardMessage dispatchers.

- Introduce ironrdp-agent crate ([#1339](https://github.com/Devolutions/IronRDP/issues/1339)) ([2046639870](https://github.com/Devolutions/IronRDP/commit/2046639870530458479cdacec0b2eb056ee10edf)) 

  Add a CLI-driven, daemon-backed RDP client designed for programmatic
  (e.g. LLM) consumption. A single binary plays two roles: a long-lived
  daemon that owns the ironrdp-client engine and one RDP session, and a
  short-lived CLI that drives it over a local IPC transport (Unix domain
  socket / Windows named pipe).

### <!-- 4 -->Bug Fixes

- Use desktop size for RFX channel size ([#756](https://github.com/Devolutions/IronRDP/issues/756)) ([806f1d7694](https://github.com/Devolutions/IronRDP/commit/806f1d7694313b1a59842af300a437ae2f6c2463)) 

- Inject socket local address for the client addr ([#759](https://github.com/Devolutions/IronRDP/issues/759)) ([712da42ded](https://github.com/Devolutions/IronRDP/commit/712da42dedc193239e457d8270d33cc70bd6a4b9)) 

  We used to inject the resolved target server address, but that is not
  what is expected. Server typically ignores this field so this was not a
  problem up until now.

- Fix special key modifiers on linux ([#906](https://github.com/Devolutions/IronRDP/issues/906)) ([0f9e8b1017](https://github.com/Devolutions/IronRDP/commit/0f9e8b1017421c5195d2c23aeebd3ac667238baf)) 

  This makes CTRL+C, CTRL+V, etc. work.

- Rename option no_server_pointer into enable_server_pointer ([218fed03c7](https://github.com/Devolutions/IronRDP/commit/218fed03c7993af0f958453e3944c58bcf9f43cb)) 

- Rename option no_audio_playback into enable_audio_playback ([5d8a487001](https://github.com/Devolutions/IronRDP/commit/5d8a487001c1280cbaf9f581f2a9a2f47d187bf0)) 

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

- Propagate negotiated share_id to all outgoing ShareDataPdu ([#1147](https://github.com/Devolutions/IronRDP/issues/1147)) ([2b24e9664d](https://github.com/Devolutions/IronRDP/commit/2b24e9664dd05620ff63a24d092377477fdde863)) 

- Correct binding and error macro in SendInitiateFileCopy arm ([#1390](https://github.com/Devolutions/IronRDP/issues/1390)) ([b407c6fab4](https://github.com/Devolutions/IronRDP/commit/b407c6fab48a657a828f2c324e0855968f96dd75)) 

  The `ClipboardMessage::SendInitiateFileCopy` arm in the clipboard event
  handler refers to a `cliprdr` binding and a `session::custom_err!` macro
  that are not in scope. The surrounding handler binds the processor as
  `cliprdr_client`, and every other arm in the same match uses
  `ironrdp_session::custom_err!`. With the `clipboard` feature enabled
  this fails to compile:
  
  ```
  error[E0425]: cannot find value `cliprdr` in this scope
  error[E0433]: failed to resolve: use of unresolved module or unlinked crate `session`
  ```
  
  so master does not build with the clipboard feature on. This aligns the
  arm with its siblings.
  
  Verified with `cargo check -p ironrdp-client --features
  rustls,clipboard`.

- [**breaking**] Remove ironrdp-connector dependency ([#1435](https://github.com/Devolutions/IronRDP/issues/1435)) ([c6a0286dcb](https://github.com/Devolutions/IronRDP/commit/c6a0286dcb49d9ac54c65c4f9325b41e05d541b8)) 

  Removes the last ironrdp-connector coupling from ironrdp-session by
  turning Deactivate-All handling into a bare signal and shifting ownership
  of the Deactivation-Reactivation activation sequence back to each consumer.
  It introduces a ConnectionActivationFactory (fresh sequence per reactivation)
  and an ActiveStageBuilder so session construction no longer depends on
  ConnectionResult.

### <!-- 7 -->Build

- Bump tokio-tungstenite from 0.26.2 to 0.27.0 ([#848](https://github.com/Devolutions/IronRDP/issues/848)) ([14e245d73e](https://github.com/Devolutions/IronRDP/commit/14e245d73e0b9a3e15a28fc3d1a27cc8f0a728b5)) 

- Bump uuid from 1.17.0 to 1.18.0 ([#915](https://github.com/Devolutions/IronRDP/issues/915)) ([a682d9cc48](https://github.com/Devolutions/IronRDP/commit/a682d9cc483690f91ca3c85e798b935386aa430e)) 

- Bump inquire from 0.7.5 to 0.8.0 ([#984](https://github.com/Devolutions/IronRDP/issues/984)) ([0b39078c26](https://github.com/Devolutions/IronRDP/commit/0b39078c2619030c3cfff10f6838cd8283c85d7e)) 

- Bump tokio-tungstenite from 0.27.0 to 0.28.0 ([#1009](https://github.com/Devolutions/IronRDP/issues/1009)) ([d24dbb1e2c](https://github.com/Devolutions/IronRDP/commit/d24dbb1e2c580c2ca66c7d7a0cc3ce468e8e78a7)) 

- Bump windows from 0.61.3 to 0.62.1 ([#1010](https://github.com/Devolutions/IronRDP/issues/1010)) ([79e71c4f90](https://github.com/Devolutions/IronRDP/commit/79e71c4f90ea68b14fe45241c1cf3953027b22a2)) 

- Bump uuid from 1.18.1 to 1.19.0 ([#1056](https://github.com/Devolutions/IronRDP/issues/1056)) ([113284a053](https://github.com/Devolutions/IronRDP/commit/113284a0539a6b1e232a7957d877a3593d82252a)) 

- Bump whoami from 1.6.1 to 2.0.2 ([#1081](https://github.com/Devolutions/IronRDP/issues/1081)) ([b099279424](https://github.com/Devolutions/IronRDP/commit/b0992794249e9d384c9319f1e270c280fac585f8)) 

- Bump uuid from 1.19.0 to 1.20.0 ([#1087](https://github.com/Devolutions/IronRDP/issues/1087)) ([50168c8693](https://github.com/Devolutions/IronRDP/commit/50168c8693b1e96409adee2378818f90a6e3808d)) 

- Bump uuid from 1.20.0 to 1.21.0 ([#1119](https://github.com/Devolutions/IronRDP/issues/1119)) ([e406559e0d](https://github.com/Devolutions/IronRDP/commit/e406559e0d2743dbfc4105120baabb303584a699)) 

- Upgrade sspi to 0.19, picky to rc.22, fix NTLM fallback ([#1188](https://github.com/Devolutions/IronRDP/issues/1188)) ([c70d38a9f1](https://github.com/Devolutions/IronRDP/commit/c70d38a9f190d6ad6c84bd9027a388b5db3296ba)) 

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 

- Bump the patch group across 1 directory with 3 updates ([#1233](https://github.com/Devolutions/IronRDP/issues/1233)) ([4282fb22f0](https://github.com/Devolutions/IronRDP/commit/4282fb22f0bf34c76ab665f7a6435bc0eb9f7e44)) 

- Bump tokio-tungstenite from 0.28.0 to 0.29.0 ([#1234](https://github.com/Devolutions/IronRDP/issues/1234)) ([f6fa5779a1](https://github.com/Devolutions/IronRDP/commit/f6fa5779a1cc131ce37b83d7457b8e454e695b31)) 

### Refactor

- [**breaking**] Add supported codecs in BitmapConfig ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 

  "session" has a fixed set of supported codecs with associated IDs.
  
  "connector" must expose the set of codecs during capabilities exchange.
  It currently uses hard-codes codec IDs in different places.
  
  Move the BitmapCodecs set to ironrdp-pdu. Shared code will be used by
  the server, so this is a suitable common place.

- [**breaking**] Enable `unwrap_used` clippy correctness lint ([#965](https://github.com/Devolutions/IronRDP/issues/965)) ([630525deae](https://github.com/Devolutions/IronRDP/commit/630525deae92f39bfed53248ab0fec0e71249322)) 


