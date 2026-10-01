# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.8.0...ironrdp-dvc-v0.9.0)] - 2026-10-01

### <!-- 0 -->Security

- Wire UDP multitransport into ironrdp-server ([#1954](https://github.com/Devolutions/IronRDP/issues/1954)) ([73dfa30e38](https://github.com/Devolutions/IronRDP/commit/73dfa30e3831e51cf8aa58f93e27b1b00102a7ec)) 

  - Add `RdpServerBuilder::with_udp_transport(udp_bind_addr)`: opt-in,
  `None` by default, no behavior change unless called.
  - When set (and the security mode is `Tls` or `Hybrid`, matching the
  reference client's Enhanced-Security-only gate), the acceptor offers UDP
  multitransport, and `accept_finalize` uses
  `accept_finalize_with_multitransport` with a callback that binds a fresh
  UDP socket per connection, reuses the connection's own TLS certificate
  (`TlsAcceptor::config()`) for the sideband transport, and calls
  `accept_udp()`.
  - Once established, the transport is used to migrate EGFX graphics
  traffic off TCP: `request_reliable_udp` is called opportunistically the
  first time EGFX has data to send (its dynamic channel id is only known
  once the client opens it). From the request on, every server message on
  that channel goes over the tunnel, starting with the batch that
  triggered it: the request carries SOFT_SYNC_TCP_FLUSHED and the server
  MUST keep using the named tunnel immediately after sending it
  (MS-RDPEDYC 2.2.5.1, 3.3.5.3.1). DRDYNVC replies for a tunneled channel
  go over the tunnel too, whichever path produced them. A new
  `client_loop` select arm feeds incoming tunnel payloads into
  `DrdynvcServer::process_tunnel()`; payloads that arrive before the
  client's Soft-Sync Response are held and processed once it does
  (3.3.5.3.2), rather than dropped.
  - The UDP accept runs as an ordinary `tokio::spawn` task, so enabling
  UDP adds no runtime requirement for the caller.
  - A failure before EGFX has moved (bind, handshake, TLS, or the tunnel
  closing) leaves the session on TCP, matching the reference client's
  posture. Once EGFX is on the tunnel, the tunnel closing ends the
  connection: Soft-Sync cannot move a channel back to TCP (MS-RDPEDYC
  2.2.5.1), and the tunnel lasts as long as the connection (MS-RDPEMT
  1.3.3).
  - A client that answers the Initiate Multitransport Request with E_ABORT
  (MS-RDPBCGR 2.2.15.2) has given up on the sideband transport, so the
  pending UDP accept is stopped as soon as that response arrives, whether
  during finalization or later on the message channel, instead of holding
  its socket until the 15 s accept timeout. Windows clients send it about
  2.7 s after connecting.
  - When the UDP bind address has an unspecified IP, each connection's
  socket binds to the local address that client reached over TCP instead.
  A socket bound to the unspecified address replies from whichever address
  the routing table picks, and on a host with several IPv6 addresses that
  is not always the one the client sent to: mstsc dropped the replies and
  gave up with E_ABORT. `run()` records the address itself; embedders
  driving `run_connection_with` pass it with the new
  `RdpServer::set_connection_local_addr`.
  - A successful Initiate Multitransport Response that arrives after
  finalization now enables EGFX migration for the rest of the session,
  provided Soft-Sync was negotiated. mstsc finishes its UDP bootstrap
  after the TCP finalization (0.87 s later in my test), so migration was
  previously decided before its response existed and the session never
  left TCP even with the sideband transport up.
  - The EGFX channel is found whichever way it was registered: an embedder
  that takes a frame handle from its `GfxServerFactory` registers it as
  `GfxDvcBridge`, which the migration lookup did not recognise, so it
  never sent the Soft-Sync Request. The Soft-Sync Request, the client's
  response (with the tunnels and channels it accepted) and the switch of
  EGFX onto UDP are now logged at debug level.

### <!-- 1 -->Features

- Expose generic session configuration and lifecycle APIs ([#1522](https://github.com/Devolutions/IronRDP/issues/1522)) ([57b1366650](https://github.com/Devolutions/IronRDP/commit/57b13666506dc40c15b4c4702d35150beee99133)) 

  ## Summary
  - expose generic client configuration for connection metadata,
  compression, shell/work directory, audio, and runtime static-channel
  factories
  - add bounded input delivery with independent close cancellation, host
  clipboard plumbing, lifecycle events, and Display Control resize
  readiness/fallback handling
  - update agent, viewer, web, FFI, examples, and tests for the generic
  APIs
  
  ## Stack dependencies
  This PR is stacked on `copilot/tls-validation-policy` (`b2bbcece`),
  which already includes the merged runtime static-channel support from
  `master`. It intentionally contains no TLS implementation/policy,
  ActiveX/COM, SVC implementation, decompression, or bitmap-recovery
  changes.
  
  ## Validation
  - `cargo fmt --check --all`
  - `cargo xtask check tests --no-run -v`
  - `cargo xtask check lints -v`
  - `cargo test -p ironrdp-client --lib --features rustls`
  - `cargo check -p ironrdp-agent -p ironrdp-viewer -p ironrdp-web -p ffi`
  
  ---------

- [**breaking**] Add Soft-Sync PDU support ([#1584](https://github.com/Devolutions/IronRDP/issues/1584)) ([bd630842ba](https://github.com/Devolutions/IronRDP/commit/bd630842bacc49cc129e613610482023a0f760db)) 

  Add Soft-Sync codecs and DVC dispatch for tunnel assignments.
  
  Keep decoding forward compatible and bound peer-controlled allocations.
  Reject exchanges before their required multitransport endpoint is ready.

- Create channels with assigned IDs ([#1416](https://github.com/Devolutions/IronRDP/issues/1416)) ([41293c2442](https://github.com/Devolutions/IronRDP/commit/41293c2442dfb2da6b61ca05a0c842706d048fc1)) 

  Reserve the channel ID before constructing its processor so the processor
  and its dependencies can use the ID during initialization.
  
  Add a fallible builder API that preserves construction errors.

- Attach recorded dynamic channels ([#1664](https://github.com/Devolutions/IronRDP/issues/1664)) ([93780feeec](https://github.com/Devolutions/IronRDP/commit/93780feeec1f09e13c8dd4691d5c5da20fae9310)) 

  Attach known channel IDs for offline replay.
  
  Reject duplicate IDs and failed startup atomically.

- [**breaking**] Populate decode/encode error offsets from cursor positions ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec)) 

  ## Summary
  
  The workspace sweep that follows #1266. Decode and encode error
  construction sites now pass the cursor, so the reported position is the
  byte the decoder or encoder actually stopped at.
  
  Stacked on #1266 and merges after it.
  
  ## What "no position" means here
  
  #1266 makes `offset` an `Option<usize>` where `None` means the error has
  no position in the input stream at all, rather than a position that
  happened to be unavailable. This PR is the other half of that: it walks
  the workspace and gives a real position to every site that has one, so
  the sites left reporting `None` are the ones that genuinely never had
  one.
  
  Those are constructors validating their arguments, integer conversions,
  cache lookups that missed, accessors on already-decoded structures, and
  the declared-size checks described below. They report nothing rather
  than byte zero, and that is now their permanent answer rather than a gap
  awaiting another sweep.
  
  There are no `at: 0` sites left anywhere in the workspace.
  
  ## The rule
  
  The position is attached where the cursor identifies the bytes being
  complained about. It is omitted where the complaint is about a size the
  peer declared, computed from data already consumed, because there the
  cursor points at a byte that is not the problem.

- Add location redirection ([#1778](https://github.com/Devolutions/IronRDP/issues/1778)) ([1cee7a8613](https://github.com/Devolutions/IronRDP/commit/1cee7a86135a0556c01965d0406233bd7df367a9)) 

  Implement MS-RDPEL v1 codecs and the location DVC state machine, then
  route the ActiveX methods through the bounded client input queue.
  
  Preserve mstsc-compatible validation and altitude caching while
  surfacing inactive sessions, channel readiness, queue pressure, and
  encoding failures. Coordinates are caller-supplied only and are never
  logged or persisted.

- Route channels over Soft-Sync tunnels ([#1826](https://github.com/Devolutions/IronRDP/issues/1826)) ([a989229409](https://github.com/Devolutions/IronRDP/commit/a9892294095d32808ee24dc8dad74dce46cb0bb4)) 

  Add DvcMessageBatch to carry a dynamic channel ID alongside its
  encoded SVC messages, and validate that a Soft-Sync-selected tunnel
  matches the channel before forwarding tunneled DRDYNVC data.

### <!-- 4 -->Bug Fixes

- [**breaking**] Replace DVC wrappers with typed accessors ([#1377](https://github.com/Devolutions/IronRDP/issues/1377)) ([d43ecf9a54](https://github.com/Devolutions/IronRDP/commit/d43ecf9a54363d37e0c485a1e9e73da0d47ae540)) 

  Follow-up to #1368. This is not urgent; review whenever the DVC API
  direction is worth revisiting.
  
  Rework DVC channel access APIs so callers can recover a typed processor
  together with its dynamic channel id, without exposing internal channel
  wrapper types.
  
  - Add typed borrowed DVC accessors carrying both channel id and
  processor borrow for `DrdynvcClient`.
  - Keep dynamic channel wrapper types private.
  - Align client listener/registration APIs on `DvcClientProcessor`.

- Log channel name on DVC creation failure ([#1765](https://github.com/Devolutions/IronRDP/issues/1765)) ([0a4147e5b1](https://github.com/Devolutions/IronRDP/commit/0a4147e5b1e2d4bd35b8cb9c401d61fb06960396)) 

  ## Summary
  
  - When a client rejects a DVC create request, `DrdynvcServer::process()`
  only logged the raw PDU (channel_id and status code). The channel's name
  is already in scope one line earlier, from building the original create
  request, but the failure branch never surfaces it.
  - I found this while debugging a real server log: a channel_id 0
  creation failure with no way to tell which channel it was without
  reading the source and tracing registration order by hand.
  - Added the channel name to the failure log line using structured
  tracing fields, and raised the level from implicit debug to warn, since
  a channel creation failure is a real, actionable event most deployments
  would want visible at default verbosity rather than something requiring
  debug or trace logging to notice.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  Five-line diff, `crates/ironrdp-dvc/src/server.rs` only. No behavior
  change beyond the added log line.

- Switch a Soft-Sync tunnel that also lists declined channels ([#2007](https://github.com/Devolutions/IronRDP/issues/2007)) ([3b3a17b958](https://github.com/Devolutions/IronRDP/commit/3b3a17b958e1edae1a00cd3a3a97b0103d932792)) 

  Windows lists every dynamic channel it intends to move in its Soft-Sync
  request, including the ones the client declined with NO_LISTENER.
  Against a Windows 11 host the request lists channels 2, 6, 7, 8, 9, 10,
  11 and 12 (CoreInput, MouseCursor, Graphics, Video, Geometry, ...), and
  only channel 7, the graphics pipeline, is open.
  
  `process_soft_sync_request` dropped a whole channel list as soon as one
  ID in it was not open. The tunnel was then never switched, and the
  channels the client had opened stayed on TCP while the server was
  already sending them on the tunnel (MS-RDPEDYC 3.2.5.3.1).
  
  Unopened channels are now skipped one by one, and the tunnel is switched
  for the rest.
  
  ## Testing
  
  - New `dvc::client::soft_sync_skips_channels_the_client_did_not_open` in
  `ironrdp-testsuite-core`.
  - Live, against a Windows 11 host over RDP-UDP version 2, with the
  viewer built from a branch that also carries the tunnel and client PRs
  of this series: the Soft-Sync request above now switches the tunnel, and
  the graphics pipeline moves onto it.
  
  ## Checks
  
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --features helper,__bench
  --locked -- -D warnings`
  - `cargo test --locked -p ironrdp-testsuite-core -p
  ironrdp-testsuite-extra`, plus the lib tests of the crates touched here
  - `cargo test --workspace --locked` on a branch that merges this PR with
  the other Windows interop PRs from this series
  - `typos` on the changed files
  
  ## Series
  
  These PRs port the Windows interop fixes and Linux backends from a
  downstream IronRDP fork, so the fork can be retired. Each one is based
  on `master` and can be reviewed and merged on its own. I also checked
  that all of them merge cleanly together in this order.

- [**breaking**] Serve channels and graphics that Windows moves onto a tunnel ([#2008](https://github.com/Devolutions/IronRDP/issues/2008)) ([bfd16a2e55](https://github.com/Devolutions/IronRDP/commit/bfd16a2e5573d4cd6617dd71b2802b4e7b944a59)) 

  Once Soft-Sync has moved dynamic channels onto the reliable UDP tunnel,
  Windows keeps using the tunnel for more than channel data. Two gaps kept
  the graphics pipeline from working there.

- Log a declined channel at debug ([#1988](https://github.com/Devolutions/IronRDP/issues/1988)) ([bf64e4e5cf](https://github.com/Devolutions/IronRDP/commit/bf64e4e5cf15a719705d25dc7121b2f93631d10d)) 



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.7.0...ironrdp-dvc-v0.8.0)] - 2026-07-10

### <!-- 1 -->Features

- Expose dynamic channel accessors ([#1368](https://github.com/Devolutions/IronRDP/issues/1368)) ([985d353543](https://github.com/Devolutions/IronRDP/commit/985d353543cf45eacfe0cc57aca86502665a3a44)) 

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.6.0...ironrdp-dvc-v0.7.0)] - 2026-06-05

### <!-- 4 -->Bug Fixes

- [**breaking**] Add channel_id parameter to DvcChannelListener::create ([#1358](https://github.com/Devolutions/IronRDP/issues/1358)) ([f21470c6dc](https://github.com/Devolutions/IronRDP/commit/f21470c6dc20e1b10b4bbf750a406644479a4b35)) 

  Updates the dynamic virtual channel (DVC) client listener interface in ironrdp-dvc to pass the channel_id (from the incoming DYNVC_CREATE_REQ) into the listener’s create method, enabling listeners to differentiate/control per-instance behavior based on the negotiated dynamic channel ID.



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.5.0...ironrdp-dvc-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- Implement ECHO virtual channel ([#1109](https://github.com/Devolutions/IronRDP/issues/1109)) ([6f6496ad29](https://github.com/Devolutions/IronRDP/commit/6f6496ad29395099563d50417d6dfff623914ee6)) 

- Add DvcChannelListener for multi-instance DVC support ([#1142](https://github.com/Devolutions/IronRDP/issues/1142)) ([28e8628f0e](https://github.com/Devolutions/IronRDP/commit/28e8628f0e3cea9f7723a73abf5fd7ed2da968f0)) 

- Close channel API for server and client ([#1302](https://github.com/Devolutions/IronRDP/issues/1302)) ([196d18dfaa](https://github.com/Devolutions/IronRDP/commit/196d18dfaa7ec899946bb90f4dcb8bad31872f48)) 

### <!-- 4 -->Bug Fixes

- Negotiate DVC version from server capabilities ([d094cbeb75](https://github.com/Devolutions/IronRDP/commit/d094cbeb7501c83fc6ad5401ba69d22f79d6657c)) 

  The client was hardcoded to respond with CapsVersion::V1 regardless
  of what the server requested. Servers that require V2 or V3 (such
  as XRDP) would reject the channel with "Dynamic Virtual Channel
  version 1 is not supported."
  
  Echo the server's requested version in the capabilities response
  instead. This correctly handles V1, V2, and V3 depending on what
  the server advertises. When a Create arrives before Capabilities
  (fallback path), default to V2 as the most broadly compatible
  version.
  
  Also bump the server-side capabilities request from V1 to V2 to
  advertise priority charge support.
  
  Add CapabilitiesRequestPdu::version() accessor to expose the
  server's requested version from the parsed PDU.

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.4.0...ironrdp-dvc-v0.4.1)] - 2025-09-04

### <!-- 1 -->Features

- Add API to attach dynamic channels to an already created `DrdynvcClient` instance (#938) ([17833fe009](https://github.com/Devolutions/IronRDP/commit/17833fe009279823c4076d3e2e0c7d063fd24a43)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.3.0...ironrdp-dvc-v0.3.1)] - 2025-06-27

### <!-- 1 -->Features

- Add `DynamicChannelSet::get_by_channel_id` (#791) ([5482365655](https://github.com/Devolutions/IronRDP/commit/5482365655e5c171cd967eda401b01161a9f6602)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.3...ironrdp-dvc-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.2...ironrdp-dvc-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.1...ironrdp-dvc-v0.1.2)] - 2025-01-28

### <!-- 1 -->Features

- Some debug statement on invalid channel state ([265b661b81](https://github.com/Devolutions/IronRDP/commit/265b661b81af19860c4564ba35ad22564f61cd02)) 

- Add CreationStatus::NOT_FOUND ([ab8a87d942](https://github.com/Devolutions/IronRDP/commit/ab8a87d94259a4e1df5f3a2a8d4c592377857b21)) 

  For completeness, this error is used by FreeRDP.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.0...ironrdp-dvc-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
