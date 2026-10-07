# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.10.0...ironrdp-acceptor-v0.11.0)] - 2026-10-07

### <!-- 0 -->Security

- Validate auto-reconnect cookies ([#1509](https://github.com/Devolutions/IronRDP/issues/1509)) ([44f675e244](https://github.com/Devolutions/IronRDP/commit/44f675e244ee76b5311756668ffbbe28e98c7175)) 

  ## Summary
  - parse and carry `ARC_CS_PRIVATE_PACKET` data through the acceptor
  - validate returning Enhanced RDP Security cookies with HMAC-MD5 before
  reconnecting
  - rotate reconnect randoms per connection and hourly, with runtime
  cookie updates
  - restrict cookie authentication to TLS/Hybrid and document the behavior
  
  ## Testing
  - `cargo test -p ironrdp-pdu -p ironrdp-acceptor -p ironrdp-server`
  - `cargo clippy -p ironrdp-pdu -p ironrdp-acceptor -p ironrdp-server
  --all-targets -- -D warnings`

- Send periodic Heartbeat PDUs on the message channel ([#1842](https://github.com/Devolutions/IronRDP/issues/1842)) ([8f57691a9a](https://github.com/Devolutions/IronRDP/commit/8f57691a9a6e497388dc2824bffe93eeed7bb698)) 

  The server never sends Server Heartbeat PDUs (MS-RDPBCGR 2.2.16.1), even
  though the client half of the feature is in place: HeartbeatPdu
  encode/decode landed with #1814 and clients decode and ignore them. This
  adds the send half, opt-in.

- Add server-side UDP multitransport bootstrapping ([#1951](https://github.com/Devolutions/IronRDP/issues/1951)) ([c36883027d](https://github.com/Devolutions/IronRDP/commit/c36883027de1975d22c0dbc7943a17b3f73bc5d0)) 

  ## Summary
  
  - Add a `MultitransportBootstrapping` state to the acceptor sequence,
  entered right after licensing.
  - Advertise UDP multitransport support in the GCC Server
  MultiTransportChannelData block via a new `set_multitransport_offer()`
  config (previously always `None`, so a server could never enable it),
  gated on the client having populated its own Client
  MultiTransportChannelData block (MS-RDPBCGR 2.2.1.4).
  - When the client reciprocates the reliable-UDP flag, send the Initiate
  Multitransport Request (MS-RDPBCGR 2.2.15.1) on the MCS message channel,
  then move straight on to capability negotiation.
  - `multitransport_request()` surfaces the sent request so the caller can
  establish the sideband UDP transport (RDPEUDP2 + TLS + RDPEMT) in
  parallel, without the acceptor waiting for the client's response first.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  9 tests in `ironrdp-testsuite-core/tests/server/acceptor.rs` driving the
  full handshake through a real MCS channel join sequence:
  offered-and-reciprocated (request sent on the message channel, response
  tolerated before Confirm Active), disabled by default,
  client-does-not-reciprocate, a late response tolerated during
  ConnectionFinalization (not just before Confirm Active), non-response
  message-channel traffic (Auto-Detect Response, Heartbeat) not
  misclassified as a multitransport response, a response with trailing
  bytes not consumed, security cookie and request ID taken from the
  injected RNG, and an offer changed after Basic Settings Exchange
  (request and Soft-Sync) not affecting the negotiation already
  advertised.
  
  ## Notes
  
  The acceptor does not wait for the client's Initiate Multitransport
  Response before continuing: MS-RDPBCGR 3.2.5.15.1 only obliges the
  client to send one when Soft-Sync is negotiated or the sideband attempt
  failed, so blocking on it would stall the handshake on the plain
  successful path. A response can legitimately arrive either before the
  mandatory Confirm Active or after it, during ConnectionFinalization:
  both `CapabilitiesWaitConfirm` and `ConnectionFinalization` recognize it
  (by channel and a successful strict decode of the payload, so other
  message-channel traffic like Auto-Detect Response or Heartbeat correctly
  falls through instead) and drop it rather than erroring or desyncing the
  finalization sequence.
  
  Only reliable UDP (`TRANSPORT_TYPE_UDP_FECR`) is requested; lossy UDP is
  accepted in the offered flags for advertisement but never requested.
  Server-side wiring (`accept_finalize_with_multitransport` and
  `ironrdp-server` integration) is a follow-up.
  
  ## One PR is stacked on this
  
  #1953 (`accept_finalize_with_multitransport`, an async driver consuming
  this PR's API) is stacked on this branch. Its diff is filed against
  `master` and is cumulative with this one; see its own body for the
  incremental compare.

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

- Expose client multitransport flags on AcceptorResult ([#1453](https://github.com/Devolutions/IronRDP/issues/1453)) ([f0fc215555](https://github.com/Devolutions/IronRDP/commit/f0fc215555394a89510ff85c7b8a93b20e878074)) 

  ## What
  
  The acceptor already parses the client's GCC `MultiTransportChannelData`
  block (MS-RDPBCGR §2.2.1.3.8) into `ClientGccBlocks` during
  `BasicSettingsWaitInitial` and then discards it, keeping only the
  early-capability flags, core desktop size, and keyboard layout. This
  surfaces the client's multitransport (MS-RDPEMT) capability flags on
  `AcceptorResult`.
  
  ## Why
  
  A server implementing UDP multitransport needs to know whether the
  client advertised support (`SOFT_SYNC_TCP_TO_UDP`,
  `TRANSPORT_TYPE_UDP_FEC{R,L}`) before deciding whether to send a Server
  Initiate Multitransport Request. Today that information is parsed and
  thrown away, so there's no way for a downstream server to see it.
  
  ## Shape
  
  Purely additive, mirroring the existing `keyboard_layout` ([#1397](https://github.com/Devolutions/IronRDP/issues/1397)) and
  desktop-size ([#1373](https://github.com/Devolutions/IronRDP/issues/1373)) surfacing of GCC client data the acceptor already
  parses:
  
  - new private `multitransport_flags: gcc::MultiTransportFlags` field on
  `Acceptor`, captured from `gcc_blocks.multi_transport_channel`;
  - new `pub multitransport_flags: gcc::MultiTransportFlags` field on
  `AcceptorResult`;
  - empty when the client sends no multitransport block;
  - carried across a deactivation-reactivation like the sibling fields.
  
  No behavior change — the acceptor just stops discarding a block it
  already decodes.
  
  `cargo clippy -p ironrdp-acceptor --all-targets` and `cargo fmt --check`
  are clean.

- [**breaking**] Clamp honored client desktop size to an operator maximum ([#1404](https://github.com/Devolutions/IronRDP/issues/1404)) ([d3747a05b2](https://github.com/Devolutions/IronRDP/commit/d3747a05b202ba2d87ac19698354ae7e487850a2)) 

  Follow-up to #1373 (the resource-hardening angle you flagged in review —
  thanks for the go-ahead 🙂).
  
  ## Problem
  
  `#1373` gated honor-client-desktop-size behind a bare `bool`. With it
  on, the acceptor adopts the client-requested desktop size bounded only
  by the protocol range `[200, 8192]`. But the desktop size is a
  client-controlled `u16`, and the server still builds its
  framebuffer/encoder from the negotiated size — so a client could request
  e.g. `8192x8192` and drive the server's allocation off an untrusted
  number (~256 MiB per frame buffer). Mild, and only on an opt-in
  default-off path, but it's a resource-exhaustion vector driven purely by
  a number the client picks.
  
  Your review comment: *"[200, 8192] is a protocol ceiling, not a resource
  guard … tracked the 'clamp/range policy rather than a bare bool' idea as
  a future follow-up (an operator-set max size)."* This is that PR.
  
  ## Change
  
  Replace the `bool` with `Option<DesktopSize>` carrying an **operator-set
  maximum**:
  
  - `None` (default) — disabled; always enforce the server-provided size
  (unchanged behavior).
  - `Some(max)` — honor the client's request, **clamped per dimension to
  `max`**. The client can ask for a smaller desktop, never a larger one.
  
  The acceptor clamps the requested `width`/`height` to `max` *before* the
  existing `validate_desktop_size` protocol-range check, so the negotiated
  size can never exceed what the operator is willing to render — set `max`
  to the host display's native resolution (or whatever ceiling the server
  can afford).

- Support runtime-defined static virtual channels ([#1517](https://github.com/Devolutions/IronRDP/issues/1517)) ([8b4c483ba0](https://github.com/Devolutions/IronRDP/commit/8b4c483ba0c900a8de0b2718347754f56dd363ba)) 

  ## Summary
  - add keyed runtime-defined static-channel registration, lookup, and
  negotiated ID attachment
  - enforce the static-channel limit and reject malformed SVC fragment
  sequences
  - wire generic connector, acceptor, and session name-based dispatch
  support
  
  ## Testing
  - `cargo test -p ironrdp-testsuite-core --test integration_tests_core
  svc::`
  - `cargo clippy -p ironrdp-testsuite-core --test integration_tests_core
  -- -D warnings`
  
  ---------

- Negotiate monitor topology ([#1675](https://github.com/Devolutions/IronRDP/issues/1675)) ([063efcdc30](https://github.com/Devolutions/IronRDP/commit/063efcdc3088d8f44e423cc322077d40bf9aadf2)) 

  Negotiate the client monitor layout from UseMultimon and expose the
  confirmed remote topology through the ActiveX compatibility interface.
  
  Advertise Monitor Layout PDU support whenever Extended Client Data is
  negotiated, and forward layouts from activation, active sessions, and
  reactivation so advertised support does not terminate sessions.
  
  Keep fallback reporting truthful when servers do not honor the request,
  while preserving single-monitor resize behavior and blocking
  multi-monitor resizing.
  
  Do not send Client Monitor Extended Data; per-monitor DPI and
  orientation remain unavailable.

- [**breaking**] Pass frame arrival time into Sequence::step ([#1530](https://github.com/Devolutions/IronRDP/issues/1530)) ([6a499faece](https://github.com/Devolutions/IronRDP/commit/6a499faece8911e50a715a3fb08d4fd8e7d7dc87)) 

  ## Summary
  
  - Connect-time bandwidth measurement needs to know when bytes arrived,
  and nothing in the sans-I/O layer could tell it. #1465, now merged,
  answers the server's Bandwidth Measure Stop with a nominal interval for
  exactly that reason: the connector has no way to observe the real one.
  - Introduce `MonotonicInstant`, a millisecond counter with an arbitrary
  epoch, and make `Option<MonotonicInstant>` a required parameter of
  `Sequence::step`. The I/O drivers already know when a read completed, so
  `Framed` records the arrival time of each read and hands it to the state
  machine. A driver with no clock passes `None`.
  - With arrival times available, measure for real: a Bandwidth Measure
  Start opens a window, Payload messages accumulate their byte counts, and
  Stop reports the elapsed time between its own arrival and the Start's.
  
  #1465 has merged, so this applies directly to master and carries no
  merge-order dependency. That PR was the FreeRDP unblock on its own; this
  is the design change behind it, split out at @CBenoit's suggestion in
  review.
  
  ## Why the clock lives in the driver
  
  Two reasons, both of which rule out having the sequence read a clock
  itself.

- [**breaking**] Expose per-connection keyboard metadata via ConnectionHandler ([#1691](https://github.com/Devolutions/IronRDP/issues/1691)) ([393869b30b](https://github.com/Devolutions/IronRDP/commit/393869b30b1078da7204c6bf20e8a5472e419070)) 

  ## Summary
  
  AcceptorResult already carried keyboard_layout (the client's GCC Client
  Core Data keyboardLayout, MS-RDPBCGR 2.2.1.3.2), but ironrdp-server's
  client_accepted never read it, and the only extension point that could
  plausibly expose it, ConnectionHandler::on_accept/on_disconnected, only
  fires from RdpServer::run's own accept loop. An embedder with its own
  accept loop calling run_connection or run_connection_with directly never
  sees these hooks at all.
  
  Added keyboard_type and ime_file_name to Acceptor and AcceptorResult,
  captured from the same Client Core Data alongside keyboard_layout.
  
  Added a new ConnectionInfo struct and a default-no-op
  ConnectionHandler::on_connection_info(&ConnectionInfo) method, fired
  from client_accepted itself, right after credential and auto-reconnect
  validation succeed. This is reachable from every code path that
  completes connection setup, not only run's accept loop, so it is usable
  by embedders that never call run.
  
  Kept the hook synchronous. It only hands the embedder a small Clone-able
  struct; an embedder that needs to do blocking work in response can spawn
  its own task, the same way the existing on_accept/on_disconnected hooks
  already work.
  
  Open question: AcceptorResult is a public struct without non_exhaustive,
  so the two new fields are a real breaking change for any consumer
  destructuring it exhaustively, same class as the keyboardType change in
  #1689. AcceptorResult's attributes are unchanged here since marking it
  non_exhaustive is a broader decision than this PR's two fields.
  
  ## Validation
  
  cargo xtask check fmt/lints/tests/typos/locks all pass.
  
  ## Review round and rebase, 2026-08-19
  
  #1689 (KeyboardType) merged. This branch was still carrying a stale
  pre-merge copy of that commit, so the diff was cumulative against
  master. Rebased onto current master, which dropped the redundant
  duplicate commit (its content was already upstream) and left this PR's
  own single commit.
  
  Four review findings from the bot review, all addressed:
  
  - `on_connection_info` fired on every Deactivation-Reactivation resize,
  not just the initial connection, since `accept_finalize` loops back into
  `client_accepted` with `result.reactivation` set. Gated the call on
  `!result.reactivation`, matching the existing gate on the static-channel
  start block just below it.
  - `get_result()` took `ime_file_name` via `mem::take`, emptying it out
  of the acceptor before `new_deactivation_reactivation` copied the same
  acceptor's field into the next result, so every reactivation after the
  first reported an empty IME name. Changed to a clone, matching how the
  Copy-type sibling fields on the same lines already survive.
  - No regression test covered the permissive zero/unrecognized-value
  keyboardType decode in the Input capability set. Added
  `keyboard_type_zero_decodes_to_none` and
  `keyboard_type_unrecognized_value_round_trips` (0x51).
  - A fourth finding asked for the same coverage on Client Core Data's own
  keyboardType field; that test already exists on master, added to #1689
  in response to its own review. The rebase above inherits it directly, so
  no new code was needed there.

- Add accept_finalize_with_multitransport driver ([#1953](https://github.com/Devolutions/IronRDP/issues/1953)) ([22006ce1f9](https://github.com/Devolutions/IronRDP/commit/22006ce1f9dae8a052ae8b6131fb5f7f2fce663e)) 

  - Add `accept_finalize_with_multitransport`, an async driver mirroring
  the client side's `connect_finalize_with_multitransport`.
  - It drives the acceptor sequence to completion exactly like
  `accept_finalize` already does, and awaits an app-supplied handler once,
  synchronously, the moment the acceptor sends an Initiate Multitransport
  Request, so the caller can establish the sideband UDP transport
  (RDPEUDP2 + TLS + RDPEMT).
  - Unlike the client-side callback, the handler reports nothing back into
  the sequence: the acceptor has already sent the request and moved on by
  the time it runs.
  - `accept_finalize` becomes a thin wrapper around this with a no-op
  handler, matching the client side's
  `connect_finalize`/`connect_finalize_with_multitransport` relationship.



## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.9.0...ironrdp-acceptor-v0.10.0)] - 2026-07-10

### <!-- 1 -->Features

- Negotiate the MCS message channel ([#1347](https://github.com/Devolutions/IronRDP/issues/1347)) ([efa5732805](https://github.com/Devolutions/IronRDP/commit/efa573280572f3c0f0270a40ae51a154562706cc)) 

  Updates the handshake to properly negotiate the MCS message channel by advertising Extended Client Data Blocks support and, when requested by the client, allocating/joining the message channel and surfacing its ID in AcceptorResult. This enables server-initiated PDUs that must use the message channel (e.g., network auto-detect) to have a valid transport.

- Expose the client's keyboard layout on AcceptorResult ([#1397](https://github.com/Devolutions/IronRDP/issues/1397)) ([5ca84a5724](https://github.com/Devolutions/IronRDP/commit/5ca84a5724f48093193e39a3097c4f4987d64bbe)) 

- Honor the client-requested desktop size ([#1373](https://github.com/Devolutions/IronRDP/issues/1373)) ([d471bd066f](https://github.com/Devolutions/IronRDP/commit/d471bd066f303df22f4767801fd97ecdbf527869)) 

  Adds an opt-in server/acceptor knob to negotiate the RDP session desktop size using the client’s originally requested resolution (from GCC Client Core Data) so the server can start at the client’s native size without a Deactivation–Reactivation resize round trip.

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-async` public dependency to 0.10

- [**breaking**] Update `ironrdp-connector` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.8.0...ironrdp-acceptor-v0.9.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Send RDP_NEG_FAILURE on security protocol mismatch ([#1152](https://github.com/Devolutions/IronRDP/issues/1152)) ([02b9f4efbb](https://github.com/Devolutions/IronRDP/commit/02b9f4efbbe634a50efa0601f30e0a2096a6f78e)) 

  When the client and server have no common security protocol, the
  acceptor now sends a proper `RDP_NEG_FAILURE` PDU before returning an
  error, instead of dropping the TCP connection.

### <!-- 1 -->Features

- Expose received client credentials in AcceptorResult ([#1155](https://github.com/Devolutions/IronRDP/issues/1155)) ([eda32d8acf](https://github.com/Devolutions/IronRDP/commit/eda32d8acffbb2e37d13c790105ff022067f5efb)) 

- Skip credential check when server credentials are None ([#1150](https://github.com/Devolutions/IronRDP/issues/1150)) ([84015c9467](https://github.com/Devolutions/IronRDP/commit/84015c946731579dfd7a49294b2e55259e4f8d3f)) 

### <!-- 7 -->Build

- Upgrade sspi to 0.19, picky to rc.22, fix NTLM fallback ([#1188](https://github.com/Devolutions/IronRDP/issues/1188)) ([c70d38a9f1](https://github.com/Devolutions/IronRDP/commit/c70d38a9f190d6ad6c84bd9027a388b5db3296ba)) 


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.7.0...ironrdp-acceptor-v0.8.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.5.0...ironrdp-acceptor-v0.6.0)] - 2025-07-08

### <!-- 1 -->Features

- [**breaking**] Support for server-side Kerberos (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.4.0...ironrdp-acceptor-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Make the CredsspSequence type public ([5abd9ff8e0](https://github.com/Devolutions/IronRDP/commit/5abd9ff8e0da8ea48c6747526c4b703a39bf4972)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.3.1...ironrdp-acceptor-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.3.0...ironrdp-acceptor-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.2.1...ironrdp-acceptor-v0.3.0)] - 2025-01-28

### <!-- 0 -->Security

- Allow using basic RDP/no security ([7c72a9f9bb](https://github.com/Devolutions/IronRDP/commit/7c72a9f9bbe726d6f9f2377c19e9a672d8d086d5)) 

### <!-- 4 -->Bug Fixes

- Drop unexpected PDUs during deactivation-reactivation ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

  The current behavior of handling unmatched PDUs in fn read_by_hint()
  isn't good enough. An unexpected PDUs may be received and fail to be
  decoded during Acceptor::step().
  
  Change the code to simply drop unexpected PDUs (as opposed to attempting
  to replay the unmatched leftover, which isn't clearly needed)

- Reattach existing channels ([c4587b537c](https://github.com/Devolutions/IronRDP/commit/c4587b537c7c0a148e11bc365bc3df88e2c92312)) 

  I couldn't find any explicit behaviour described in the specification,
  but apparently, we must just keep the channel state as they were during
  reactivation. This fixes various state issues during client resize.

- Do not restart static channels on reactivation ([82c7c2f5b0](https://github.com/Devolutions/IronRDP/commit/82c7c2f5b08c44b1a4f6b04c13ad24d9e2ffa371)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.2.0...ironrdp-acceptor-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
