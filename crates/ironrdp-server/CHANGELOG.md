# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.14.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.13.0...ironrdp-server-v0.14.0)] - 2026-09-01

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

- Send periodic Heartbeat PDUs on the message channel ([#1842](https://github.com/Devolutions/IronRDP/issues/1842)) ([8f57691a9a](https://github.com/Devolutions/IronRDP/commit/8f57691a9a6e497388dc2824bffe93eeed7bb698)) 

  The server never sends Server Heartbeat PDUs (MS-RDPBCGR 2.2.16.1), even
  though the client half of the feature is in place: HeartbeatPdu
  encode/decode landed with #1814 and clients decode and ignore them. This
  adds the send half, opt-in.

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

- Add static-channel factories ([#1633](https://github.com/Devolutions/IronRDP/issues/1633)) ([e48b29c017](https://github.com/Devolutions/IronRDP/commit/e48b29c0173096c1718c9f657bed3a59926aa97c)) 

  Create fresh static-channel processors before GCC negotiation, expose
  the acceptor type needed to configure them, and exercise RDPDR
  initialization and drive I/O end to end.
  
  ---------

- Measure and report network characteristics ([#1470](https://github.com/Devolutions/IronRDP/issues/1470)) ([224e8db7ce](https://github.com/Devolutions/IronRDP/commit/224e8db7cec2dad39032aac097f550a6600e742c)) 

  ## Summary
  
  - The server measures round-trip time and bandwidth from the continuous
    auto-detect exchange and reports both to the client in a Network
  Characteristics Result on the MCS message channel ([MS-RDPBCGR]
  2.2.14.1.5).
  - Nothing is sent until both are known. A result carrying RTT alone
  reports part
  of the picture as though it were the whole, which is what krdp and grd
  avoid by
    returning early when they have no bandwidth figure.
  - The result is paced at one per second on its own clock rather than
  once per
  probe, and is withheld unless a client response has arrived since the
  last one.
  A client that stops answering stops producing results instead of leaving
  the
    last window values advertised indefinitely.
  - `baseRTT` is the lowest RTT seen over the session, per 2.2.14.1.5's
  "lowest
    detected round-trip time". `averageRTT` is the window average, so the
    difference between them is queueing delay.
  
  ## Validation
  
  - `cargo xtask check fmt/lints/tests/typos/locks` and `cargo xtask wasm
  check`
    all pass.
  - `cargo semver-checks -p ironrdp-server --baseline-rev master`: no
  update
    required.
  - Tests live in `ironrdp-testsuite-core`. `ironrdp-server` sets
  `[lib] test = false`, so an inline module would compile and never run.
  Each new
  test was checked by planting the corresponding regression and confirming
  it
    fails.
  
  ## Notes
  
  - Supersedes #1471, which added bandwidth as a follow-up. With the gate
  above,
  this PR alone could only ever emit the form it now declines to send, so
  the two
    are one change.
  - #1487 has merged, so the earlier dependency note no longer applies.

- Make the RemoteFX quantization table configurable ([#1685](https://github.com/Devolutions/IronRDP/issues/1685)) ([925e7c0f7c](https://github.com/Devolutions/IronRDP/commit/925e7c0f7cb7ae937a92ec93d4cb758289594cc0)) 

  Add Quant::try_new(), a validating constructor that rejects any subband
  value outside the 6..=15 range, and
  RdpServerBuilder::with_remotefx_quant(quant: Quant), wiring it through
  RdpServerOptions to the same capability-negotiation call site #1684
  touched.
  
  Per [MS-RDPRFX] 2.2.2.1.5, each of the 10 TS_RFX_CODEC_QUANT values is a
  4-bit field, and the legal range is 6 to 15. #1557 reports the quant
  table is hardcoded to Quant::default(); this gives callers a validated
  way to set it instead.
  
  Builds on #1684, which added the storage this PR wires up. Default
  behavior is unchanged: with_remotefx_quant is opt-in, and a server that
  doesn't call it still gets Quant::default(), the same values Windows RDP
  servers send.

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

- Expose session-lifetime baseline RTT to the embedder ([#1737](https://github.com/Devolutions/IronRDP/issues/1737)) ([ac7800c989](https://github.com/Devolutions/IronRDP/commit/ac7800c989dd5926eef2a1adf983458ad0aec9ee)) 

  ## Summary
  
  - RttSnapshot.min_ms is a sliding-window low that can rise as low
  samples
    age out of the window, and is explicitly documented as not baseRTT per
    MS-RDPBCGR 2.2.14.1.5, which defines baseRTT as the session-lifetime
    lowest. AutoDetectManager already tracks that true low internally as
    min_rtt_ms for the wire NetworkCharacteristicsResult, but nothing
    exposed it as its own value: an embedder reading snapshot() or
    autodetect_rtt_handle() alone cannot derive averageRTT - baseRTT as a
    queueing-delay signal, since that relationship only holds when the
    floor cannot rise.
  - Adds AutoDetectManager::baseline_rtt_ms() as the public getter for the
    existing field, and mirrors the autodetect_rtt_handle plumbing on
    RdpServer: a new autodetect_baseline_rtt: Arc<AtomicU32> field,
    autodetect_baseline_rtt_handle() accessor, and
    with_autodetect_baseline_rtt_handle() builder method.
  - A matched RTT sample always updates min_rtt_ms in the same
    handle_response call that returns it, so the store site reads the new
    getter unconditionally right after storing the RTT sample, with no
    before/after comparison needed (unlike the bandwidth case in #1734,
    where the underlying value can be cleared to None on an unusable
    measurement).
  - Adds a manager-level test pinning the session-low-not-window-low
    property on the new getter directly, plus the usual pair of
    handle-plumbing tests (sentinel default, injected-handle round trip).
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass, including the
  3 new tests.
  
  ## Notes
  
  No public API break: `RdpServer::new` is crate-private, and the public
  surface (`RdpServerBuilder`) only gains an additive optional field and a
  new method.

- [**breaking**] Give mouse button events a position, fix X1/X2 and middle-button/hwheel handling ([#1769](https://github.com/Devolutions/IronRDP/issues/1769)) ([e9c5b1c54a](https://github.com/Devolutions/IronRDP/commit/e9c5b1c54a1153e5d7116d7fadf636bd551020ea)) 

  ## Summary
  
  - Fixes #1466: `MouseEvent`'s button variants were position-less, so
  `From<MousePdu>` and `From<MouseXPdu>` silently discarded the wire PDU's
  x/y whenever a button flag was set (position only survived for a pure
  Move). A client that sends a tap as a single button PDU with no
  preceding Move (confirmed: Windows App on iOS/iPadOS, touch mode) clicks
  at the stale cursor position rather than where it tapped.
  - While fixing that, found two more bugs in the same conversion layer,
  both confirmed against MS-RDPBCGR: `From<MousePdu>` never checked
  `MIDDLE_BUTTON_OR_WHEEL` or `HORIZONTAL_WHEEL`, so middle-click and
  horizontal wheel silently fell through to the Move fallback.
  `From<MouseXPdu>` mapped `PointerXFlags::BUTTON1`/`BUTTON2` to
  Left/Right, but per 2.2.8.1.2.2.4 these are "Extended mouse button 1
  (also referred to as button 4)" and "...2 (...button 5)", the X1/X2 side
  buttons. `MouseRelPdu`'s own sibling `From` impl already distinguishes
  these correctly via `XBUTTON1`/`XBUTTON2`, so this was an inconsistency,
  not a deliberate choice.
  - Checked FreeRDP, xrdp, KDE krdp, and GNOME Remote Desktop before
  settling on a shape. All four check explicit MOVE flags rather than
  falling through on else, all four handle middle button and both wheel
  flags, none conflate X1/X2 with primary buttons. xrdp's own source has a
  maintainer comment confirming real clients exercise the HWHEEL gap: "As
  mstsc does MOUSE not MOUSEX for horizontal scrolling, PTRFLAGS_HWHEEL
  must be handled here."
  - `MouseEvent::Button { x, y, button, pressed }` replaces the ten
  position-less `LeftPressed`/`LeftReleased`/.../`Button5Released`
  variants, scoped to the PDU types that actually carry absolute position
  (`MousePdu`, `MouseXPdu`, `ainput::MousePdu`, which already carried x/y
  on every event but never applied it to buttons either). `MouseRelPdu`
  gets `MouseEvent::ButtonRel { button, pressed }` instead, since it only
  has relative deltas. Both share a new `MouseButton` enum
  (`Left`/`Right`/`Middle`/`X1`/`X2`). Added
  `MouseEvent::HorizontalScroll` to pair with the existing
  `VerticalScroll`. Breaking anyway, so `MouseEvent` and `MouseButton` are
  both `#[non_exhaustive]`.
  - There's independent, unmerged prior art for the middle-button half of
  this on branch `probakowski/xrdp` (commit `e924272`, ad hoc xrdp-interop
  debugging, never opened as a PR) that arrived at the same
  `MIDDLE_BUTTON_OR_WHEEL` fix. Crediting it here since I found it while
  researching this.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  Single file, `crates/ironrdp-server/src/handler.rs`. Grepped the whole
  workspace for `MouseEvent::` usage; this file is the only one that
  constructs or matches it, no other call sites need updating for this PR.
  Downstream, I'm updating my own `lamco-rdp-input`/`lamco-rdp-server`
  consumers to the new shape as a follow-up.

- Wire MS-RDPEI server-side into ironrdp-server ([#1773](https://github.com/Devolutions/IronRDP/issues/1773)) ([aa260fef9e](https://github.com/Devolutions/IronRDP/commit/aa260fef9ebbc0a420b17e796616d710fb41e702)) 

  ## Summary
  
  - MS-RDPEI (multitouch and pen) got server-side plumbing in
  `ironrdp-rdpei` (the recent Input DVC PR), but nothing in
  `ironrdp-server` ever registers the `RdpeiServer`/`RdpeiHandler` it
  added. Grepped the whole crate for `rdpei` before starting: zero
  references. This wires it in.
  - Follows the exact shape of the existing
  `SoundServerFactory`/`with_sound_factory` (the simplest of the three
  existing opt-in factories, and RDPEI's needs are equally simple): a new
  `RdpeiServerFactory` trait, a `with_rdpei_factory` builder method, and
  registration in `attach_channels` alongside the other optional DVC
  factories. The factory builds and returns the whole configured
  `RdpeiServer`, not just a handler, so it can reach
  `with_protocol_version`/`with_supported_features` to advertise
  non-default capabilities.
  - Re-exports everything a downstream `RdpeiHandler` implementation needs
  from the crate root: `RdpeiServerFactory`, `RdpeiHandler`,
  `RdpeiServer`, and the touch/pen/ready PDU types plus the nested payload
  types their callbacks embed (`CsReadyFlags`, `PenContact` and its
  fields/flags, `PenFrame`, `TouchContactFields`/`TouchContactDataFlags`).
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Adds a
  regression test in `ironrdp-testsuite-core` (`ironrdp-server` has `[lib]
  test = false`, so an inline test would never run in CI) covering that a
  registered `RdpeiServerFactory` is invoked during per-connection channel
  setup.
  
  ## Notes
  
  Two crates: `ironrdp-server` (`Cargo.toml`, `Cargo.lock`, `builder.rs`,
  `lib.rs`, `server.rs`, the new `rdpei.rs`) and `ironrdp-testsuite-core`
  (`Cargo.toml`, `tests/server/mod.rs`, the new `tests/server/rdpei.rs`).
  `RdpServer::new` is `pub(crate)`, only ever called from the builder, so
  adding the new parameter there has no external API impact beyond the new
  builder method itself.

- Add rdpeusb server integration ([#1417](https://github.com/Devolutions/IronRDP/issues/1417)) ([4755faec5e](https://github.com/Devolutions/IronRDP/commit/4755faec5efb1649e2cb839f553c8aaae7534aad)) 

  Related discussion: #1516.
  
  *Currently blocked by:*
  - #1682 — `ironrdp-usb`, the protocol-independent USB layer
  - #1683 — `ironrdp-rdpeusb::usb`, the RDPEUSB adapter, which is also
  blocked by #1682
  
  > **Reviewing this PR:** because #1682 and #1683 have not merged yet,
  the *Files changed* tab shows the cumulative diff. The incremental diff
  for this PR alone is here:
  >
  https://github.com/uchouT/IronRDP/compare/ironrdp-rdpeusb-usb...ironrdp-server
  
  This PR carries the design rationale for all three, since it is the
  first place where the whole stack is visible at once.
  
  [qemu-rdp](https://gitlab.com/uchouT/qemu-display/-/tree/feat/usbredir)
  now redirects USB mass storage over RDP end to end, verified against a
  FreeRDP client built from source. These three PRs split the reusable
  part out into IronRDP, so that any RDP server can offer USB redirection
  instead of reimplementing the channel.
  
  ## The layering
  
  #1516 proposed splitting this into three layers. The implementation
  followed that split; this section records what each layer ended up
  owning.
  
  ```text
  ironrdp-usb        USB semantics: descriptors, control requests, transfers.
        ↑            Knows nothing about any transport.
  ironrdp-rdpeusb    RDPEUSB adapter: TS_URB forms, USBD conventions,
        ↑            capability negotiation, channel state.
  ironrdp-server     Server facade: request lifetime, cancellation, routing.
        ↑            Speaks ironrdp-usb types outward.
  application        A usbredir bridge, like qemu-rdp, macrdp, or any other consumer.
  ```
  
  The rule used to place a concept is: **it may live in a layer only if
  its contract can be explained using that layer's vocabulary alone** —
  USB, or MS-RDPEUSB, or a generic server facade. A concept that can only
  be justified by what one downstream consumer happens to need stays in
  that consumer.
  
  That rule is not free, so two worked examples:
  
  - `CompletionData` ([#1683](https://github.com/Devolutions/IronRDP/issues/1683)) and `InterfaceAlloc` (this PR) were both
  added because the server layer needed them, and both stayed in
  `ironrdp-rdpeusb` because both are explainable purely in RDPEUSB terms:
  one is the union of the four completion result types the channel
  defines, the other allocates the interface IDs RDPEUSB requires per
  redirected device. Being needed downstream was neither sufficient on its
  own nor disqualifying.
  - Going the other way: usbredir packet types and IDs, its
  synchronous/asynchronous ordering rules, its inflight scheduling, and
  its stream policy all stayed out of these crates, because none of them
  can be stated without naming usbredir.
  
  ## What the facade exposes
  
  Behind an optional `usb` feature:
  
  - **`DeviceFactory`** creates a per-device backend when the client opens
  a redirection channel; **`UsbRedirDevice`** receives the device
  announcement, device text, and channel close.
  - **`UsbDeviceHandle`** drives the device in `ironrdp-usb` terms:
  `get_descriptor`, `get_configuration`, `select_configuration`,
  `get_interface`, `select_interface`, `control_transfer`,
  `bulk_transfer`, `interrupt_transfer`, `isochronous_transfer`,
  `clear_halt`, `reset_device`, and `current_frame_number`, plus
  `query_device_text_request` and `retract_request`.
  - **`PendingRequest`**, **`PendingHandle`**, **`CompletionFut`**, and
  **`RawPending`** own the request lifetime: a caller can await a
  completion, hold a cancellation handle, or drop the request. Dropping a
  submitted request emits `CANCEL_REQUEST`, so an abandoned transfer does
  not stay in flight on the client.
  - **`RdpServerBuilder::with_usb_factory`** opts a server in. The channel
  is created only when the client negotiates the capability.
  
  No `TS_URB`, RDPEUSB processor, or `ServerEvent` wiring appears in any
  of those signatures.
  
  ## What stays in this layer, and why
  
  Request identifiers, the pending-request map, cancellation, device
  closure, and completion routing all live here rather than in
  `ironrdp-rdpeusb`. Keeping them out of the protocol crate is what lets
  #1683 stay sans-I/O and independently testable, and it means a second
  consumer of the facade gets the lifetime handling for free without
  inheriting RDPEUSB details.
  
  ## Also in this PR
  
  `refactor(rdpeusb): expose InterfaceAlloc at the crate root` —
  `InterfaceAlloc` was a private helper of `UrbdrcListener`, so a server
  driving the channel had no way to allocate the interface IDs it must
  assign to redirected devices. It is now public at the crate root, with
  the initial value expressed through `Default`. This is a pure
  relocation: the allocation range and the exhaustion behaviour are
  unchanged.
  
  ## TODOs
  
  Stated up front rather than left to be discovered:
  
  1. **The `usb` feature is not linted in CI.**. Locally, `cargo clippy -p
  ironrdp-server --features usb --all-targets -- -D warnings` is clean.
  Whether to add the feature to the CI matrix is a maintainer call, so I
  have not touched CI configuration here.
  2. **RDPEUSB types still reaches the consumer.**
  `RdpUsbDeviceAnnounceInfo` carries the raw `DeviceAnnounce`, so a
  consumer that wants the device speed reads `DeviceSpeed`, an RDPEUSB PDU
  type. The goal is for a downstream bridge to need no `ironrdp-rdpeusb`
  dependency at all. I would rather agree on the facade shape than guess
  at it, so it is left as a follow-up.
  
  ---------

- [**breaking**] Introduce typed ServerError on the public API boundary ([#1242](https://github.com/Devolutions/IronRDP/issues/1242)) ([b5b9558eae](https://github.com/Devolutions/IronRDP/commit/b5b9558eae164a3500ace069b4db12be25eba7bd)) 

  ## Summary
  
  First in a staged migration toward a typed public error story for
  `ironrdp-server`, addressing #1209. Introduces:
  
  ```rust
  pub type ServerError = ironrdp_error::Error<ServerErrorKind>;
  pub type ServerResult<T> = Result<T, ServerError>;
  ```
  
  `ServerErrorKind` is a `#[non_exhaustive]` enum with concretely typed
  variants (`Encode`, `Io`, `Channel`, `Unsupported`, `Reason`, `Custom`).
  Sources are attached through `ironrdp_error::Error::with_source` rather
  than embedded as `Box<dyn Error>` in variant data. This mirrors the
  shape of `ConnectorErrorKind` in `ironrdp-connector`, so the
  connection-management layer stays internally consistent.
  
  ## Why this shape
  
  The audit before drafting found that `ironrdp-connector`,
  `ironrdp-session`, `ironrdp-pdu`, `ironrdp-mstsgu`, and `ironrdp-core`
  (`EncodeError`/`DecodeError`) all use the same
  `ironrdp_error::Error<Kind>` pattern. The `thiserror`-based bare enums
  elsewhere in the workspace (`GccError`, `BulkError`, etc.) are
  leaf-layer errors at the PDU/codec level; the connection-management
  layer sister crate to `ironrdp-server` is the right template.
  
  ## Scope of this PR
  
  Public function signatures only:
  
  - `RdpServer::run` → `ServerResult<()>`
  - `RdpServer::run_connection<S>` → `ServerResult<()>`
  - `RdpServer::run_connection_with<S>` → `ServerResult<()>`
  - `TlsIdentityCtx::init_from_paths` → `ServerResult<Self>`
  - `TlsIdentityCtx::make_acceptor` → `ServerResult<TlsAcceptor>`
  - `EchoServerHandle::send_request` → `ServerResult<()>`
  
  Internal call sites continue to use `anyhow::Result`. A private
  `from_anyhow_with_context` helper bridges at the public boundary,
  tagging each call site with the operation that failed. The
  `ConnectionHandler::on_disconnected(error: Option<&anyhow::Error>)`
  parameter is unchanged in this PR.
  
  `run_connection_with` keeps its anyhow body in a private
  `run_connection_with_inner`, which the accept loop calls directly so
  `on_disconnected` still receives an `anyhow::Error`. That helper is
  transitional and its removal is inside this stack rather than deferred
  indefinitely: #1244 converts `on_disconnected` to `ServerError` and
  deletes it, with the accept loop calling `run_connection` from there.
  `TlsIdentityCtx::init_from_paths` and `make_acceptor` use the same
  wrapper-plus-private-body shape, matching the `run` / `run_inner` pair
  this PR already introduces.
  
  ## Companion ext traits
  
  ```rust
  pub trait ServerErrorExt {
      fn encode(error: EncodeError) -> Self;
      fn io(context: &'static str, error: io::Error) -> Self;
      fn channel(context: &'static str) -> Self;
      fn unsupported(context: &'static str) -> Self;
      fn reason(context: &'static str, reason: impl Into<String>) -> Self;
      fn custom<E>(context: &'static str, error: E) -> Self
      where E: core::error::Error + Sync + Send + 'static;
  }
  pub trait ServerResultExt {
      fn with_context(self, context: &'static str) -> Self;
  }
  ```
  
  Mirrors `ConnectorErrorExt` / `ConnectorResultExt`, trimmed to the
  constructors and the one result-side helper that have call sites in this
  stack (`decode` and `with_source` did not, so neither shipped).
  
  ## Stack ordering
  
  This is step 1 of 4. The full chain (must merge in PR-number order):
  
  | Step | PR | Scope |
  |------|-----|-------|
  | **1** | **this PR** | Add `ServerError` / `ServerErrorKind` / ext
  traits, convert 5 public functions, internal anyhow stays via private
  bridge |
  | 2 | #1243 | Convert encoder/helper/echo internals (anyhow → typed) |
  | 3 | #1244 | Convert server.rs internals + `on_disconnected` parameter
  (breaking) |
  | 4 | #1245 | Convert display traits, drop anyhow dep, finish (breaking)
  |
  
  All four close #1209 together. Each step is independently reviewable but
  is rebased onto its predecessor as a Stacked branch; please merge in the
  order above so the rebases land cleanly.
  
  ## Breaking change
  
  Marked with `!` in the conventional commit. Consumers of the five listed
  public functions need to update their `Result` types. Pre-1.0, and per
  @elmarco's note on #1209: "I am ok with breaking API at this point :)".
  
  ## Test plan
  
  - `cargo xtask check fmt -v` clean
  - `cargo xtask check lints -v` clean (workspace, all-targets, with
  helper + __bench features)
  - `cargo xtask check tests -v` passes
  - `cargo build --workspace --all-targets` clean (one example consumer in
  `crates/ironrdp/examples/server.rs` updated to convert at its own
  boundary)
  
  Closes part of #1209.

- Convert encoder/helper/echo internals from anyhow to ServerError ([#1243](https://github.com/Devolutions/IronRDP/issues/1243)) ([f25d7083e6](https://github.com/Devolutions/IronRDP/commit/f25d7083e662368c4667d5f27c3a489fb5866a5a)) 

  ## Summary
  
  Second step of the staged migration toward a typed public error story
  for `ironrdp-server` ([#1209](https://github.com/Devolutions/IronRDP/issues/1209)). Stacks on #1242.
  
  Replaces `anyhow` construction sites in modules whose internal flow does
  **not** pass through the `ConnectionHandler::on_disconnected` callback
  (which still takes `&anyhow::Error` and is the subject of #1244):
  
  - **`encoder/mod.rs`**: ~15 `anyhow!` / `.context()` / `bail!` sites
  converted to typed `ServerError` variants (`Encode`, `Reason`,
  `Custom`). `EncodeError` sources go through `ServerError::encode`
  (matching `ConnectorErrorExt::encode`). `spawn_blocking` `JoinError`,
  qoi codec errors, and zstd error codes go through `ServerError::custom`
  or `ServerError::reason` as appropriate.
  - **`helper.rs`**: TLS cert/key loading paths construct
  `ServerError::io` for `std::io::Error` sources, `ServerError::reason`
  for missing-key cases, `ServerError::custom` for `x509-cert` and PEM
  parsing errors. Removes the `from_anyhow` bridge and the inner-fn split
  introduced in #1242.
  - **`echo.rs`**: `build_echo_request` returns `ServerResult`, builds
  errors via `ServerError::custom` directly. `send_request` keeps its
  already-typed `Channel` and `Reason` variants.
  
  ## What this PR does NOT touch
  
  - `server.rs` internals (the heavy `.context()` chain in `run_inner` and
  `run_connection_inner`) stay on `anyhow` because they propagate into
  `ConnectionHandler::on_disconnected(error: Option<&anyhow::Error>)`.
  #1244 will (a) change that parameter to `Option<&ServerError>`, and (b)
  convert the server.rs internal sites in the same change.
  - `RdpServerDisplay` / `RdpServerDisplayUpdates` traits (which return
  `anyhow::Result`) stay unchanged. #1245 will convert those, drop the
  `anyhow` dependency entirely, and complete the migration.
  
  ## Why split this way
  
  `server.rs` and the display traits are both `anyhow`-flavored at the
  boundary; converting them in a single PR with `on_disconnected` keeps
  the type story coherent. Splitting them now would either require a
  temporary `anyhow ↔ ServerError` two-way bridge (ugly) or leave the
  public trait inconsistent with the rest of the crate (worse). Better to
  ship this batch (encoder + helper + echo all internal-only) and then
  take server.rs + on_disconnected as one coherent step.
  
  ## Stack ordering
  
  This is step 2 of 4. The full chain (must merge in PR-number order):
  
  | Step | PR | Scope |
  |------|-----|-------|
  | 1 | #1242 | Add `ServerError` / `ServerErrorKind` / ext traits,
  convert 5 public functions, internal anyhow stays via private bridge |
  | **2** | **this PR** | Convert encoder/helper/echo internals (anyhow →
  typed) |
  | 3 | #1244 | Convert server.rs internals + `on_disconnected` parameter
  (breaking) |
  | 4 | #1245 | Convert display traits, drop anyhow dep, finish (breaking)
  |
  
  Branch is stacked on `feat/server-typed-error` ([#1242](https://github.com/Devolutions/IronRDP/issues/1242)). Rebase onto
  master after #1242 merges.
  
  ## Test plan
  
  - `cargo xtask check fmt -v` clean
  - `cargo xtask check lints -v` clean (workspace, all-targets, with
  helper + __bench features)
  - `cargo xtask check tests -v` passes
  - `cargo build --workspace --all-targets` clean

- [**breaking**] Convert server.rs internals + on_disconnected to ServerError ([#1244](https://github.com/Devolutions/IronRDP/issues/1244)) ([b919a41932](https://github.com/Devolutions/IronRDP/commit/b919a419320dfabe4b932c0d3d623386883b072f)) 

  ## Summary
  
  Third step of the staged migration started in #1242 and continued in
  #1243. Combines the `on_disconnected` signature change with the
  `server.rs` internal site conversion since both touch anyhow-flowing
  code; doing them together avoids a temporary `anyhow ↔ ServerError`
  two-way bridge during the intermediate state.
  
  ## Public API change
  
  ```diff
   fn on_disconnected(
       &mut self,
       peer: SocketAddr,
       duration: Duration,
  -    error: Option<&anyhow::Error>,
  +    error: Option<&ServerError>,
   ) -> PostConnectionAction { ... }
  ```
  
  This is a **breaking change** for handler implementations of
  `ConnectionHandler`. Pre-1.0, and per @elmarco's note on #1209: "I am ok
  with breaking API at this point :)".
  
  ## Internal changes
  
  - The two `run_inner` / `run_connection_inner` wrapper functions
  introduced in #1242 are folded back into `run` / `run_connection`. The
  accept loop calls the public method directly; `result.as_ref().err()`
  now feeds the new `ServerError`-typed parameter naturally.
  - ~25 `.context()` / `bail!` sites in `run`, `run_connection`,
  `accept_finalize`, `handle_io_channel_data`, `handle_x224`,
  `handle_input_backlog`, and the `encode_share_data_pdu` /
  `deactivate_all` helpers replaced with typed `ServerError` variants.
  Pattern alignment with `ConnectorErrorExt`:
    - `EncodeError` sources → `ServerError::encode`
    - `DecodeError` sources → `ServerError::decode`
    - `std::io::Error` sources → `ServerError::io`
    - `Option<channel>` with `.ok_or_else` → `ServerError::channel`
  - `bail!("Fastpath output not supported!")` → `ServerError::unsupported`
    - everything else → `ServerError::custom` with a static context.
  
  ## What this PR does NOT touch
  
  - The `RdpServerDisplay::updates()` and
  `RdpServerDisplayUpdates::next_update()` trait methods still return
  `anyhow::Result`. Their conversion is #1245, which also drops the
  `anyhow` dependency entirely.
  - The `from_anyhow` private helper introduced in #1242 is retained only
  at one boundary: the `display.updates()` call site in `run_connection`
  wraps its anyhow result via `from_anyhow` until #1245 lands.
  
  ## Stack ordering
  
  This is step 3 of 4. The full chain (must merge in PR-number order):
  
  | Step | PR | Scope |
  |------|-----|-------|
  | 1 | #1242 | Add `ServerError` / `ServerErrorKind` / ext traits,
  convert 5 public functions, internal anyhow stays via private bridge |
  | 2 | #1243 | Convert encoder/helper/echo internals (anyhow → typed) |
  | **3** | **this PR** | Convert server.rs internals + `on_disconnected`
  parameter (breaking) |
  | 4 | #1245 | Convert display traits, drop anyhow dep, finish (breaking)
  |
  
  Branch is stacked on `feat/server-typed-error-internal` ([#1243](https://github.com/Devolutions/IronRDP/issues/1243)), which
  is stacked on `feat/server-typed-error` ([#1242](https://github.com/Devolutions/IronRDP/issues/1242)).
  
  ## Coordinate with #1239
  
  The `ConnectionHandler` trait was extended in #1239 (SuppressOutput /
  RefreshRectangle / FrameAcknowledge handlers). If this PR lands first,
  #1239 needs a trivial rebase to the new trait shape (the new methods
  stay; only `on_disconnected`'s signature changes around them). If #1239
  lands first, this PR rebases onto its trait shape. Either order works.
  
  ## Test plan
  
  - `cargo xtask check fmt -v` clean
  - `cargo xtask check lints -v` clean (workspace, all-targets, with
  helper + __bench features)
  - `cargo xtask check tests -v` passes
  - `cargo build --workspace --all-targets` clean

- [**breaking**] Convert display traits to ServerResult, drop anyhow dep ([#1245](https://github.com/Devolutions/IronRDP/issues/1245)) ([9abd6eb482](https://github.com/Devolutions/IronRDP/commit/9abd6eb482d8e7bcc64d578fc0a97bb12b6c82d9)) 

  ## Summary
  
  Final step of the staged migration started in #1242 and continued in
  #1243 / #1244. Completes the typed error story for `ironrdp-server` by
  converting the last consumer-facing trait surface and dropping the
  `anyhow` dependency entirely. Closes #1209.
  
  ## Public API changes
  
  ```diff
   pub trait RdpServerDisplayUpdates {
  -    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>>;
  +    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>>;
   }
  
   pub trait RdpServerDisplay: Send {
  -    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>>;
  +    async fn updates(&mut self) -> ServerResult<Box<dyn RdpServerDisplayUpdates>>;
   }
  ```
  
  These are **breaking changes** for handler implementations of the two
  display traits.
  
  ## Internal changes
  
  - `from_anyhow` private bridge and `AnyhowError` wrapper struct removed
  from `error.rs`.
  - `anyhow` dependency removed from `ironrdp-server/Cargo.toml`.
  - `builder.rs` `NoopDisplayUpdates` / `NoopDisplay` impls and the
  docstring examples in `display.rs` and `README.md` updated to match the
  new trait shapes.
  - `crates/ironrdp/examples/server.rs` and
  `crates/ironrdp-testsuite-extra/tests/main.rs` updated to return
  `ServerResult` from their `RdpServerDisplay/Updates` impls.
  - `benches/src/perfenc.rs` updated to construct `ServerError` variants
  instead of `anyhow::Error` and converts at its own `anyhow::Result` main
  boundary via `.map_err(|e| anyhow::anyhow!(e))`.
  - `urbdrc.rs`, the `usb`-gated USB device facade, converted from
  `anyhow` to `ServerResult`/`ServerError` (69 sites). This file predates
  the typed error migration and was never touched by it, so removing the
  `anyhow` dependency broke it under the `usb` feature. Typed external
  errors from `ironrdp-usb`/`ironrdp-rdpeusb` are wrapped with
  `ServerError::custom`; hand-rolled invariant checks (former
  `bail!`/`ensure!`) use `ServerError::reason`.
  
  ## After this PR
  
  `ironrdp-server` has **no anyhow dependency** and the public surface is
  fully typed against `ServerError`, including the `usb`-gated facade. The
  full chain:
  
  | Step | PR | Scope |
  |------|-----|-------|
  | 1 | #1242 | Add `ServerError` / `ServerErrorKind` / ext traits,
  convert 5 public functions, internal anyhow stays via private bridge |
  | 2 | #1243 | Convert encoder/helper/echo internals |
  | 3 | #1244 | Convert server.rs internals + `on_disconnected` parameter
  |
  | **4** | **this PR** | Convert display traits and the usb facade, drop
  anyhow dep, finish |
  
  ## Stacking note
  
  Stacked on `feat/server-typed-error-server` ([#1244](https://github.com/Devolutions/IronRDP/issues/1244)). Rebase on landing
  in PR-number order.
  
  ## Test plan
  
  - `cargo xtask check fmt -v` clean
  - `cargo xtask check lints -v` clean (workspace, all-targets, with
  helper + __bench features)
  - `cargo xtask check tests -v` passes (including doctests)
  - `cargo xtask check features --case workspace/powerset-runtime` clean
  (44/44, covers `ironrdp-server` across the full feature powerset
  including `usb`)
  - `cargo build --workspace --all-targets` clean

- Expose measured bandwidth to the embedder ([#1734](https://github.com/Devolutions/IronRDP/issues/1734)) ([e01839bfd8](https://github.com/Devolutions/IronRDP/commit/e01839bfd84ff5a392ca81ae80c0350bf457f7b1)) 

  ## Summary
  
  - After #1470/#1471, the server can tell the client its measured
    bandwidth over the wire (Network Characteristics Result), but nothing
    exposes that figure outside the crate: `snapshot()` returns
    `RttSnapshot` (min/max/avg/sample_count only) and
    `autodetect_rtt_handle()` carries RTT alone. An embedder's own
    health or flow-control layer that wants the same bandwidth figure
    the client already received has no way to read it.
  - Mirrors the existing `autodetect_rtt_handle` shape exactly: a new
    `autodetect_bandwidth` `Arc<AtomicU32>` field (`u32::MAX` sentinel
    until the first measurement, matching `autodetect_rtt`), an
    `autodetect_bandwidth_handle()` accessor, and a
    `with_autodetect_bandwidth_handle()` builder method for injecting a
    shared instance. Adds `AutoDetectManager::bandwidth_kbps()` as the
    underlying getter; the field already existed internally with no
    public accessor.
  - The store site needed a before/after comparison rather than a plain
    `else` branch on `handle_response`'s result: that function reports a
    matched RTT sample through its return value but only updates
    bandwidth internally, so a naive `else` would also fire, and
    mislabel the store as a fresh bandwidth measurement, on any
    unmatched or stray RTT response once a bandwidth figure had been
    recorded at least once. Comparing `bandwidth_kbps()` before and
    after the call only stores and logs when a measurement genuinely
    completed.
  - Adds three tests to the existing autodetect test file, mirroring the
    RTT-handle tests already there: `bandwidth_kbps()` reflecting a
    completed measurement, the handle's sentinel default, and the
    injected-handle round trip.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass, including the
  3 new tests (22/22 passing in `server::autodetect`).
  
  ## Notes
  
  No public API break: `RdpServer::new` is crate-private, and the public
  surface (`RdpServerBuilder`) only gains an additive optional field and a
  new method.

- Wire RdpdrServer into ironrdp-server ([#1784](https://github.com/Devolutions/IronRDP/issues/1784)) ([695af5a6e8](https://github.com/Devolutions/IronRDP/commit/695af5a6e814badb3daa60184a8d2048d0e66716)) 

  ## Depends on #1783
  
  Stacked on `feat/rdpdr-server-core` ([#1783](https://github.com/Devolutions/IronRDP/issues/1783)), which itself depends on
  #1779. Neither PDU codec bidirectionality nor RdpdrServer's
  orchestration is reachable from ironrdp-server without this.
  
  ## Summary
  
  - Wires RdpdrServer into ironrdp-server, mirroring SoundServerFactory
  and RdpsndServer exactly. RDPDR is a static virtual channel with the
  same build-a-backend-then-attach shape as audio, not a dynamic channel
  like EGFX and not a combined backend-plus-factory like clipboard.
  - RdpdrServerFactory extends the ServerEventSender supertrait rather
  than inlining a set_sender method, matching the live convention
  SoundServerFactory and CliprdrServerFactory already use. build_backend
  is named to match SoundServerFactory::build_backend.
  - Server-initiated drive I/O needs the same async relay every other
  channel uses. Added RdpdrServerMessage, one variant per RdpdrServer
  drive_* method, and a ServerEvent::Rdpdr(RdpdrServerMessage) arm in
  dispatch_server_events that looks up the live RdpdrServer instance,
  calls the matching drive_* method, and writes the encoded result, the
  same shape as the existing Rdpsnd arm.
  - Adding a fourteen-variant enum to ServerEvent pushed
  AutoReconnectCookieHandle::set's Result past clippy's result_large_err
  threshold, a method this change otherwise has nothing to do with.
  Suppressed locally with an explained reason rather than boxing the Rdpdr
  payload, which would have changed ServerEvent's public shape for a size
  concern from one unrelated method.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Added an
  exhaustive-match test over every RdpdrServerMessage variant in
  ironrdp-testsuite-core/tests/server/rdpdr.rs (no wildcard arm), so a
  future variant added to one side of the dispatch without the other fails
  to compile rather than silently falling through.
  
  ## Notes
  
  This closes out the RDPDR server-side contribution's three-part sequence
  (PDU codec, RdpdrServer orchestration, ironrdp-server wiring).

- Add graceful client disconnect with ServerSetErrorInfo ([#1798](https://github.com/Devolutions/IronRDP/issues/1798)) ([f7253861fa](https://github.com/Devolutions/IronRDP/commit/f7253861faa430d28c9dbce29d3e9e930c54748f)) 

  ## Summary
  
  `RdpServer` had no framework-level way to disconnect a client
  mid-session with a wire-visible reason. A handler wanting to reject a
  client had to build a `ServerSetErrorInfoPdu` by hand and find a way to
  reach the active connection's writer, or fall back to
  `ServerEvent::Quit`, which tears the connection down silently with no
  signal to the client at all.
  
  ## Public API changes
  
  Adds `ServerEvent::Disconnect(ErrorInfo)` and
  `ErrorInfoDisconnectHandle`, mirroring the existing
  `AutoReconnectCookieHandle` shape:
  
  ```rust
  pub fn error_info_disconnect_handle(&self) -> ErrorInfoDisconnectHandle;
  
  impl ErrorInfoDisconnectHandle {
      pub fn disconnect(&self, error: ErrorInfo) -> Result<(), mpsc::error::SendError<ServerEvent>>;
  }
  ```
  
  `ErrorInfo` is re-exported at the crate root, matching how
  `ServerAutoReconnect` is already re-exported for the auto-reconnect
  cookie API. Purely additive, no existing signature changes.
  
  ## Internal changes
  
  `Disconnect` is handled in `dispatch_server_events`, where the writer is
  already in scope: encodes `ServerSetErrorInfoPdu(error)` (MS-RDPBCGR
  2.2.5.1) via the existing `encode_share_data_pdu` helper (the same
  mechanism the auto-reconnect cookie's Save Session Info PDU already
  uses), writes it, then returns `RunState::Disconnect`.
  
  ## Notes
  
  No test added. The closest existing precedent for this class of feature,
  `AutoReconnectCookieHandle`, has no test coverage anywhere in the
  workspace either, so this is consistent with the established bar rather
  than a gap specific to this PR.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks` clean. `cargo xtask
  check features --case workspace/powerset-runtime` clean (44/44, covers
  `ironrdp-server`).

- [**breaking**] Support the Large Pointer Capability Set ([#1787](https://github.com/Devolutions/IronRDP/issues/1787)) ([bde5824213](https://github.com/Devolutions/IronRDP/commit/bde5824213bfbc59d83c8fb3d2cfbc69ad9dab77)) 

  ## Depends on #1786
  
  Stacked on top of #1786's branch: Both touch `client_accepted()`'s
  capabilities loop
  and `UpdateEncoder::new()`'s constructor signature. #1786 must land
  first.
  
  ## Summary
  
  - `ironrdp-server` never advertises or reads the Large Pointer
  Capability Set
  (MS-RDPBCGR 2.2.7.2.7), even though `ironrdp-pdu` already defines
  everything needed
  (`LargePointer`/`LargePointerSupportFlags`, `LargePointerAttribute`) and
  the client
  side already decodes and renders `UpdateCode::LargePointer`. The server
  side was the
  only gap: A pointer shape above 32x32 (or 96x96 with just the base
  Pointer
    Capability Set) could never be sent.
  - Advertises the capability derived from `ironrdp-server`'s own
  configured
  `max_request_size`, not asserted independently: MS-RDPBCGR 2.2.7.2.7
  requires the
  Multifragment Update Capability Set's `MaxRequestSize` to be at least
  38,055 bytes
    for `LARGE_POINTER_FLAG_96x96` and at least 608,299 bytes for
  `LARGE_POINTER_FLAG_384x384`. The default `max_request_size` (8 MiB)
  clears both, so
    this is enabled by default unless a server has lowered it.
  - Reads the client's negotiated flags back and enforces a distinction
  the spec draws
  explicitly: `LARGE_POINTER_FLAG_96x96` only raises the existing
  Color/New Pointer
  Update's size ceiling from 32x32 to 96x96 (New Pointer Update wraps the
  same
  `ColorPointerAttribute` as Color Pointer Update, so MS-RDPBCGR
  2.2.9.1.1.4.4's
  width/height field docs apply to both); it does not enable the separate
  Large
    Pointer Update PDU. Only `LARGE_POINTER_FLAG_384x384` does that.
  `UpdateEncoder` now enforces both halves: `RGBAPointer` is dropped
  (debug log) if it
    exceeds the negotiated 32x32/96x96 ceiling, and the new
  `DisplayUpdate::LargePointer` is dropped if the client never advertised
  `LARGE_POINTER_FLAG_384x384`, or if the shape exceeds the protocol's
  absolute
    384x384 maximum regardless of what was advertised.
  - `DisplayUpdate::LargePointer` mirrors `RGBAPointer`'s shape exactly
  (32bpp XOR mask
  only, no AND mask) since that's the only color depth this crate's
  pointer path
  produces. `ColorPointer`'s own ceiling (same `LargePointerSupportFlags`)
  is left
  unenforced for the same reason the prior PR left its separate
  availability gate
  (`colorPointerCacheSize`) ungated: Nothing in this crate constructs a
  `ColorPointer`
    update.
  
  ## Breaking change
  
  `DisplayUpdate` gains a new variant (`LargePointer`), and
  `UpdateEncoder`'s (crate
  internal, `#[cfg(feature = "__bench")]`-exposed for benchmarks)
  constructor gains a
  6th parameter. No exhaustive match over `DisplayUpdate` exists elsewhere
  in the
  workspace today, so nothing else needed updating.
  
  ## Validation
  
  `cargo xtask check fmt/typos/locks/lints/tests` all pass.

- Add diagnostic logging for EGFX flow control and dispatch timing ([#1834](https://github.com/Devolutions/IronRDP/issues/1834)) ([8d1e91eb32](https://github.com/Devolutions/IronRDP/commit/8d1e91eb3256e0ba007e909512bbaf84cb76d67b)) 

  I ran into a case where I needed to debug slow frame acknowledgement and
  dispatch stalls in ironrdp-server and ironrdp-egfx, and found the
  relevant signals were either missing or buried at TRACE level where
  nobody enables them in normal operation. This PR adds targeted
  diagnostic logging without changing any behavior.
  
  - FrameTracker now edge-triggers a debug log when backpressure or
  ack_suspended change state, instead of logging every call (which would
  flood at 30+/sec) or nothing at all.
  - FrameAcknowledge is now logged at DEBUG with latency, queue_depth, and
  in-flight count. An ack for an unknown frame_id (protocol violation or
  stale ack per MS-RDPEGFX 2.2.4.3) is now a warning instead of silent.
  - drain_output (ZGFX compression) now tracks per-batch compress time and
  ratio, logging at INFO when a batch exceeds a 10ms budget and DEBUG
  otherwise, since it runs under both the state lock and the writer lock
  and can block inbound PDU processing.
  - Incoming EGFX DVC PDUs are logged at DEBUG with their kind, so the
  client-to-server side of the channel is visible without enabling TRACE.
  - dispatch_pdu and dispatch_events now separately time lock-acquisition
  wait and handler dispatch, warning when either exceeds 50ms so the two
  causes (lock contention vs. handler/runtime stall) can be told apart
  instead of both surfacing as one generic slow-dispatch symptom.
  
  No public API changes, no behavioral changes, logging only.
  
  Note on CI: the workspace-wide test compile is currently broken on
  master independent of this PR, ironrdp-daemon's consume_output() call
  site passes a raw Receiver where OutputEventReceiver is expected. I
  confirmed this reproduces on a clean, unmodified checkout of master.
  This PR only touches ironrdp-server and ironrdp-egfx.

### <!-- 4 -->Bug Fixes

- Weigh RemoteFX ICAP entropy coders instead of taking the last ([#1686](https://github.com/Devolutions/IronRDP/issues/1686)) ([185f312019](https://github.com/Devolutions/IronRDP/commit/185f3120199f0373a11c7e21cecfac77b7be56af)) 

  The client's advertised TS_RFX_ICAP array was folded with a plain
  assignment in the capability exchange loop, so whichever entry happened
  to be parsed last silently decided the entropy coder. Add
  pick_remotefx_entropy_coder and
  RdpServerBuilder::with_remotefx_entropy_coder so the caller can state a
  preferred coder, with a documented fallback instead of an accidental
  one.
  
  Per MS-RDPRFX 2.2.1.1.1.1, the TS_RFX_ICAP array is the set of codecs
  the client supports, not a ranked preference, so the server is entitled
  to pick any entry. With no preference configured, the new fallback is
  whichever coder the client offers first, which is deterministic and
  matches the array's natural order, instead of whichever happened to be
  parsed last. mstsc sends RLGR1 then RLGR3, so today's last-wins behavior
  always lands on RLGR3, and RLGR1 never runs against a real client.
  
  Builds on #1684 and #1685, which give the quant table ([#1557](https://github.com/Devolutions/IronRDP/issues/1557)) the same
  server-side shape. Default behavior changes for clients that offer both
  coders: the server now uses the first-offered coder instead of the
  last-offered one, unless with_remotefx_entropy_coder states otherwise.

- Release the static channels when a connection ends ([#1721](https://github.com/Devolutions/IronRDP/issues/1721)) ([8b1f0dab08](https://github.com/Devolutions/IronRDP/commit/8b1f0dab088c2205ec1d9e8cb1b49bf902971b23)) 

  `run_connection` leaves the connection's static channels attached to the
  server. `run` clears them right after each session, so servers driven by
  it
  are unaffected — but the channel set is per-connection state, and
  `run_connection` is the public entry point for embedders that run their
  own
  accept loop. There is no public API to clear it from outside.
  
  Channel backends own real resources, and several are released through
  `Drop`:
  `RdpsndServer::drop` stops its handler, which is how an audio backend
  learns
  to stop capturing. Held past the connection, they keep running with no
  client
  to serve, and their events accumulate in the server event queue until
  the
  next client attaches new channels.
  
  Measured on hypr-rdp, a Wayland RDP server that accepts through
  `run_connection` so it can answer concurrent connections instead of
  leaving
  them in the backlog. After the client was killed:
  
  | | audio capture | virtual sink | RSS over 50 s |
  |---|---|---|---|
  | `run()` | stopped | removed | flat |
  | `run_connection()` | still running | still the default output | +176
  KB/s |
  
  176 KB/s is exactly 44100 Hz x 2ch x 16-bit — the capture thread still
  recording into the queue.
  
  The reset moves into `run_connection_with` so it covers every exit path,
  including the early returns, and the now-redundant one in `run` is
  dropped.
  The doc comment above the same function already explains why
  per-connection
  state has to be handled here rather than by the caller.

- [**breaking**] Drop the tokio and tokio_rustls re-exports ([#1796](https://github.com/Devolutions/IronRDP/issues/1796)) ([cac7bd6f37](https://github.com/Devolutions/IronRDP/commit/cac7bd6f37120711570b0156dd87c8ce6145fad6)) 

  ## Summary
  
  `ironrdp-server` re-exported both `tokio` and `tokio_rustls` wholesale
  so consumers did not need their own `tokio` dependency. This pins every
  consumer's `tokio` version to whatever `ironrdp-server` picks, with no
  way to upgrade independently. Drops the re-export.
  
  ```diff
  -pub use {tokio, tokio_rustls};
  ```
  
  ## Public API changes
  
  - `ironrdp_server::tokio` and `ironrdp_server::tokio_rustls` no longer
  exist. Consumers using either path need their own `tokio` and
  `tokio-rustls` dependencies.
  - `tokio-rustls` stays marked `# public` in `Cargo.toml`: `TlsAcceptor`
  genuinely appears in public signatures (`with_tls`, `with_hybrid`,
  `TransportTls::Tls`). `tokio`'s `# public` marker is dropped since no
  public signature returns or takes a bare `tokio` type; it was only
  public via the re-export.
  
  ## Internal changes
  
  - The one workspace consumer, `crates/ironrdp/examples/server.rs`, now
  imports `tokio` directly instead of through the facade.
  - `tokio` added as a direct dev-dependency of the `ironrdp` facade
  crate, and to its existing `#[cfg(test)] use { ... as _ }`
  unused-dependency silencer, the same pattern already used there for
  every other example-only dependency.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks` clean. `cargo xtask
  check features --case workspace/powerset-runtime` clean (44/44, covers
  `ironrdp-server`).

- Honor the client's negotiated pointer cache size ([#1786](https://github.com/Devolutions/IronRDP/issues/1786)) ([e01cc76116](https://github.com/Devolutions/IronRDP/commit/e01cc76116fc0b238bca25acccce03eadb158550)) 

  ## Summary
  
  - MS-RDPBCGR 2.2.7.1.5 (Pointer Capability Set): pointerCacheSize is the
  client's advertised cache size for the New Pointer Update specifically
  (colorPointerCacheSize is the separate, always-supported Color Pointer
  Update cache). A zero or absent pointerCacheSize means the client did
  not advertise New Pointer Update support at all, and the server must not
  use it.
  - client_accepted() already decodes the client's Pointer capability set
  correctly, but the value was discarded: CapabilitySet::Pointer(_) fell
  into a wildcard arm and never reached anything downstream. RGBAPointer
  and CachedPointer (the New Pointer Update and its cache-reference
  companion) were emitted unconditionally regardless of what the client
  actually negotiated.
  - Threads the negotiated pointerCacheSize into UpdateEncoder, alongside
  the other capability-derived construction parameters (surface_flags,
  codecs) it already receives from the same capabilities loop. RGBAPointer
  and CachedPointer are now dropped with a debug log rather than encoded
  when pointerCacheSize is zero, the same drop-and-log shape the encoder
  already uses for a Bitmap update that exceeds the desktop size.
  - ColorPointer is left alone: nothing in this crate currently constructs
  a ColorPointer update, so gating it on the separate
  colorPointerCacheSize field would be validation for a path that can't
  happen yet.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  Found while scoping server-driven cursor shape support for a downstream
  embedder: RGBAPointer emission needs this gate to be spec-compliant
  before it can be used at all.

- Suppress result_large_err on ErrorInfoDisconnectHandle::disconnect ([#1812](https://github.com/Devolutions/IronRDP/issues/1812)) ([b828a6cf3d](https://github.com/Devolutions/IronRDP/commit/b828a6cf3d91688a14b1ccd84d48ca2e2cc84325)) 

  ## Summary
  
  `cargo xtask check lints` is currently red on master.
  `ErrorInfoDisconnectHandle::disconnect` (added in #1798) returns
  `Result<(), mpsc::error::SendError<ServerEvent>>`, and `ServerEvent`
  grew past clippy's `result_large_err` threshold when #1784 added
  `RdpdrServerMessage`. `AutoReconnectCookieHandle::set` already carries
  the same error type and got a scoped `#[expect(clippy::result_large_err,
  ...)]` for exactly this reason; `disconnect` didn't get the matching
  suppression when it landed.
  
  This adds the identical `#[expect]`, same attribute, same reason text,
  to `disconnect`.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Confirmed the
  failure reproduces on bare master before this change and clears after.
  
  ## Notes
  
  No behavior change. `ServerEvent`'s size is driven by its largest
  per-channel payload regardless of this method; boxing the payload to
  shrink the type is a separate, larger change out of scope here.

- Disable Nagle on accepted connections ([#1799](https://github.com/Devolutions/IronRDP/issues/1799)) ([915e681845](https://github.com/Devolutions/IronRDP/commit/915e681845d9c27198776d735952c272e19566b9)) 

  RDP output is a stream of small writes the peer is waiting on: a frame,
  a pointer update, a channel PDU. Nagle holds the trailing partial
  segment of each until the previous one is acknowledged, so against a
  peer using delayed acknowledgements every such write waits on a timer
  that has nothing to do with the encoder or the link. `run` sets
  `SO_REUSEADDR` on the listening socket and nothing on the accepted ones.
  
  A failure is logged rather than propagated: the session works either
  way, only less promptly, and refusing a connection over a socket option
  would cost more than the latency it avoids.
  
  `run_connection` and `run_connection_with` take a stream from the caller
  and cannot set this themselves, so their documentation now says whose
  job it is. For reference, `ironrdp-client` sets it on its RDCleanPath
  transport and not on a direct connection.

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
