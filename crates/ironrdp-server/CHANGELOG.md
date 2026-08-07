# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.14.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.13.0...ironrdp-server-v0.14.0)] - 2026-08-07

### <!-- 0 -->Security

- Send the Server Auto-Reconnect Cookie during logon ([#1405](https://github.com/Devolutions/IronRDP/issues/1405)) ([7d35d65248](https://github.com/Devolutions/IronRDP/commit/7d35d6524886ab4e9f610b4b70566eaa21ea8177)) 

  ## What
  
  Adds an optional **Server Auto-Reconnect Cookie** (MS-RDPBCGR 2.2.4.3
  `ARC_SC_PRIVATE_PACKET`) to `ironrdp-server`. When set, `RdpServer`
  sends a Save Session Info PDU (`LogonExtended` +
  `AUTO_RECONNECT_COOKIE`) carrying it once per connection, right after
  activation (Confirm Active processed, encoder built), on the IO channel.
  
  ## Why
  
  A client only enters its **automatic reconnection sequence** (MS-RDPBCGR
  1.3.1.5) on an *ungraceful* disconnect if it was handed this cookie
  during logon. Without it, a dropped connection just reports as
  disconnected — **mstsc in particular won't auto-reconnect at all**.
  Today `ironrdp-server` never sends the cookie, so there's no way for a
  server to opt into that behavior.
  
  The concrete use case: a server that *intentionally* drops a connection
  and expects the client to come straight back — e.g. a recovery path that
  cycles the session — currently forces the user to reconnect by hand.
  With the cookie provisioned, the client re-establishes on its own
  (re-authenticating via NLA/CredSSP from cached credentials). It's also
  just standard behavior a real RDP server provides.
  
  ## API
  
  Mirrors the existing `credential_validator` pattern exactly — a builder
  method plus a runtime setter:
  
  ```rust
  // build time
  RdpServer::builder()
      .with_auto_reconnect_cookie(Some(ServerAutoReconnect { logon_id, random_bits }))
      // ...
  
  // or dynamically
  server.set_auto_reconnect_cookie(Some(cookie));
  ```
  
  `ServerAutoReconnect` (already public in
  `ironrdp-pdu::rdp::session_info`) carries a `logon_id` and a 16-byte
  `random_bits` (generate from a CSPRNG). A per-connection guard
  (`auto_reconnect_sent`, reset in `run_connection_with`) sends it exactly
  once — not again on a Deactivation-Reactivation.
  
  ## Scope / additive
  
  - **Additive, non-breaking.** Default is `None` (send nothing); existing
  servers are byte-for-byte unaffected.
  - All PDU types already exist in `ironrdp-pdu`
  (`rdp::session_info::{SaveSessionInfoPdu, LogonInfoExtended,
  LogonExFlags, ServerAutoReconnect, InfoType, InfoData}`) — this is
  purely wiring the server-side send.
  - Reuses the existing `encode_share_data_pdu` helper.
  
  ## Design point for review — the returning cookie
  
  This PR implements the **send** side only: it *enables* the client's
  automatic reconnection. It does **not** validate the
  `ARC_CS_PRIVATE_PACKET` the client sends back on reconnect (MS-RDPBCGR
  2.2.4.4). For a server that re-authenticates every connection by other
  means (NLA/CredSSP) that's sufficient and safe, and it's documented as
  such on the setter. If you'd prefer the crate to also offer *validation*
  of the returning cookie (so it can be an authentication factor — the
  server would store issued `(logon_id, random_bits)` and verify the
  client's `SecurityData`/`ARC_CS` on the next connect), I'm happy to do
  that as a follow-up, or fold it in here — it's a larger, stateful
  feature so I kept this PR to the send path. Let me know which you'd
  prefer.
  
  Built + `clippy --features egfx -D warnings` clean; tests compile.
  
  ---------

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

### <!-- 1 -->Features

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

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b)) 

### Refactor

- [**breaking**] Take auto-detect timestamps from the caller ([#1487](https://github.com/Devolutions/IronRDP/issues/1487)) ([f614e4acfc](https://github.com/Devolutions/IronRDP/commit/f614e4acfcb8abf7113e0f5c653112af94ed80af)) 

  ## Summary
  
  - `AutoDetectManager` read the clock itself: `std::time::Instant` in
  `pending_probes`, `Instant::now()` in `send_rtt_request`, and
  `Instant::elapsed()` in `handle_response` and `expire_stale_probes`.
  - It now takes `now_ms`, a caller-supplied monotonic millisecond counter
  whose epoch is arbitrary as long as it's consistent across calls.
  `ironrdp-server` supplies it from a process-wide monotonic origin in
  `server.rs`, so the clock lives in the I/O driver rather than in the
  state machine.
  
  ## Why
  
  - **Testability.** The RTT assertions were wall-clock dependent.
  `snapshot_reflects_measurements` could only check that the average came
  out under an arbitrary 100 ms bound, which almost any bug would satisfy.
  It now supplies both timestamps and asserts exact values: samples of 10,
  20 and 30 ms giving min 10, max 30, average 20.
  - **Portability.** `std::time::Instant::now` panics on
  `wasm32-unknown-unknown`, so a type that reads it internally can't be
  reused from a WASM build.
  - **Layering.** A state machine that reads ambient time can't satisfy
  the no-I/O rule the Core Tier crates follow, which forecloses moving
  this code in that direction later.
  
  Also covers the edges of the arithmetic the injected clock exposes.
  There are two `saturating_sub` sites and they saturate in opposite
  directions:
  
  - `handle_response`: a clock that ran backwards between request and
  response yields a zero sample rather than a wrapped value near
  `u32::MAX`, and the zero reaches the sample window, not just the return
  value.
  - `expire_stale_probes`: the same backwards clock makes the age zero,
  which is below any maximum, so the probe stays pending. Wrapping would
  make it look older than any limit and drop a probe whose response is
  still in flight.
  
  A third test covers the `u32::try_from(..).unwrap_or(u32::MAX)` on the
  first of those lines, where a gap wider than about 49.7 days clamps
  rather than truncating to the low 32 bits.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  Each of the three new tests was checked against a mutation of the code
  it guards rather than only for passing: the two backwards-clock tests
  fail if either `saturating_sub` becomes `wrapping_sub`, and the clamp
  test fails if the `try_from` becomes a truncating `as u32`, which
  reports `Some(0)` for a 49.7 day gap.
  
  ## Notes
  
  - Came out of the discussion on #1465, where the same question arises on
  the connector side. `ironrdp-connector` reads no clock at all today, and
  answering a connect-time Bandwidth Measure properly needs one; taking
  the timestamp from the caller is the shape that works on every target.
  - `AutoDetectManager` arrived in #1177 with the internal clock, so this
  corrects code I wrote rather than anyone else's.



## [[0.13.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.12.0...ironrdp-server-v0.13.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 1 -->Features

- Expose NetworkAutoDetect RTT via a shared handle ([#1346](https://github.com/Devolutions/IronRDP/issues/1346)) ([481ea5d161](https://github.com/Devolutions/IronRDP/commit/481ea5d161964b06a08f0b1ace0a1efd11773b4a)) 

  Exposes the server’s NetworkAutoDetect RTT measurement via a shared Arc<AtomicU32> handle so display backends can read a fresh RTT value even after run() takes ownership of the server.

- Dispatch initiate_file_copy via ClipboardMessage ([#1388](https://github.com/Devolutions/IronRDP/issues/1388)) ([b6325f9ea6](https://github.com/Devolutions/IronRDP/commit/b6325f9ea6900a84643b4415f9ebc7b1010cf3cd)) 

  Extends the CLIPRDR backend-facing API to properly support offering clipboard file lists (so later FileContentsRequests can be serviced) by introducing ClipboardMessage::SendInitiateFileCopy(Vec<FileDescriptor>) and wiring it through the in-tree ClipboardMessage dispatchers.

- Honor the client-requested desktop size ([#1373](https://github.com/Devolutions/IronRDP/issues/1373)) ([d471bd066f](https://github.com/Devolutions/IronRDP/commit/d471bd066f303df22f4767801fd97ecdbf527869)) 

  Adds an opt-in server/acceptor knob to negotiate the RDP session desktop size using the client’s originally requested resolution (from GCC Client Core Data) so the server can start at the client’s native size without a Deactivation–Reactivation resize round trip.

- Accept connections with TLS terminated at a lower layer ([#1281](https://github.com/Devolutions/IronRDP/issues/1281)) ([18bf75c7b3](https://github.com/Devolutions/IronRDP/commit/18bf75c7b3442881b42ee79b5f530ca97ab391ed)) 

  Adds a way to run a single RDP connection over a byte stream whose
  confidentiality is already provided by the embedder's transport, rather
  than having ironrdp-server perform the inner TLS handshake itself when
  X.224 selects PROTOCOL_SSL.



## [[0.12.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.11.0...ironrdp-server-v0.12.0)] - 2026-06-05

### <!-- 1 -->Features

- Opt-in support for NSCodec via feature flag ([#1332](https://github.com/Devolutions/IronRDP/issues/1332)) ([54af8f677f](https://github.com/Devolutions/IronRDP/commit/54af8f677fde726e2734f7bb1b451f3099d63532)) 

  Adds an opt-in implementation of the legacy RDP NSCodec encoder as a standalone crate, and wires it into `ironrdp-server` behind a feature flag so servers can serve NSCodec-only clients (notably macOS Microsoft Remote Desktop / Windows App) without default-build behavior changes.

- Add CredentialValidator trait for server-side auth ([#1172](https://github.com/Devolutions/IronRDP/issues/1172)) ([8a3b126396](https://github.com/Devolutions/IronRDP/commit/8a3b12639632f58291442a292a89fc6e22f82985)) 

### <!-- 4 -->Bug Fixes

- Emit RGB-channel QOI for opaque captures so ironrdp-session can decode ([#1335](https://github.com/Devolutions/IronRDP/issues/1335)) ([8a9ee6268c](https://github.com/Devolutions/IronRDP/commit/8a9ee6268ccdb5704c2bb60bed6d2adf57761427)) 

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-displaycontrol`, `ironrdp-dvc`, and `ironrdp-echo` public dependencies



## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.10.0...ironrdp-server-v0.11.0)] - 2026-06-01

### <!-- 1 -->Features

- Add clipboard data locking methods ([#1064](https://github.com/Devolutions/IronRDP/issues/1064)) ([58c3df84bb](https://github.com/Devolutions/IronRDP/commit/58c3df84bb9cafc8669315834cead35a71483c34)) 

  Per MS-RDPECLIP sections 2.2.4.6 and 2.2.4.7, the Local
  Clipboard Owner may lock the Shared Clipboard Owner's clipboard data before
  requesting file contents to ensure data stability during multi-request transfers.
  
  This enables server implementations to safely request file data from
  clients when handling clipboard paste operations.

- Add request_file_contents method ([#1065](https://github.com/Devolutions/IronRDP/issues/1065)) ([c30fc35a28](https://github.com/Devolutions/IronRDP/commit/c30fc35a28d6218603c1662e98e8b3053bea3aa5)) 

  Per MS-RDPECLIP section 2.2.5.3, the Local Clipboard Owner
  sends File Contents Request PDU to retrieve file data from the Shared
  Clipboard Owner during paste operations.
  
  This enables server implementations to request file contents from
  clients, completing the bidirectional file transfer capability.

- Add SendFileContentsResponse message variant ([#1066](https://github.com/Devolutions/IronRDP/issues/1066)) ([25f81337aa](https://github.com/Devolutions/IronRDP/commit/25f81337aa494af9a21f55f12ec27fd946465cbe)) 

  Adds `SendFileContentsResponse` to `ClipboardMessage` enum, enabling
  clipboard backends to signal when file data is ready to send via
  `submit_file_contents()`.
  
  This provides the message-based interface pattern used consistently by
  server implementations for clipboard operations.

- Expose client display size to RdpServerDisplay ([#1083](https://github.com/Devolutions/IronRDP/issues/1083)) ([3cf570788d](https://github.com/Devolutions/IronRDP/commit/3cf570788d418ef0d83670c8581ddb61582237fe)) 

  This allows the server implementation to handle the requested initial
  client display size. The default implementation simply returns
  `self.size()` so there's no change to existing behavior.
  
  Note that this method is also called during reactivations.

- Add EGFX server integration with DVC bridge ([#1099](https://github.com/Devolutions/IronRDP/issues/1099)) ([4ba696c266](https://github.com/Devolutions/IronRDP/commit/4ba696c266c7065c93a691b9f818644fd471429b)) 

- Implement ECHO virtual channel ([#1109](https://github.com/Devolutions/IronRDP/issues/1109)) ([6f6496ad29](https://github.com/Devolutions/IronRDP/commit/6f6496ad29395099563d50417d6dfff623914ee6)) 

- Make run_connection generic over stream type ([#1181](https://github.com/Devolutions/IronRDP/issues/1181)) ([c30d853fa3](https://github.com/Devolutions/IronRDP/commit/c30d853fa34c2da02047b1dcb626f1009de2b61c)) 

  Generalizes `RdpServer::run_connection` to accept arbitrary Tokio `AsyncRead + AsyncWrite` streams instead of a concrete `TcpStream`, enabling non-TCP transports (e.g., Unix sockets, VSOCK, in-process streams) to reuse the same server connection logic.

- Add auto-detect RTT measurement ([#1177](https://github.com/Devolutions/IronRDP/issues/1177)) ([2515470fdb](https://github.com/Devolutions/IronRDP/commit/2515470fdb7187d20ee3fba8244b839efa4cbce4)) 

  Adds server-side RTT measurement using the protocol-standard auto-detect
  mechanism (MS-RDPBCGR 2.2.14).

- IPv6 dual-stack and SO_REUSEADDR for run() ([#1187](https://github.com/Devolutions/IronRDP/issues/1187)) ([f10625cc80](https://github.com/Devolutions/IronRDP/commit/f10625cc806cc0ea9128c711df0dfd3ba8456b4f)) 

- Add ConnectionHandler trait for connection lifecycle hooks ([#1194](https://github.com/Devolutions/IronRDP/issues/1194)) ([5c08c7fe3d](https://github.com/Devolutions/IronRDP/commit/5c08c7fe3ded6f645cbddc53cdc0a02e8c45a037)) 

- Implement clipboard file transfer support ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9)) 

  Add end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel per MS-RDPECLIP.

- Handle SuppressOutput / RefreshRectangle and expose state ([#1319](https://github.com/Devolutions/IronRDP/issues/1319)) ([aa7ff679b9](https://github.com/Devolutions/IronRDP/commit/aa7ff679b914dbbc9bfe137d7f4f26bea30d6323)) 

- Add pointer caching support to ironrdp-server ([1a6b4206d5](https://github.com/Devolutions/IronRDP/commit/1a6b4206d5f0fe3333da721adeaea3f7d2aa65cf)) 

### <!-- 4 -->Bug Fixes

- Make MultifragmentUpdate max_request_size configurable ([#1100](https://github.com/Devolutions/IronRDP/issues/1100)) ([d437b7e0b9](https://github.com/Devolutions/IronRDP/commit/d437b7e0b9a47f5b9246e24c76554df82f47670e)) 

  The hardcoded `max_request_size` of 16,777,215 in the server's
  MultifragmentUpdate capability causes mstsc to reject the connection (it
  likely tries to allocate that buffer upfront). FreeRDP hit the same
  problem and adjusted their value in FreeRDP/FreeRDP#1313.
  
  This adds a configurable `max_request_size` field to `RdpServerOptions`
  with a default of 8 MB (matching what `ironrdp-connector` already uses
  on the client side) and exposes it through the builder via
  `with_max_request_size()`.

- Tile bitmaps that exceed `MultifragmentUpdate` limit ([#1133](https://github.com/Devolutions/IronRDP/issues/1133)) ([db2f40b5b0](https://github.com/Devolutions/IronRDP/commit/db2f40b5b0af66a4c83e0e075e2814467c060b1d)) 

  Split oversized dirty rects into horizontal strips that fit within `max_request_size`
  before handing them to the bitmap encoder.

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

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

- Keep newest queued waves on per-batch overflow ([#1276](https://github.com/Devolutions/IronRDP/issues/1276)) ([6e8479763f](https://github.com/Devolutions/IronRDP/commit/6e8479763f2bcf0938bd4091e35fd5a322a787dd)) 

- Drop raw user_data dump from McsMessage::SendDataRequest debug log ([#1295](https://github.com/Devolutions/IronRDP/issues/1295)) ([424590ac76](https://github.com/Devolutions/IronRDP/commit/424590ac76f3f82de19b3d6d1aa7a0119f616fab)) 

### <!-- 7 -->Build

- Bump rayon from 1.11.0 to 1.12.0 ([#1235](https://github.com/Devolutions/IronRDP/issues/1235)) ([a5dab356e5](https://github.com/Devolutions/IronRDP/commit/a5dab356e5bc29cde2fdcd71b6d11fdf38a96a9f)) 


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.9.0...ironrdp-server-v0.10.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- Send TLS close_notify during graceful RDP disconnect ([#1032](https://github.com/Devolutions/IronRDP/issues/1032)) ([a70e01d9c5](https://github.com/Devolutions/IronRDP/commit/a70e01d9c5675a7dffd65eda7428537c8ad6a857)) 

  Add support for sending a proper TLS close_notify message when the RDP
  client initiates a graceful disconnect PDU.

## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.8.0...ironrdp-server-v0.9.0)] - 2025-09-24

### <!-- 4 -->Bug Fixes

- [**breaking**] RdpServerDisplayUpdates::next_update now returns a Result

## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.7.0...ironrdp-server-v0.8.0)] - 2025-08-29

### <!-- 1 -->Features

- [**breaking**] Add server_codecs_capabilities() ([d3aaa43c23](https://github.com/Devolutions/IronRDP/commit/d3aaa43c23b252077b8720bb8ecfeceaaf7b7a7f)) 

  Teach the server to support customizable codecs set. Use the same
  logic/parsing as the client codecs configuration.
  
  Replace "with_remote_fx" with "codecs".

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a similar size
  of PNG, while offering 20x-50x faster encoding and 3x-4x faster decoding.

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec for SetSurface command. The PDU data contains the same
  data as the QOI codec, with zstd compression.

## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.6.1...ironrdp-server-v0.7.0)] - 2025-07-08

### Build

- Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.5.0...ironrdp-server-v0.6.0)] - 2025-05-27

### <!-- 1 -->Features

- Add stride debug info ([7f57817805](https://github.com/Devolutions/IronRDP/commit/7f578178056282e590179a10cd1eedb8f4d9ad63)) 

- Add Framebuffer helper struct ([1e87961d16](https://github.com/Devolutions/IronRDP/commit/1e87961d1611ed31f58b407f208295c97c0d2944)) 

  This will hold the updated bitmap data for the whole framebuffer.

- Add BitmapUpdate::sub() ([a76e84d459](https://github.com/Devolutions/IronRDP/commit/a76e84d45927d61e21c27abcfa31c4f0c7a17bbf)) 

- Implement some Encoder Debug ([137d91ae7a](https://github.com/Devolutions/IronRDP/commit/137d91ae7a096170ada289d420785c8f5de0663b)) 

- Keep last full-frame/desktop update ([aeb1193674](https://github.com/Devolutions/IronRDP/commit/aeb1193674641846ae1873def8c84a62a59213d5)) 

  It should reflect client drawing state.
  
  In following changes, we will fix it to draw bitmap updates on it, to
  keep it up to date.

- Find and send the damaged tiles ([fb3769c4a7](https://github.com/Devolutions/IronRDP/commit/fb3769c4a7fce56e340df8c4b19f7d90cda93e50)) 

  Keep a framebuffer and tile-diff against it, to save from
  encoding/sending the same bitmap data regions.

### <!-- 4 -->Bug Fixes

- Use desktop size for RFX channel size (#756) ([806f1d7694](https://github.com/Devolutions/IronRDP/commit/806f1d7694313b1a59842af300a437ae2f6c2463)) 

- [**breaking**] Remove time_warn! from the public API (#773) ([cc78b1e3dc](https://github.com/Devolutions/IronRDP/commit/cc78b1e3dc1c554dd3fcf6494763caa00ba28ad7)) 

  This is intended to be an internal macro.

### Refactor

- [**breaking**] Drop support for pixelOrder ([db6f4cdb7f](https://github.com/Devolutions/IronRDP/commit/db6f4cdb7f379713979b930e8e1fa1a813ebecc4)) 

  Dealing with multiple formats is sufficiently annoying, there isn't much
  need for awkward image layout. This was done for efficiency reason for
  bitmap encoding, but bitmap is really inefficient anyway and very few
  servers will actually provide bottom to top images (except with GL/GPU
  textures, but this is not in scope yet).

- [**breaking**] Use bytes, allowing shareable bitmap data ([3c43fdda76](https://github.com/Devolutions/IronRDP/commit/3c43fdda76f4ef6413db4010471364d6b1be2798)) 

- [**breaking**] Rename left/top -> x/y ([229070a435](https://github.com/Devolutions/IronRDP/commit/229070a43554927a01541052a819fe3fcd32a913)) 


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.4.2...ironrdp-server-v0.5.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.4.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.4.1...ironrdp-server-v0.4.2)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.4.0...ironrdp-server-v0.4.1)] - 2025-01-28

### <!-- 1 -->Features

- Advertize Bitmap::desktopResizeFlag ([a0fccf8d1a](https://github.com/Devolutions/IronRDP/commit/a0fccf8d1a3eeab6c73ed7d9cdbb4342cca173c4)) 

  This makes freerdp keep the flag up and handle desktop
  resize/deactivation-reactivation. It should be okay to advertize,
  if the server doesn't resize anyway, I guess.

- Add volume support (#641) ([a6c36511f6](https://github.com/Devolutions/IronRDP/commit/a6c36511f6584f67b8c6e795c34d5007ec2b24a4)) 

  Add server messages and API to support setting client volume.

### <!-- 4 -->Bug Fixes

- Drop unexpected PDUs during deactivation-reactivation ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

  The current behaviour of handling unmatched PDUs in fn read_by_hint()
  isn't good enough. An unexpected PDUs may be received and fail to be
  decoded during Acceptor::step().
  
  Change the code to simply drop unexpected PDUs (as opposed to attempting
  to replay the unmatched leftover, which isn't clearly needed)

- Reattach existing channels ([c4587b537c](https://github.com/Devolutions/IronRDP/commit/c4587b537c7c0a148e11bc365bc3df88e2c92312)) 

  I couldn't find any explicit behaviour described in the specification,
  but apparently, we must just keep the channel state as they were during
  reactivation. This fixes various state issues during client resize.

- Do not restart static channels on reactivation ([82c7c2f5b0](https://github.com/Devolutions/IronRDP/commit/82c7c2f5b08c44b1a4f6b04c13ad24d9e2ffa371)) 

- Check client size ([0f9877ad39](https://github.com/Devolutions/IronRDP/commit/0f9877ad3901b37f58406095e05f345fbc8a5eaa)) 

  It's problematic when the client didn't resize, as we send bitmap
  updates that don't fit. The client will likely drop the connection.
  Let's have a warning for this case in the server.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.3.1...ironrdp-server-v0.4.0)] - 2024-12-17

### <!-- 1 -->Features

- [**breaking**] Make TlsIdentityCtx accept PEM files ([#623](https://github.com/Devolutions/IronRDP/pull/623)) ([9198284263](https://github.com/Devolutions/IronRDP/commit/9198284263e11706fed76310f796200b75111126)) 

  This is in general more convenient than DER files.

  This patch also includes a breaking change in the public API. 
  The `cert` field in the `TlsIdentityCtx` struct is replaced by a `certs` field containing multiple `CertificateDer` items.

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.3.0...ironrdp-server-v0.3.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
