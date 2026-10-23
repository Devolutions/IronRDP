# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.12.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.11.0...ironrdp-session-v0.12.0)] - 2026-10-23

### <!-- 0 -->Security

- [**breaking**] Support session resume via the auto-reconnect cookie ([#1501](https://github.com/Devolutions/IronRDP/issues/1501)) ([74b3365c1f](https://github.com/Devolutions/IronRDP/commit/74b3365c1f98c0da6feed7507779c67e1b8e6d08)) 

  > **Rebased onto post-#1522 master.** #1509 landed the server half of
  #1508 while this was open, including the `ClientAutoReconnect`
  structure. This PR no longer declares it; it extends it, and picks up
  the parts #1509 did not build.
  
  ## What
  
  The client half of automatic reconnection. The session layer surfaces
  the Server Auto-Reconnect Cookie, `ironrdp-pdu` derives and verifies the
  client's response to it, and the connector sends that response when
  resuming a session.
  
  ## Why
  
  A client whose connection drops ungracefully can reattach to its session
  instead of making the user log on again, provided it returns the cookie
  the server issued during logon ([MS-RDPBCGR] 1.3.1.5).
  
  #1509 built the server side of that: it validates a returning
  `ARC_CS_PRIVATE_PACKET` and rotates the random. Nothing answers it.
  `ironrdp-session` decodes the cookie and drops it, `ironrdp-connector`
  has no way to send one back, and `TODO([#271](https://github.com/Devolutions/IronRDP/issues/271))` still sits in
  `ironrdp-client`. So `ironrdp-client` cannot resume a session against
  `ironrdp-server`, and the validation #1509 added has no in-tree
  counterpart to exercise it.
  
  The wire encoding was already there. `ExtendedClientOptionalInfo`
  carries, encodes and decodes a 28-byte `autoReconnectCookie` and its
  builder already had a `reconnect_cookie` step; `ServerAutoReconnect`
  already decoded; #1509 added `ClientAutoReconnect` and its decode.
  Nothing connected them.
  
  ## The three parts
  
  **Receive.** `SaveSessionInfo` now also surfaces the cookie, as
  `ProcessorOutput::AutoReconnectCookie` and
  `ActiveStageOutput::AutoReconnectCookie`. #1522 added a `SaveSessionInfo
  { logon_complete }` output on that same handler; the two coexist rather
  than compete, since both are read off one PDU and neither supersedes the
  other. The handler emits the logon notification unconditionally and
  appends the cookie when one is present, and a test pins that surfacing
  the cookie does not suppress the notification. #1509's server replaces
  the cookie whenever a client connects and again hourly ([MS-RDPBCGR]
  3.3.6.2), so this can arrive more than once in a session and the
  consumer keeps the most recent.
  
  **Derive.** `ClientAutoReconnect::from_server_cookie` implements
  [MS-RDPBCGR] 5.5:
  
  > The auto-reconnect random is used to key the HMAC function
  ([RFC2104]), which uses MD5 as the iterative hash function. The security
  verifier is derived by applying the HMAC to the client random received
  in Step 3.
  >
  > `SecurityVerifier = HMAC(AutoReconnectRandom, ClientRandom)`
  >
  > When Enhanced RDP Security is in effect the client random value is not
  generated (section 5.3.2). In this case, for the purpose of generating
  the security verifier, the client random is assumed to be an array of 32
  zero bytes.
  
  IronRDP implements no Standard RDP Security path (there is no Security
  Exchange PDU), so the zero-client-random case is the only one that
  arises. As 5.5 notes, that makes the verifier constant for a given
  cookie, so it proves possession of the cookie and nothing more; session
  security comes from the outer TLS/CredSSP handshake.
  
  @clintcan independently confirmed this construction against real
  **mstsc** while validating #1509
  ([comment](https://github.com/Devolutions/IronRDP/pull/1509#issuecomment-5151200681)):
  a Windows client's `ARC_CS_PRIVATE_PACKET` verifies against
  `HMAC-MD5(random_bits, [0u8; 32])`. That is the same derivation
  implemented here, so the two halves interoperate with Microsoft's client
  and not only with each other.
  
  **Send.** `ClientConnector::with_auto_reconnect_cookie` takes the cookie
  last received and makes the connector put the derived Client
  Auto-Reconnect Packet ([MS-RDPBCGR] 2.2.4.3) in the Client Info PDU.
  Absent, that PDU is byte-for-byte what it was.
  
  Unlike the server packet, this structure has no enclosing logon-info
  field header, so it encodes to exactly the 28 bytes the cookie field
  expects. `to_bytes` writes that layout directly rather than going
  through `Encode`, so filling a fixed-size field has no error path a
  caller must handle; a test pins the two to agree.
  
  ## One derivation, not two
  
  Putting `from_server_cookie` in `ironrdp-pdu` would leave the workspace
  with two implementations of 5.5, since #1509 added a private HMAC to
  `ironrdp-server`. So `ClientAutoReconnect` also gains `verify`, and the
  server routes through it.
  
  `verify` keeps the constant-time comparison the server had. The verifier
  is the whole credential, so a comparison returning early on the first
  differing byte would let a peer recover it a byte at a time from the
  timing; the session identifier is not secret and is compared normally.
  `ironrdp-server` keeps the policy around the check, which cookies are
  live and whether the security protocol permits auto-reconnect, and drops
  its `hmac` and `md-5` dependencies. `hmac` moves to `ironrdp-pdu` as
  `default-features = false`; the crate's full feature powerset still
  checks clean, including `--no-default-features`.
  
  I would rather not have reached into `ironrdp-server` in a
  `pdu,session,connector` change, but the alternative was shipping the
  duplicate and filing a follow-up to remove it, which is a worse trade
  for reviewer time.
  
  ## Tests that were not running
  
  That move also rehomes the known-answer tests @clintcan contributed on
  #1509. They went in as an inline `#[cfg(test)]` module in
  `crates/ironrdp-server/src/server.rs`, and that crate sets `[lib] test =
  false`, so they have never executed in CI. They now live in
  `ironrdp-testsuite-core` against the public API, where CI runs them: his
  HMAC-MD5 reference vector is kept as a second vector alongside a
  differently-keyed one, plus the cases for a tampered verifier and a
  mismatched logon ID.
  
  Worth flagging separately: `ironrdp-server` is not alone.
  `ironrdp-agent`, `ironrdp-session` and `ironrdp-web` also set `[lib]
  test = false` and between them carry 16 files of inline `#[cfg(test)]`
  modules that CI never runs. That is out of scope here, but I am happy to
  open an issue if it would be useful.
  
  ## Breaking changes
  
  `ActiveStageOutput` and `x224::ProcessorOutput` gain a variant, and
  `ClientConnector` gains a public field, so exhaustive matches and struct
  literals need updating.
  
  Confirmed with `cargo-semver-checks` against the merge-base: those three
  are the only findings this branch introduces. The others it reports on
  `master` today (`ShareDataPdu::Compressed` and the `ShareDataCtx` fields
  from #1518, `ProcessorBuilder.bulk_decompressor` from #1518,
  `ServerEvent::SetAutoReconnectCookie` from #1509) are present on
  `master` unchanged. The `ironrdp-pdu` additions are additive.
  
  ## Scope
  
  This is the library half. `ironrdp-client`, `ironrdp-web` and the FFI
  bindings gain an arm for the new output but none of them reconnect
  automatically yet; that is the remaining part of #271, and the existing
  `TODO([#271](https://github.com/Devolutions/IronRDP/issues/271))` in `ironrdp-client` marks where it goes.
  
  I kept receive, derive and send together deliberately. Split up, none of
  them is usable on its own: without the receive half there is no way to
  obtain a cookie, and without the send half there is nothing to do with
  one.
  
  ## Tests
  
  Thirteen, all in `ironrdp-testsuite-core`.
  
  On the packet and the derivation: the `SecurityVerifier` matches two
  independently computed HMAC-MD5 vectors of 32 zero bytes under different
  keys, so the tests pin the derivation rather than restating the code;
  the logon ID carries over from the server cookie; the encoding matches
  the 2.2.4.3 field layout byte for byte with `cbLen` fixed at `0x1C`;
  `to_bytes` agrees with `Encode`; it round-trips; and it rejects both a
  wrong packet length and an unknown version.
  
  On verification: a derived answer is accepted, a single flipped byte in
  the verifier is rejected, a correct verifier under a different logon ID
  is rejected, and an answer derived from a different random is rejected.
  
  On the surfacing path: a Save Session Info PDU framed the way a server
  sends it, through the real x224 processor, yields an
  `AutoReconnectCookie` carrying the right logon ID and random bits,
  alongside #1522's logon notification rather than in place of it; and one
  without a cookie surfaces no cookie.
  
  ## Verification
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass on 1.94.1,
  including a `fuzz/` build before the lock check.
  
  ## Note
  
  #1496 also touches the `ClientAutoReconnect` declaration. Whichever of
  the two lands second needs a one-line rebase on the derive attribute;
  happy to take that in either order.

- Decode the Server Heartbeat PDU on the message channel ([#1814](https://github.com/Devolutions/IronRDP/issues/1814)) ([4275b3d7fd](https://github.com/Devolutions/IronRDP/commit/4275b3d7fdf33c0aedeeb6bc9f58bfec83d0893d)) 

  ## Summary
  
  Adds `HeartbeatPdu` (MS-RDPBCGR 2.2.16.1), decoded on the MCS message
  channel that #1347/#1348 wired up for Auto-Detect. The Heartbeat PDU is
  server-to-client only and defines no client response; the client is free
  to ignore `count1`/`count2`.
  
  Fixes a real gap this exposed: the message-channel demux in
  `ironrdp-session` unconditionally decoded incoming traffic as an
  Auto-Detect Request PDU. A Heartbeat PDU arriving on the same channel
  would fail that decode and error the session. The demux now peeks the
  security-header flags first (masking
  `SEC_RESET_SEQNO`/`SEC_IGNORE_SEQNO`, which MS-RDPBCGR 2.2.8.1.1.2.1
  says MUST be ignored, the same way
  `MultitransportRequestPdu`/`MultitransportResponsePdu` already do) and
  dispatches by the masked value. Anything it doesn't recognize is logged
  and ignored rather than treated as session-fatal, matching the
  forward-safe posture the connect-time demux in `ironrdp-connector`
  already has.
  
  Multitransport Bootstrapping, the other optional feature the message
  channel carries, is already fully implemented ([#1098](https://github.com/Devolutions/IronRDP/issues/1098)); this PR is the
  Heartbeat half only.
  
  ## Validation
  
  `cargo xtask check fmt/typos/tests/locks` and `wasm check` all pass.
  `lints` has one pre-existing failure on master unrelated to this diff
  (`clippy::result_large_err` on `ErrorInfoDisconnectHandle::disconnect`,
  fix filed as #1812); this PR's own diff is clean under the same lints
  run.
  
  New tests: `HeartbeatPdu` encode/decode round-trip plus the same
  seqno-flag and cross-PDU-discriminator coverage
  `MultitransportRequestPdu` has, in `ironrdp-pdu`; a `Processor::process`
  integration test in `ironrdp-testsuite-core` covering the full
  message-channel path (Heartbeat produces no output, Auto-Detect still
  works alongside it, and an unrecognized PDU is ignored rather than
  erroring).

- Honor input timing settings ([#1872](https://github.com/Devolutions/IronRDP/issues/1872)) ([ffb921c211](https://github.com/Devolutions/IronRDP/commit/ffb921c211477c6b77cccaf98eb500d627513374)) 

  Wire MinInputSendInterval and KeepAliveInterval through the client
  configuration and active session loop.
  
  Delay mouse-only batches without delaying forced input, keep the
  software-rendered cursor aligned with the final batched position, and
  synthesize current-position keepalives using native defaults and value
  handling.
  
  Keep idle timeout, cache, video, device, and security policy settings
  explicitly unsupported until their owning backends exist.

- [**breaking**] Answer bandwidth measurements during the session ([#2012](https://github.com/Devolutions/IronRDP/issues/2012)) ([9be90a7674](https://github.com/Devolutions/IronRDP/commit/9be90a767483cb4ac8153425a177da8ebfbde132)) 

  The session answered RTT requests on the message channel but ignored
  continuous Bandwidth Measure Start and Stop. A shared
  `AutoDetectResponder` now answers them.
  
  - A continuous reliable Start opens or resets a window. Received TCP
  frames contribute their complete length when no RDP Security Header is
  present, including fast-path and TPKT/X224/MCS framing. Where a Security
  Header is present, only bytes following it count.
  - Results use the measured elapsed time with a 1 ms floor and saturating
  counters. A missing or untimed window returns 1 ms and zero bytes.
  - Connect-time requests belong to the connection sequence handled by
  `ironrdp-connector` and are ignored here. Lossy-tunnel requests are not
  answered on the main connection.

- [**breaking**] Follow server redirection to the target host ([#2208](https://github.com/Devolutions/IronRDP/issues/2208)) ([485657a6ec](https://github.com/Devolutions/IronRDP/commit/485657a6eca682a0356e100a9fda229342861fd7)) 

  An RD Connection Broker sends a Server Redirection PDU to move a client
  to the session host that holds or will hold its session. IronRDP
  rejected the PDU as an unexpected share control PDU, so a client could
  not connect to an RDS farm through its broker.
  
  The PDU layer decodes and encodes the Server Redirection Packet with
  every optional field, the target net addresses and the Base64 target
  certificate container, in both its Enhanced Security variant (share
  control type PDUTYPE_SERVER_REDIR_PKT) and its Standard Security
  variant. The Client Info PDU can carry an opaque password cookie
  verbatim, as FreeRDP sends it.
  
  The connector recognizes the PDU during capabilities exchange and
  connection finalization and ends in a `Redirected` state instead of
  failing; `connect_finalize` returns a `ConnectionOutcome` that is either
  `Connected` or `Redirected`. The session reports a redirection received
  later as `ActiveStageOutput::ServerRedirection`. PDUs marked
  LB_NOREDIRECT are ignored.
  
  `ironrdp-client` now always advertises redirection support in the Client
  Cluster Data and follows a redirection for every front-end: it closes
  the connection, emits `RdpOutputEvent::ServerRedirected` and reconnects
  with the load-balancing information as the X.224 routing token, the
  session ID in the cluster data, the password cookie in the Client Info
  PDU and the redirected user name and domain. Transport, TLS and CredSSP
  settings are kept. A direct connection tries the target addresses first,
  like mstsc, and authenticates the FQDN; `use redirection server name`
  (ActiveX `UseRedirectionServerName`) or a gateway puts the names first.
  A target certificate reported by the broker is accepted under strict
  validation, a TsvUrl mismatch aborts, and at most 8 redirections are
  followed. RDSTLS-encrypted passwords are refused as unsupported.
  
  `ironrdp-server` gains a redirection handle that sends the PDU to the
  active client and waits for it to disconnect, and reports the routing
  token, redirected session ID and password cookie of an incoming
  connection. The FFI exposes the redirection outcome to .NET.

### <!-- 1 -->Features

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

- Negotiate static channel chunk sizing ([#1622](https://github.com/Devolutions/IronRDP/issues/1622)) ([4e3903fbbe](https://github.com/Devolutions/IronRDP/commit/4e3903fbbef2904505f35e3437a7106807ac5987)) 

  Use the validated server VCChunkSize for outgoing static virtual channel
  data and retain 1600-byte chunks when it is absent or invalid.
  
  Apply refreshed values after reactivation across native, web, and FFI
  active stages while preserving channel flags.

- Forward negotiated windowing orders ([#1631](https://github.com/Devolutions/IronRDP/issues/1631)) ([0c3fbe78b4](https://github.com/Devolutions/IronRDP/commit/0c3fbe78b4366533b9fcea046b2b53654e003a72)) 

  Preserve Window List support during activation.
  Forward validated orders through ActiveStage and the raw FFI output.
  Desktop and web consumers retain their existing behavior.

- Add Input DVC and ActiveX touch ([#1647](https://github.com/Devolutions/IronRDP/issues/1647)) ([a912e19bd2](https://github.com/Devolutions/IronRDP/commit/a912e19bd2bb31f403fd7c35c8efd729a5ab5f6f)) 

  Implement MS-RDPEI for multi-touch over the dynamic virtual channel
  Microsoft::Windows::RDS::Input, and wire Windows pointer messages in
  ActiveX through session encode helpers.
  
  Introduce ironrdp-rdpei PDUs and processors, register the channel from
  the client, encode touch frames from ActiveX WM_POINTER*, and cover the
  protocol with unit and integration tests.

- Expose MS-RDPEI pen and multi-touch ([#1652](https://github.com/Devolutions/IronRDP/issues/1652)) ([51f8738ea1](https://github.com/Devolutions/IronRDP/commit/51f8738ea156daeea292c63bba09d9137791af57)) 

  Extend the agent RPC path beyond single-contact touch so multi-contact
  frames, pen frames, and dismiss-hovering can be driven end-to-end
  against a real host.
  
  Wire Pen/Dismiss through rpc, daemon, CLI, ActiveX control RPC, client
  input loop, and session encode helpers. Add pen contact flag legality
  checks and round-trip coverage.

- Support automatic reconnect ([#1662](https://github.com/Devolutions/IronRDP/issues/1662)) ([34d7b31dc5](https://github.com/Devolutions/IronRDP/commit/34d7b31dc5039273dc5502d0ca3301f256eb6154)) 

  Support automatic reconnect through the ActiveX control with bounded
  host-approved retries after transport loss.
  
  Reuse ARC cookies only for resumed sessions, reject server ARC status
  failures, and report success after active-session traffic.
  
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

- [**breaking**] Record byte offset on decode and encode error variants ([#1266](https://github.com/Devolutions/IronRDP/issues/1266)) ([a1f9189c30](https://github.com/Devolutions/IronRDP/commit/a1f9189c307516361a8faff6ecb7c1690b267998)) 

  ## Summary
  
  Records a byte offset on every `DecodeErrorKind` and `EncodeErrorKind`
  variant that can know one, so decode and encode errors surface the
  position in the input stream where the failure was detected. Reshaped
  twice after review; see "Review history" below if you reviewed an
  earlier shape.
  
  Contributes to the structured-fuzzing roadmap in #1120 by giving
  crash-replay analysis and Wireshark-style malformed-PDU reporting the
  byte-offset dimension that source `Location` ([#1262](https://github.com/Devolutions/IronRDP/issues/1262)) alone does not
  provide.
  
  ## API
  
  Variants that gain `offset: Option<usize>`:
  
  - `DecodeErrorKind::NotEnoughBytes { received, expected, offset }`
  - `DecodeErrorKind::InvalidField { field, reason, offset }`
  - `DecodeErrorKind::UnexpectedMessageType { got, offset }`
  - `DecodeErrorKind::UnsupportedVersion { got, offset }`
  - `DecodeErrorKind::UnsupportedValue { name, value, offset }`
  - `EncodeErrorKind` mirrors the same shape for the encode side

- Add location redirection ([#1778](https://github.com/Devolutions/IronRDP/issues/1778)) ([1cee7a8613](https://github.com/Devolutions/IronRDP/commit/1cee7a86135a0556c01965d0406233bd7df367a9)) 

  Implement MS-RDPEL v1 codecs and the location DVC state machine, then
  route the ActiveX methods through the bounded client input queue.
  
  Preserve mstsc-compatible validation and altitude caching while
  surfacing inactive sessions, channel readiness, queue pressure, and
  encoding failures. Coordinates are caller-supplied only and are never
  logged or persisted.

- Render the EGFX graphics pipeline output ([#1461](https://github.com/Devolutions/IronRDP/issues/1461)) ([d414622231](https://github.com/Devolutions/IronRDP/commit/d414622231ed3a944d8896118f409edead1d3df3)) 

  ## Summary
  
  - The `ironrdp-egfx` compositor exposes changed output regions via
  `drain_output()`, but nothing consumed them, so an EGFX session decoded
  frames and dropped them.
  - Drains the compositor from `ActiveStage::process`: composites each
  completed-frame `OutputUpdate` into the `DecodedImage` and emits
  `ActiveStageOutput::GraphicsUpdate`, so every session consumer renders
  EGFX with no new code. No-op when the graphics DVC is not registered.
  - Adds a by-type mutable DVC accessor `DrdynvcClient::get_dvc_mut`,
  mirrored on the session (the x224 processor and `ActiveStage`), matching
  the existing `get_dvc`.
  - All regions drained in one pass are composited first by
  `composite_graphics_updates`, then surfaced as a single `GraphicsUpdate`
  covering their union. A consumer may redraw whatever region an update
  names, and `ironrdp-client` rebuilds the whole framebuffer for each one,
  so emitting per region would copy the desktop once per rectangle. A
  single `RDPGFX_SOLIDFILL_PDU` or `RDPGFX_CACHE_TO_SURFACE_PDU` can name
  up to `u16::MAX` of them.
  
  ## Validation
  
  - `cargo xtask check fmt/lints/tests/typos/locks` all pass, including
  the dependency guard.
  - Four tests cover the coalescing: two disjoint deltas collapsing to the
  rectangle spanning both, 64 deltas yielding exactly one region, a single
  delta passing through unwidened, and an empty drain surfacing nothing.
  Checked against a reverted coalescing, where the first two fail.
  - The loop lives in `composite_graphics_updates` rather than inline in
  `ActiveStage::process` so it can be tested without standing up an x224
  processor. It takes `(ExclusiveRectangle, Vec<u8>)` rather than
  `OutputUpdate`, since that type is `#[non_exhaustive]` and cannot be
  constructed from `ironrdp-session`; this keeps the change inside the
  crate instead of widening an already-merged API for testability.
  
  ## Notes
  
  - `ironrdp-session` already depends on `ironrdp-displaycontrol` and
  `ironrdp-graphics`; `ironrdp-egfx` is consumed the same way. The guard
  keeping `ironrdp-connector` and `sspi` out of `ironrdp-session` still
  passes.
  - Reuses `apply_rgba32` (previously gated behind the `qoi` feature);
  converts the compositor's exclusive-rectangle regions to the session's
  inclusive-rectangle convention.
  - #1377 and #1460 have both merged, so this now sits directly on master
  and the diff is its own change alone: 6 files, +160/-6.
  - #1462 is stacked on this one, so the merge order is this then #1462.
  - Part of #1464. Motivated by #1446.

- Handle multitransport on message channel ([#1836](https://github.com/Devolutions/IronRDP/issues/1836)) ([561aebd0d8](https://github.com/Devolutions/IronRDP/commit/561aebd0d85e1a01a5c7ae63e5befd71d55887e3)) 

  Route SEC_TRANSPORT_REQ through the negotiated MCS message channel and
  surface the decoded request to active-session consumers. Encode the
  client response on the same channel, expose that encoder through
  ActiveStage, and fail explicitly when the message channel was not
  negotiated.
  
  Ignore a request received on the I/O channel because the specification
  does not require terminating the main session for this optional
  transport anomaly.

- Expose routable DVC batches ([#1857](https://github.com/Devolutions/IronRDP/issues/1857)) ([8369ea6989](https://github.com/Devolutions/IronRDP/commit/8369ea69897e6d6892d2c7d8f93f35b943467e9f)) 

  Expose Soft-Sync tunnel state and typed processing through ActiveStage.
  
  Return validated DVC batches for resize and RDPEI requests.
  Keep existing TCP encoding methods as route-safe compatibility wrappers.

- Present EGFX dirty regions ([#1874](https://github.com/Devolutions/IronRDP/issues/1874)) ([2b44c62bdb](https://github.com/Devolutions/IronRDP/commit/2b44c62bdbd997f4aa33bdbaf4749ac4ecc85140)) 

  Propagate validated ResetGraphics extents into the session framebuffer
  and deliver exact packed desktop damage to ActiveX without rebuilding
  full image snapshots.
  
  Coalesce only fully covered regions, preserve sparse updates with a
  64-event and 256 MiB pixel-data budget, and backpressure without
  evicting accepted frame or static-channel payloads. Update retained GDI
  and RPC surfaces in place. Reuse framebuffer storage with fallible
  growth, and retain software cursor shape, position, visibility, and
  clipping across graphics resets. Keep V8 non-AVC negotiation and
  full-frame output fallback unchanged. Rejected oversized resets still
  destroy prior surfaces and are reported as unsupported without
  allocating a session framebuffer.

- [**breaking**] Multi-monitor layouts with orientation and DPI ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857)) 

  Display Control (MS-RDPEDISP) was only usable for a single primary
  monitor at 100% device scale. This lets clients send complete monitor
  layouts during the session, with real orientation and DPI, and wires it
  into the viewer and the ActiveX control.
  
  ## Changes
  
  **`ironrdp-displaycontrol`** (breaking)
  - `DisplayControlMonitorLayout::new` now rejects overlapping monitors
  and, for multi-monitor layouts, monitors not adjacent to another one
  (MS-RDPEDISP 3.1.5.2). Layouts it previously accepted can now fail.
  - `fit_to_capabilities` enforces the server's `MaxNumMonitors` and
  maximum monitor area (MS-RDPEDISP 3.2.5.2): extra monitors are dropped
  (the primary is kept), a single oversized monitor is scaled down keeping
  its aspect ratio, and an oversized multi-monitor layout is rejected
  rather than shrunk (which would move monitors relative to each other).
  - `DisplayControlClient` stores the server capabilities and gains
  `encode_monitor_layout`; `encode_single_primary_monitor` fits to the
  capabilities too.
  - `new_single_primary_monitor` clamps the desktop scale to 100..=500 and
  derives the device scale (100/140/180) from it instead of hard-coding
  100%; a square monitor is landscape.
  - Helpers: capability getters, `area`, `desktop_size`,
  `MonitorLayoutEntry::adjust_desktop_scale_factor`,
  `DeviceScaleFactor::from_value` / `from_desktop_scale_factor`.

- Play video optimized remoting with geometry tracking ([#2185](https://github.com/Devolutions/IronRDP/issues/2185)) ([ed65759d4a](https://github.com/Devolutions/IronRDP/commit/ed65759d4ac670f661fbacfb5e15d14ea98d9b38)) 

  ## Summary
  
  Adds MS-RDPEVOR video optimized remoting with MS-RDPEGT geometry
  tracking to the client.
  
  > Stacked on the camera redirection PR (it reuses
  `DrdynvcClient::with_dynamic_channel_factory` and touches the same
  client wiring). Merge that one first; this PR then retargets to
  `master`.
  
  - **`ironrdp-rdpegt`** (new, `no_std`): `MAPPED_GEOMETRY_PACKET` codec
  with RGNDATA regions, and `GeometryClient`, which tracks the mappings
  and notifies an observer.
  - **`ironrdp-rdpevor`** (new): TSMM PDU codecs, `VideoControlClient` /
  `VideoDataClient`, and a sans-I/O `VideoPresenter` that:
    - answers Start Presentation only once it has a decoder;
  - reassembles fragmented samples and decodes them through the existing
  `ironrdp_egfx::decode::H264Decoder` trait;
    - schedules pictures from their timestamps against an injected clock;
    - scales and clips them to the mapped geometry;
    - asks for a key frame after loss or a decode failure.
  - **`ironrdp-egfx-native`** (new): `MediaFoundationH264Decoder`, an
  `H264Decoder` backed by the H.264 decoder MFT shipped with Windows. It
  runs in low-latency mode, follows format changes, and converts NV12 with
  BT.601 limited or BT.709 full range.
  - **`ironrdp-session`**: `ActiveStage::composite_rgba32`, which
  composites externally rendered pixels and records their damage.
  - **`ironrdp-client`**
    - `video` feature (in `all`) and an `openh264` feature.
  - The default decoder is Media Foundation on Windows, or OpenH264 with
  `openh264`. With no decoder the channels are still declined.
    - A presentation timer and a compositing step in the session loop.
  - A new `ironrdp_egfx:i:<0|1>` property. Windows closes the video
  channels unless the graphics pipeline is on.
  - **`ironrdp-viewer`** / **`ironrdp`**: `openh264`, `client-video`,
  `client-openh264` features.
  - **`ironrdp-daemon`**: built with `video`, so `ironrdp-agent` can play
  RDPEVOR video.
  
  ## Verification
  
  - **Live, Windows Server 2025 through `ironrdp-agent`** (`--prop
  ironrdp_egfx:i:1`):
    - YouTube in Edge streamed as a 672x448 RDPEVOR presentation.
  - 3,035 pictures were presented (86 dropped catching up), placed at the
  RDPEGT mapping; the session screenshot shows the video in place.
  - Without the graphics pipeline the server closes the video channels.
  Geometry and video channels are reopened during the session, which the
  factory registration handles.
  - **Tests**: `ironrdp-testsuite-core` covers the MS-RDPEVOR and
  MS-RDPEGT specification examples, malformed input, reassembly, timing,
  loss and key-frame requests, and scaling and clipping.
  - The real H.264 sample from MS-RDPEVOR 4.3 decodes through OpenH264
  (`openh264-bundled`) and through Media Foundation (Windows), and lands
  at the MS-RDPEGT 4.1 rectangle.
    - The two decoders agree within a mean channel difference of 2.
  - **Checks**: `cargo xtask check fmt -v`, `cargo xtask check lints -v`
  and `cargo xtask check locks -v` pass.
  - **Known**: `e2e::egfx_moves_onto_the_udp_tunnel_with_soft_sync` fails
  locally on `master` as well; it is unrelated to this change.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- [**breaking**] Remote touch and pen through RDPEI ([#2198](https://github.com/Devolutions/IronRDP/issues/2198)) ([637f838ea0](https://github.com/Devolutions/IronRDP/commit/637f838ea0e2adcfbfddc94b147958ec672898ff)) 

  ActiveX now remotes pen input. PT_PEN WM_POINTER messages become RDPEI
  pen frames with pressure, rotation, tilt, barrel, eraser and inverted
  flags, hover and contact state. Coalesced samples from
  GetPointerPenInfoHistory become one frame each, timed by the digitizer.
  Pens get their own device IDs when the server supports multipen
  injection, otherwise only device 0 is used.
  
  The viewer handles WindowEvent::Touch. It groups the per-contact winit
  events into frames that restate unchanged contacts and maps positions
  through the same scaling as the mouse. Windows mouse messages promoted
  from touch or pen are dropped, so input is not sent twice. winit 0.30
  reports pens as touch points with pressure only, so the viewer sends
  them as touch contacts.
  
  Both front-ends fall back to mouse emulation while the server does not
  accept touch or pen, for example before logon. ironrdp-client reports
  this through the new RdpOutputEvent::RdpeiInputState variant, which is a
  breaking change for exhaustive matches. The client now advertises RDPEI
  V300 with multipen injection, and RdpeiClient never advertises a version
  above the server's. A closed channel resets the negotiation.
  
  The contact lifetime state machine and the touch frame builder live in
  ironrdp_rdpei::contact so front-ends share them.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- [**breaking**] Redirect several printers with cached settings ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995)) 

  Printer redirection could announce only the Windows default queue. Every
  printer reused the `PRN1` port name and the default flag,
  `PAKID_PRN_CACHE_DATA` was dropped so server-side printing preferences
  were lost on reconnect, and the viewer ignored the `.rdp`
  `redirectprinters` property that the daemon honored.
  
  - **`ironrdp-rdpdr`:** `Rdpdr::with_redirected_printer` announces an
  `RdpdrPrinter` with its own `PRN<n>` port name, flags, and cached
  configuration. Only the first default printer keeps the default flag
  (MS-RDPEPC 2.2.2.1). `RdpdrBackendProduct` holds a list of printers and
  an optional `RdpdrPrinterCacheStore`. Cache-data PDUs are decoded in the
  new `pdu::epc` module, saved through the embedder's store, and replayed
  as `CachedPrinterConfigData` on the next announcement. A driver from an
  add event replaces the announced driver unless the user chose it.
  Entries are keyed by the local queue name and record the name the server
  gave the printer (a rename event, or a queue a user installed on the
  redirected port), which the next announcement uses (MS-RDPEPC 3.2.5.1.3,
  3.2.5.1.6); the local Windows queue is never renamed by the server. The
  channel does no I/O. Malformed cache and `PAKID_PRN_USING_XPS` PDUs fail
  the channel like other malformed RDPDR packets (MS-RDPEFS 3.1.5.2); a
  valid `PAKID_PRN_USING_XPS` is ignored because XPSFORMAT is never
  announced.
  - **`ironrdp-rdpdr-native`:** `WindowsPrinterRedirection` selects the
  default queue, every local queue and printer connection (`EnumPrintersW`
  with `PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS`, like FreeRDP), or
  named queues, with per-queue driver overrides. One worker serves all
  printers and binds each job to its device. Jobs keep the `RAW` datatype
  because the server sends device-ready PRN data (MS-RDPEPC 3.2.5.1.2); a
  queue default such as `NT EMF 1.008` or `XPS_PASS` would make the print
  processor reinterpret it. `FileRdpdrPrinterCacheStore` persists cache
  entries; it reads each entry once and writes on a background thread, so
  cache events do not block the RDPDR channel on disk I/O. The README
  records what Easy Print would need.
  - **`ironrdp-svc` / `ironrdp-session`:** chunked static-channel PDUs no
  longer always carry `CHANNEL_FLAG_SHOW_PROTOCOL`. The flag now comes
  only from the message or from a channel defined with
  `CHANNEL_OPTION_SHOW_PROTOCOL`, as in FreeRDP. A live Windows 11 host
  silently dropped every RDPDR device announcement larger than one
  1600-byte chunk, so printers announced with cached settings were never
  redirected. This departs from the MS-RDPBCGR 3.1.5.2.1 wording in favor
  of observed Windows and FreeRDP behavior.
  - **`ironrdp-client`:** `RdpdrConfig::printers` mirrors
  `redirectprinters` and can drop a product's printers; it defaults to
  true. The channel announces every printer and marks a lone printer as
  the default.
  - **`ironrdp-daemon`:** printer settings persist per server under
  `%APPDATA%\ironrdp-agent\printer-cache` (or
  `IRONRDP_AGENT_PRINTER_CACHE`).
  - **`ironrdp-viewer`:** adds `--printer-all`, `--printer-queue
  <queue>[,<driver>]`, and `--printer-default`. `redirectprinters:i:1` now
  redirects the default printer unless `ironrdp_rdpdr:i:0` is set, as in
  the daemon. mstsc writes that property by default, so stock `.rdp` files
  now redirect the default printer and save its server-side settings in
  the per-server cache folder; this is intended and should be called out
  in the release notes.
  
  There is no fallback driver: a Windows 11 host that lacks the announced
  driver still accepts the announcement and creates no queue, so the
  client cannot detect it. `--printer-queue <queue>,<driver>` picks a
  driver the host has.
  
  Easy Print (XPSFORMAT) is not implemented: it needs the MS-RDPEXPS
  `TSVCTKT` and `XPSRD` channels, PrintTicket conversion on the client,
  XPS mode, and local XPS printing.
  
  The public API changes incompatibly: `RdpdrPrinter::new` no longer marks
  the printer as the default, `RdpdrBackendProduct::with_printer` appends
  instead of replacing, `Devices::add_printer*` return whether a `PRN<n>`
  port name was left, and `RdpdrConfig` has a new public `printers` field.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- [**breaking**] Decode RemoteApp windowing orders into typed structs ([#2195](https://github.com/Devolutions/IronRDP/issues/2195)) ([2bc793ef3f](https://github.com/Devolutions/IronRDP/commit/2bc793ef3f5bb21b4a219175e653a2f65d6f5dac)) 

  Windowing Alternate Secondary Drawing Orders (MS-RDPERP 2.2.1.3) were
  only validated and forwarded as raw bytes, so every RemoteApp front-end
  re-parsed them by hand and ignored what it did not parse: icons, cached
  icons, window shapes, visibility regions, notification icons, the active
  window and the z-order.
  
  ironrdp-pdu now decodes each order into a typed WindowOrder,
  NotifyIconOrder or DesktopOrder, including TS_ICON_INFO and
  TS_CACHED_ICON_INFO, and encodes them back. ActiveStageOutput and the
  client's RdpOutputEvent carry the decoded orders instead of a slow-path
  update. The ActiveX control consumes the typed orders in place of its
  own parser, and the .NET binding still exposes a slow-path Orders update
  by re-encoding them.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)

- Persistent bitmap cache, RemoteFX and Alpha ([#2194](https://github.com/Devolutions/IronRDP/issues/2194)) ([52476f3cee](https://github.com/Devolutions/IronRDP/commit/52476f3cee6015ddb04a62e0377b17b33ba78e19)) 

  The graphics pipeline client dropped RemoteFX (RDPGFX_CODECID_CAVIDEO)
  and Alpha (RDPGFX_CODECID_ALPHA) commands into on_unhandled_pdu, and it
  never offered a persistent bitmap cache, so every reconnect waited for
  the server to upload tiles mstsc already had.
  
  Persistent cache (MS-RDPEGFX 3.3.1.5): the embedder supplies a
  PersistentCacheStore through
  GraphicsPipelineClient::with_persistent_cache. Right after
  CapabilitiesConfirm the client sends a CacheImportOffer with the stored
  keys, capped at 5461 entries and at the slot count and byte size of the
  confirmed cache (16 MB and 4096 slots with SMALL_CACHE, THINCLIENT or
  version 10.3, otherwise 100 MB and 25600). Entries over 64 KiB are
  neither offered nor saved, matching mstsc, which never offers a larger
  one. The limits read CodecCapabilities::small_cache, which already
  reports that rule, and the compositor's cache cap uses the same
  constants as the offer. The CacheImportReply loads the accepted entries
  into the assigned one-based slots; invalid slots, unsolicited replies
  and malformed entries are skipped. Imported tiles behave like
  SurfaceToCache tiles: they survive ResetGraphics, go with
  EvictCacheEntry, and count against the compositor budget. When the
  channel closes, the cache is saved back most recently used first; an
  empty cache does not overwrite the store. A client-initiated protocol
  reset saves the cache before discarding it and offers it again once the
  reset is confirmed. No file I/O lives in the core crates.
  
  ironrdp-client implements the store as one file in the format of mstsc's
  CacheNNNN.bin files, which FreeRDP also uses ("RDP8bmp\0", version 6,
  then key, width, height and BGRA pixels per entry), so a cache file
  moves between the three clients. It was checked against mstscax.dll's
  CacheFileReader, CacheImportOffer and CacheExport, and a real mstsc
  Cache0000.bin (6430 entries) lists and loads. Entries mstsc would refuse
  (key 0, over 64 KiB) are never written. The file (FilePersistentCache,
  written to an owner-only temporary and renamed into place, and read with
  FILE_SHARE_DELETE on Windows so another session can replace it),
  configured through ChannelConfig::egfx_persistent_cache or
  ConfigBuilder::with_egfx_persistent_cache. The cache is off by default,
  unlike mstsc, whose BitmapPersistenceEnabled property defaults to 1
  (mstscax.dll's property table), so desktop pixels are never written to
  disk unless asked for; the .rdp key bitmapcachepersistenable:i:1 selects
  a per-user default file and :i:0 turns it off. The ActiveX control
  implements AdvancedSettings.BitmapPeristence (previously E_NOTIMPL),
  defaulting to 0 (unlike mstscax.dll) and reading any non-zero value as
  on; it feeds bitmapcachepersistenable to the client on both connect
  paths and through the modern RDP property API. The viewer gains
  --egfx-cache <PATH>.

### <!-- 4 -->Bug Fixes

- Key auto-detect optional fields off requestType, not the Option ([#1491](https://github.com/Devolutions/IronRDP/issues/1491)) ([f9cc62fa2c](https://github.com/Devolutions/IronRDP/commit/f9cc62fa2ccb4f211838dd0961c19cdf3b79a38e)) 

  ## Summary
  
  MS-RDPBCGR decides which optional fields an auto-detect message carries
  by its `requestType`. Two of these message types encoded them by
  inspecting which `Option`s happened to be set instead, so the encoder
  and the decoder disagreed about the wire.
  
  This started as a fix for `BandwidthMeasureStop` alone. Review found the
  connect-time fallback was still non-compliant, and checking whether the
  same shape appeared elsewhere in the file turned up
  `NetworkCharacteristicsResult` with the identical defect and a worse
  consequence, so both are fixed here rather than one now and one later.
  
  ## The two failure modes
  
  **Connect-time stop with no payload: encodes to bytes the decoder
  rejects.**
  
  ```rust
  AutoDetectRequest::BandwidthMeasureStop {
      sequence_number: 7,
      request_type: BW_STOP_CONNECT_TIME,
      payload: None,
  }
  ```
  
  encodes to ten bytes with `headerLength` 0x06 and no length field.
  Decoding those bytes
  fails with `not enough bytes provided to decode: received 0 bytes,
  expected 2 bytes`,
  because the decoder reads `payloadLength` back for every
  `BW_STOP_CONNECT_TIME`.
  
  **UDP stop with a payload: silently loses it.**
  
  ```rust
  AutoDetectRequest::BandwidthMeasureStop {
      sequence_number: 3,
      request_type: BW_STOP_RELIABLE_UDP,
      payload: Some(vec![0xAA; 16]),
  }
  ```
  
  encodes the length and the sixteen bytes, which the decoder never reads
  back for
  `BW_STOP_RELIABLE_UDP` or `BW_STOP_LOSSY_UDP`.
  `AutoDetectReqPdu::decode` does not check
  for trailing bytes, so the payload is dropped in transit without an
  error rather than
  refused.
  
  The second is the worse of the two: a round trip that loses data and
  reports success.
  
  ## Network Characteristics Result (2.2.14.1.5)
  
  The same defect, found by sweeping the file rather than reported.
  
  `0x0840` carries baseRTT and averageRTT, `0x0880` carries bandwidth and
  averageRTT, `0x08C0` carries all three, and the decoder already read
  them on that basis. Encoding from the `Option`s meant:
  
  - a `0x0840` result with no `base_rtt_ms` wrote a body the decoder
  cannot read;
  - a `0x0840` result carrying `bandwidth_kbps` instead wrote that
  bandwidth into the slot the decoder reads as baseRTT, so the value came
  back **silently corrupted** rather than rejected;
  - `headerLength` was always derived from `requestType`, so it could
  contradict the body it described.
  
  Encode and `size()` now consult `requestType` through a shared helper, a
  value the type does not carry is dropped rather than written, and a
  missing one that the type requires is an error.
  
  ## Fix
  
  `Encode` and `size()` now key off `requestType`, matching the decoder.
  That makes the
  wire form canonical:
  
  - an absent payload on a connect-time stop encodes as a zero length and
  reads back as an
    empty one;
  - a payload on a UDP stop never reaches the wire.
  
  Decoding and re-encoding reproduces the same bytes in every case. The
  types can still
  express states the protocol cannot, so value-level identity is not
  achievable, but byte
  stability is, and that is what a decoder consuming real traffic depends
  on.
  
  No public signatures change; this is a behaviour fix inside the existing
  `Encode` impl.
  
  ## Tests
  
  New `pdu/autodetect.rs` in `ironrdp-testsuite-core`:
  
  - a connect-time stop with no payload round-trips, and `size()` agrees
  with `encode()`;
  - a UDP stop carrying a payload encodes header-only and comes back with
  `payload: None`,
    for both the reliable and lossy request types;
  - a connect-time stop preserves a real payload;
  - decode-then-encode reproduces identical bytes for both shapes.
  
  Three of the four fail against the current encoder and pass with the
  fix. The fourth is
  a control that passed before and after.
  
  Note the existing `pdu_round_trip` fuzz oracle would not have caught
  this even with
  auto-detect added to it, since it discards the result of the re-decode
  (`let _ =`). That
  is deliberate in the oracle's design and out of scope here, but it is
  why this went
  unnoticed.
  
  ## How this was found
  
  Writing a test for the connect-time bandwidth measurement in #1465,
  which needs to answer
  a connect-time stop. The `None` case was constructed to check the reply
  path stayed
  lenient and turned out not to be constructible on the wire at all.
  
  ## Verification
  
  - `cargo xtask check fmt -v` green
  - `cargo xtask check lints -v` green
  - `cargo xtask check tests -v` green
  - `cargo xtask check typos -v` green
  - `cargo xtask check locks -v` green

- Decode indexed pointers and foreground RLE runs ([#1519](https://github.com/Devolutions/IronRDP/issues/1519)) ([ad19280762](https://github.com/Devolutions/IronRDP/commit/ad192807620bcc3a0467eaeb07173a79cb1da257)) 

  ## Summary
  
  4bpp and 8bpp New/Large pointer shapes previously could not use the
  active session palette, and malformed or unsupported pointer data could
  terminate the session. This decodes indexed XOR masks with the current
  palette and falls back to the default cursor while evicting stale cached
  data when decoding fails.
  
  It also corrects RLE foreground runs so only set-foreground variants
  consume a foreground pixel.
  
  Palette updates now follow the RDP wire format (type, padding, 256
  packed RGB triplets) and are applied from both fast- and slow-path
  updates, ensuring indexed pointer decoding uses the negotiated palette.
  
  ## Tests
  
  - `cargo test -p ironrdp-graphics -p ironrdp-session`
  - `cargo test -p ironrdp-session palette`
  - `cargo clippy -p ironrdp-graphics -p ironrdp-session --all-targets --
  -D warnings`
  
  ---------

- Share bulk decompression across output paths ([#1518](https://github.com/Devolutions/IronRDP/issues/1518)) ([6151e21bf5](https://github.com/Devolutions/IronRDP/commit/6151e21bf58b7297e9b4abc2167aa36fc2ba77e4)) 

  Bulk compression state is stream-wide, but Fast-Path and slow-path
  outputs previously used separate or missing decompression paths. This
  could corrupt history-dependent server updates or leave negotiated
  slow-path compression undecodable.
  
  This change owns the negotiated bulk decompressor in `ActiveStage` and
  passes it to both X.224 and Fast-Path processing. It retains Share Data
  compression metadata through the PDU context, resets decompression
  history on reactivation, and initializes consumers from the connection's
  negotiated compression type.
  
  Fast-Path now decompresses each fragment before reassembly so
  compression flags apply at packet boundaries. Failures expose bounded
  protocol metadata without retaining remote payloads or decoder details.
  
  Tests cover Share Data metadata propagation, slow-path decompression
  behavior, fragmented Fast-Path reassembly and bounded errors, and
  compressed Fast-Path updates after reactivation.
  
  ---------

- Recover from malformed bitmap updates ([#1521](https://github.com/Devolutions/IronRDP/issues/1521)) ([20e2d414e5](https://github.com/Devolutions/IronRDP/commit/20e2d414e5ac060db25a100ed219f417e19f79b2)) 

  ## Summary
  - safely discard malformed bitmap and pointer updates without
  terminating the session
  - request at most one capability-gated full redraw per activation
  - propagate Refresh Rect and Suppress Output support through the
  connector and generic client
  - retain FFI compatibility and focused malformed-update regression
  coverage
  
  ## Stack
  Depends on `copilot/fix-session-share-bulk-decompression` (`47270c2a`).
  
  ## Validation
  - `cargo fmt --all -- --check`
  - `cargo test -p ironrdp-session --lib`
  - `cargo test -p ironrdp-error --lib`
  - `cargo check -p ironrdp-connector`
  - `cargo check -p ironrdp-client --features native-tls`
  - `cargo check -p ffi --features ironrdp/native-tls`
  
  ---------

- Preserve RDP6 bitmap orientation ([#1524](https://github.com/Devolutions/IronRDP/issues/1524)) ([8f1737a287](https://github.com/Devolutions/IronRDP/commit/8f1737a28765c0e29bfab1584cd376805b7e174e)) 

  ## Summary
  - restore bottom-up scanline composition for RDP6 bitmap updates
  - add asymmetric raw/RLE bitmap-update regression coverage
  - synchronize retained ActiveX DIB updates with GDI and surface copy
  failures
  
  ## Validation
  - `cargo test -p ironrdp-session`
  - `cargo test -p ironrdp-activex`
  - `cargo build -p ironrdp-activex --release`
  - verified the real `mstscex.exe -> mstsc.exe -> MsRdpEx.dll ->
  ironrdpax.dll` path renders the authorized test desktop upright without
  bands
  
  ---------

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

- Batch Fast-Path input events ([#1630](https://github.com/Devolutions/IronRDP/issues/1630)) ([3818b48037](https://github.com/Devolutions/IronRDP/commit/3818b480375ec9411d5319d8e7af161f1d662cbf)) 

  Keep outgoing Fast-Path input frames within the 255-event protocol limit
  and preserve their order across FFI.

- Handle MCS Disconnect Provider Ultimatum ([#1692](https://github.com/Devolutions/IronRDP/issues/1692)) ([75ea3b96f0](https://github.com/Devolutions/IronRDP/commit/75ea3b96f08a59e3b70fb75fedc4eb358aa8682c)) 

  Some servers (xrdp) end a session by sending a Disconnect Provider
  Ultimatum on its own, not wrapped in a Send Data Indication. We only
  decoded Send Data Indications, so a clean session end came out as a
  protocol error.
  
  Check for it when the Send Data Indication decode fails and return a
  Disconnect instead.

- Decode concatenated Share Control PDUs on I/O channel ([#1861](https://github.com/Devolutions/IronRDP/issues/1861)) ([2971839d59](https://github.com/Devolutions/IronRDP/commit/2971839d59e5269b511deba4e17868bc4710db84)) 

  Win7 and other legacy servers may concatenate multiple Share Control
  PDUs in one MCS Send Data Indication. Walk each PDU using the Share
  Control Header totalLength field, and fall back to whole-buffer decode
  when the first length is unusable.
  
  This is one of three independent follow-ups from cxxzhang's work in
  #1408.

- Rfx decoder incorrectly computes update region for multi-monitor sessions ([#1982](https://github.com/Devolutions/IronRDP/issues/1982)) ([ea5f247174](https://github.com/Devolutions/IronRDP/commit/ea5f247174eb4af18c3f9150c4263c7fbc785759)) 

  I came across this issue while prototyping multi-monitor support in the
  Teleport client.`clipping_rectangles` incorrectly clamps the incoming
  frame's right/bottom bounds to the channel width/height. This may work
  for single monitor sessions, but fails in the multi-monitor use case
  because frame updates where the framebuffer's dimensions exceed a given
  frame's dimensions. The result is that only the primary display renders
  correctly correctly for sessions with multiple monitors.

- [**breaking**] Serve channels and graphics that Windows moves onto a tunnel ([#2008](https://github.com/Devolutions/IronRDP/issues/2008)) ([bfd16a2e55](https://github.com/Devolutions/IronRDP/commit/bfd16a2e5573d4cd6617dd71b2802b4e7b944a59)) 

  Once Soft-Sync has moved dynamic channels onto the reliable UDP tunnel,
  Windows keeps using the tunnel for more than channel data. Two gaps kept
  the graphics pipeline from working there.



## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.10.0...ironrdp-session-v0.11.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 4 -->Bug Fixes

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.

- Reduce dependency on ironrdp-connector ([#1419](https://github.com/Devolutions/IronRDP/issues/1419)) ([5c22f86a71](https://github.com/Devolutions/IronRDP/commit/5c22f86a7150bc10c26a3be39bfaebf84c67d781)) 

  Drops session's reliance on ironrdp-connector legacy helpers, now sourced from ironrdp-pdu.

- [**breaking**] Remove ironrdp-connector dependency ([#1435](https://github.com/Devolutions/IronRDP/issues/1435)) ([c6a0286dcb](https://github.com/Devolutions/IronRDP/commit/c6a0286dcb49d9ac54c65c4f9325b41e05d541b8)) 

  Removes the last ironrdp-connector coupling from ironrdp-session by
  turning Deactivate-All handling into a bare signal and shifting ownership
  of the Deactivation-Reactivation activation sequence back to each consumer.
  It introduces a ConnectionActivationFactory (fresh sequence per reactivation)
  and an ActiveStageBuilder so session construction no longer depends on
  ConnectionResult.



## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.9.0...ironrdp-session-v0.10.0)] - 2026-06-05

### <!-- 4 -->Bug Fixes

- Decode RGBA QOI bitmaps instead of dropping the frame ([#1341](https://github.com/Devolutions/IronRDP/issues/1341)) ([ef20ea4e90](https://github.com/Devolutions/IronRDP/commit/ef20ea4e90455d6c6db0d3521f6522d1e960c0bb)) 

  Fixes the client-side QOI decode path in ironrdp-session so RGBA-channel QOI frames are decoded and applied to the framebuffer instead of being dropped, improving interoperability with third-party RDP servers and older ironrdp-server builds that emit RGBA QOI.

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.8.0...ironrdp-session-v0.9.0)] - 2026-05-27

### <!-- 1 -->Features

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

- Add bulk compression and wire negotiation ([ebf5da5f33](https://github.com/Devolutions/IronRDP/commit/ebf5da5f3380a3355f6c95814d669f8190425ded)) 

  - add ironrdp-bulk crate with MPPC/NCRUSH/XCRUSH, bitstream, benches, and metrics
  - advertise compression in Client Info and plumb compression_type through connector
  - decode compressed FastPath/ShareData updates using BulkCompressor
  - update CLI to numeric compression flags (enabled by default, level 0-3)
  - extend screenshot example with compression options and negotiated logging
  - refresh tests, FFI/web configs, typos, and Cargo.lock

- Complete pixel format support for bitmap updates ([#1134](https://github.com/Devolutions/IronRDP/issues/1134)) ([a6b41093ce](https://github.com/Devolutions/IronRDP/commit/a6b41093ce4ece081d2538c157f6bc547c3b2607)) 

  Wires missing bitmap pixel formats (8/15/24bpp) into the session rendering
  pipeline so bitmap updates at those depths are rendered instead of being
  dropped, and adds fast-path palette update parsing to support 8bpp indexed
  color sessions.

- Handle Auto-Detect Request PDUs from server ([#1178](https://github.com/Devolutions/IronRDP/issues/1178)) ([4dcad09980](https://github.com/Devolutions/IronRDP/commit/4dcad09980e4f5354e4e435a134cc0956e2fcf9e)) 

  Fixes a crash when the server sends Auto-Detect Request PDUs during an
  active session. After #1176 added ShareDataPdu::AutoDetectReq routing,
  these PDUs decode correctly but hit the catch-all error path in the x224
  processor: "unhandled PDU: Auto-Detect Request PDU".

- Handle slow-path graphics and pointer updates ([#1132](https://github.com/Devolutions/IronRDP/issues/1132)) ([9383380292](https://github.com/Devolutions/IronRDP/commit/938338029290f1be82a7f784d544bb77ac797aeb)) 

  Adds support for slow-path graphics and pointer updates to IronRDP, fixing connectivity issues with servers like XRDP that use slow-path output instead of fast-path. The implementation parses slow-path framing headers and routes the inner payload structures through the existing fast-path processing pipeline by extracting shared bitmap and pointer processing methods.

### <!-- 4 -->Bug Fixes

- Fix pixel format handling in bitmap decoders ([#1101](https://github.com/Devolutions/IronRDP/issues/1101)) ([75863245ab](https://github.com/Devolutions/IronRDP/commit/75863245ab376f15e35c00df434860c93b123633)) 

- Handle row padding in uncompressed bitmap updates ([4262ae75ff](https://github.com/Devolutions/IronRDP/commit/4262ae75ffa5cb1fabb4ca07d598e33d855e8fdd)) 

  Uncompressed bitmap data has rows padded to 4-byte boundaries per
  [MS-RDPBCGR] 2.2.9.1.1.3.1.2.2, but the bitmap apply functions
  expect tightly packed pixel data. Strip the per-row padding before
  passing raw bitmap data to the apply functions.
  
  This fixes garbled bitmap rendering when connecting to servers that
  send uncompressed bitmaps with non-aligned row widths, such as XRDP
  at 16 bpp.

- Skip bitmap updates that exceed bounds ([#1146](https://github.com/Devolutions/IronRDP/issues/1146)) ([2b97a95e6d](https://github.com/Devolutions/IronRDP/commit/2b97a95e6da8833e8a84e9f42960da91eee87cd6)) 

  After a desktop resize, an RDP server can send a burst of bitmap updates
  for the old resolution before its rendering pipeline has fully
  transitioned to the new one. These updates reference coordinates beyond
  the current image buffer in `DecodedImage`, causing index-out-of-bounds
  panics in the `apply_*` methods. On the server side, the same stale
  bitmaps can reach the encoder with dimensions exceeding the negotiated
  desktop size, panicking in `NoneHandler::handle()`.
  
  This commit adds bounds checks at two levels:
  - `DecodedImage::rect_fits()` guard at the entry of each `apply_*`
  method, returning an empty rectangle when the update doesn't fit
  - Encoder-level guard in `EncoderIter::next()` that drops
  `BitmapUpdate`s exceeding the current desktop size

- Propagate negotiated share_id to all outgoing ShareDataPdu ([#1147](https://github.com/Devolutions/IronRDP/issues/1147)) ([2b24e9664d](https://github.com/Devolutions/IronRDP/commit/2b24e9664dd05620ff63a24d092377477fdde863)) 

## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.7.0...ironrdp-session-v0.8.0)] - 2025-12-18


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.5.0...ironrdp-session-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a similar size
  of PNG, while offering 20x-50x faster encoding and 3x-4x faster decoding.

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec for SetSurface command. The PDU data contains the same
  data as the QOI codec, with zstd compression.

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.4.0...ironrdp-session-v0.4.1)] - 2025-06-27

### <!-- 1 -->Features

- More functions on `ActiveStage` (#791) ([5482365655](https://github.com/Devolutions/IronRDP/commit/5482365655e5c171cd967eda401b01161a9f6602)) 
  - `get_dvc_by_channel_id`
  - `encode_dvc_messages`


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.3.0...ironrdp-session-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- [**breaking**] Make DecodedImage Send ([45f66117ba](https://github.com/Devolutions/IronRDP/commit/45f66117ba05170d95b21ec7d97017b44b954f28)) 

- Add DecodeImage helpers ([cd7a60ba45](https://github.com/Devolutions/IronRDP/commit/cd7a60ba45a0241be4ecf3860ec4f82b431a7ce2)) 

### <!-- 4 -->Bug Fixes

- Update rectangle when applying None codecs updates (#728) ([a50cd643dc](https://github.com/Devolutions/IronRDP/commit/a50cd643dce9621f314231b7598d2fd31e4718c6)) 

- Return the correct updated region ([7507a152f1](https://github.com/Devolutions/IronRDP/commit/7507a152f14db594e4067bbc01e243cfba77770f)) 

  "update_rectangle" is set to empty(). The surface updates are then added
  by "union". But a union with an empty rectangle at (0,0) is still a
  rectangle at (0,0). We end up with big region updates rooted at (0,0)...

- Decrease verbosity of Rfx frame_index ([b31b99eafb](https://github.com/Devolutions/IronRDP/commit/b31b99eafb0aac2a5e5a610af21a4027ae5cd698)) 

- Decrease verbosity of FastPath header ([f9b6992e74](https://github.com/Devolutions/IronRDP/commit/f9b6992e74abb929f3001e76abaff5d7215e1cb4)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.3...ironrdp-session-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.2...ironrdp-session-v0.2.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.1...ironrdp-session-v0.2.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.0...ironrdp-session-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
