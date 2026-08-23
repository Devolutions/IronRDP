# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-client-v0.1.0...ironrdp-client-v0.2.0)] - 2026-08-23

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

- Connect to Windows Sandbox named pipes ([#1580](https://github.com/Devolutions/IronRDP/issues/1580)) ([39b020343d](https://github.com/Devolutions/IronRDP/commit/39b020343d962962bfbefc89939be64d5c716196)) 

  Windows Sandbox's default attach path is a local named pipe carrying
  plain TPKT/X.224 with PROTOCOL_RDP and ENCRYPTION_LEVEL_NONE, not
  TCP:3389 or VMConnect. Allow the connector and client to complete that
  sequence only via an explicit opt-in (`enable_standard_rdp_security`;
  NamedPipe enables it), and teach ironrdp-agent to resolve pipe path and
  guest credentials from WindowsSandboxServer after `wsb start`.
  
  Adds Transport::NamedPipe, ironrdp_named_pipe/ironrdp_sandbox_id
  properties, sandbox list/config/stop CLI helpers via an in-process
  h2/gRPC client on the per-user `\\.\pipe\wsandbox\{guid}` pipe (no .NET
  helper), and connect --sandbox-id / --sandbox-pipe. Sandbox-derived
  properties are the merge base; explicit .rdp/--prop/flags override them
  while NamedPipe TLS/CredSSP stay forced off. Local :2179+PCB remains
  unsupported.

### <!-- 1 -->Features

- Expose the server's Input capability flags on ConnectionResult ([#1488](https://github.com/Devolutions/IronRDP/issues/1488)) ([9bcf13438c](https://github.com/Devolutions/IronRDP/commit/9bcf13438cb068b53a8cb0dc23477c498727d49f)) 

  Per [MS-RDPBCGR] 2.2.8.1.2, a client must not send fast-path input
  events unless the server advertised `INPUT_FLAG_FASTPATH_INPUT` or
  `INPUT_FLAG_FASTPATH_INPUT2` in its Input Capability Set. Today the
  connector discards the server's capability sets after Demand Active, so
  client code has no way to honour that requirement.
  
  This PR captures the server's Input capability flags during capabilities
  exchange, carries them through the `ConnectionFinalization`/`Finalized`
  activation states (so they are refreshed correctly across a
  Deactivation-Reactivation Sequence too), and surfaces them as
  `ConnectionResult::input_flags`. The session layer can then choose
  between fast-path and slow-path input per server.
  
  ### Motivation / real-world interop
  
  This is not theoretical: VirtualBox's VRDP server closes the connection
  outright on receiving a fast-path input PDU — its `VBox.log` reports
  
  ```
  VRDP: Network packet length is incorrect 0x0004. Closing connection.
  ```
  
  (a single fast-path scancode event is a 4-byte packet). VirtualBox never
  advertises fast-path input; its Demand Active offers
  `InputFlags(SCANCODES)` alone. mstsc and FreeRDP honour the negotiation
  and fall back to slow-path `TS_INPUT_PDU`s, which is why they work
  against VRDE.
  
  Haven (Android RDP client built on IronRDP) has been shipping this exact
  change as a vendored-connector patch since v5.86.1, with the slow-path
  fallback keyed off `ConnectionResult::input_flags`. Verified against a
  real VirtualBox 7.2.6 VRDE server: before the gate, the first arrow-key
  press killed the session with the log line above; with the gate,
  extended input sessions run clean, and a fast-path-capable server on the
  same host still takes the fast-path branch. (Discussed in #1158; this is
  the third and last piece Haven carries in its connector fork, alongside
  #1237 and #1472.)
  
  ### Changes
  
  - `connection_activation.rs`: capture `input_flags` from the
  `CapabilitySet::Input` in Server Demand Active (empty if absent); add
  the field to `ConnectionActivationState::{ConnectionFinalization,
  Finalized}`.
  - `connection.rs`: add `ConnectionResult::input_flags`, populated from
  the `Finalized` state.
  - Call sites in `ironrdp-client`, `ironrdp-web`, ffi, and the e2e test
  updated for the new variant field (all currently ignore it).
  
  ### Testing
  
  - Two new integration tests in
  `ironrdp-testsuite-core/tests/session/connection_activation.rs`: the
  fixture's Demand Active yields `SCANCODES | MOUSEX | UNICODE |
  FASTPATH_INPUT_2` in the `ConnectionFinalization` state, and a Demand
  Active with the Input capability stripped yields `InputFlags::empty()`.
  - `cargo check --workspace --all-targets`, `cargo clippy --workspace
  --all-targets`, and `cargo fmt --check` are clean; the 7 activation
  tests pass.

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

- Add IronRDP ActiveX COM server ([#1523](https://github.com/Devolutions/IronRDP/issues/1523)) ([ee58b7c5f2](https://github.com/Devolutions/IronRDP/commit/ee58b7c5f283ef64be93a8483242f49070c6cdc9)) 

  ## Summary
  - Add the IronRDP ActiveX COM server and native MSTSC host integration.
  - Provide bounded native-host diagnostics, credential-bridge support,
  and an AxHost test harness.
  - Preserve the minimal client, connector, and error integrations needed
  by the control.
  
  ## Validation
  - cargo check -p ironrdp-activex
  - Focused ironrdp-error and ironrdp-connector tests
  - cargo test -p ironrdp-activex --lib --no-run
  - cargo fmt --all -- --check
  - cargo xtask check locks -v
  
  ---------

- Hyper-V vmconnect support ([#1503](https://github.com/Devolutions/IronRDP/issues/1503)) ([a7cc067d50](https://github.com/Devolutions/IronRDP/commit/a7cc067d5069cbbcb13bae3e0561c0611da3bcf6)) 

  Adds Hyper-V VMConnect's direct ordering: PCB → TLS → CredSSP → X.224.
  
  Enhanced Session is the default (`GUID;EnhancedMode=1`), with
  `--vmconnect-basic` for the synthetic console. Kept this separate in
  `ironrdp-vmconnect`; no SPN changes.
  
  Tested against the nested Hyper-V lab:
  - Enhanced: `HYBRID_EX`, rendered 1280×720
  - Basic: `HYBRID`, rendered 1280×720
  - `cargo xtask check fmt/lints/tests -v`
  
  ---------

- Wire RDPDR backends into client connections ([#1600](https://github.com/Devolutions/IronRDP/issues/1600)) ([1fbc9bab0b](https://github.com/Devolutions/IronRDP/commit/1fbc9bab0bc26d8fe0789d5215005d7ea22e2a54)) 

  Build a fresh RDPDR backend product for every connection attempt.
  
  Attach RDPDR only when its product has filesystem devices, advertise
  RDPSND for Windows interoperability, and deliver deferred responses.

- Negotiate static channel chunk sizing ([#1622](https://github.com/Devolutions/IronRDP/issues/1622)) ([4e3903fbbe](https://github.com/Devolutions/IronRDP/commit/4e3903fbbef2904505f35e3437a7106807ac5987)) 

  Use the validated server VCChunkSize for outgoing static virtual channel
  data and retain 1600-byte chunks when it is absent or invalid.
  
  Apply refreshed values after reactivation across native, web, and FFI
  active stages while preserving channel flags.

- Support Hyper-V connection ordering ([#1505](https://github.com/Devolutions/IronRDP/issues/1505)) ([5c1816244e](https://github.com/Devolutions/IronRDP/commit/5c1816244e83187a04249e9d9c240d096cb78f55)) 

  Hyper-V over RDCleanPath needs PCB → TLS on the proxy, then CredSSP →
  X.224 on the client. Ordinary RDCleanPath stays X.224-first.
  
  Still VERSION_1 with the same DER fields. An explicit VMConnect request
  carries a Unicode PCB payload in `preconnection_blob` with no X.224; the
  proxy encodes the binary PCB. Generic PCB requests keep their existing
  X.224-first behavior.
  
  Gateway reference implementation:
  [Devolutions/devolutions-gateway#1372](https://github.com/Devolutions/devolutions-gateway/pull/1372)
  
  Checked locally: Rust builds, formatting, Svelte typecheck, and .NET
  build. Real nested Hyper-V E2E through Gateway: Native rendered 18
  frames, Avalonia connected and rendered its first frame, and Web
  rendered a non-empty 1280×720 canvas.
  
  ---------

- Forward negotiated windowing orders ([#1631](https://github.com/Devolutions/IronRDP/issues/1631)) ([0c3fbe78b4](https://github.com/Devolutions/IronRDP/commit/0c3fbe78b4366533b9fcea046b2b53654e003a72)) 

  Preserve Window List support during activation.
  Forward validated orders through ActiveStage and the raw FFI output.
  Desktop and web consumers retain their existing behavior.

- Add RemoteApp channel support ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a)) 

  Configure and negotiate RAIL connections, then route its static channel
  through the portable client with bounded request queues and server
  control events.

- Project RemoteApp windows ([#1641](https://github.com/Devolutions/IronRDP/issues/1641)) ([f5554f40dc](https://github.com/Devolutions/IronRDP/commit/f5554f40dc280d93506ea8352e3992e641d58e96)) 

  Project server-authoritative RAIL windows for an enabled ActiveX
  RemoteApp session and launch the configured program after RAIL becomes
  available.
  
  Forward validated opaque windowing orders to the worker, maintain their
  basic HWND lifecycle, and retain windows until the server removes them.
  Leave desktop behavior and unsupported shell features unchanged.

- Add RAIL audit commands ([#1646](https://github.com/Devolutions/IronRDP/issues/1646)) ([77759dca03](https://github.com/Devolutions/IronRDP/commit/77759dca032eb829b5be54a3c44d9be92252cf41)) 

  Expose bounded client-validated RAIL events and RemoteApp launch
  requests through the daemon IPC so headless agents can verify sessions.
  
  Preserve cursors across resize reconnects, wake waiting readers for
  locally queued launches, and redact launch data in logs.
  
  Report terminal local Execute failures without disrupting an otherwise
  valid RDP session.

- Add Input DVC and ActiveX touch ([#1647](https://github.com/Devolutions/IronRDP/issues/1647)) ([a912e19bd2](https://github.com/Devolutions/IronRDP/commit/a912e19bd2bb31f403fd7c35c8efd729a5ab5f6f)) 

  Implement MS-RDPEI for multi-touch over the dynamic virtual channel
  Microsoft::Windows::RDS::Input, and wire Windows pointer messages in
  ActiveX through session encode helpers.
  
  Introduce ironrdp-rdpei PDUs and processors, register the channel from
  the client, encode touch frames from ActiveX WM_POINTER*, and cover the
  protocol with unit and integration tests.

- Harden Windows client playback path ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db)) 

  Keep client format order for wFormatNo, play pre-v8 Wave PDUs, and apply
  volume on a broader CPAL PCM offer so ActiveX mode 0 can redirect remote
  audio reliably.
  
  Also fix clippy noise in the RDPSND client suite and keep interleaved
  volume L/R phase stable across wave blocks. Volume scaling is a simple
  amplitude map, not a logarithmic MS-RDPEA model.

- Wire MS-RDPEAI capture into Windows client and ActiveX ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d)) 

  Add the client MS-RDPEAI capture path on top of hardened RDPSND
  playback: connector CFG + static channel wiring, CPAL PCM capture
  backend, ironrdp-client --audio-capture, and ActiveX
  AudioCaptureRedirectionMode.
  
  PCM capture only accepts encode formats that match the Open capture
  stream, rejects non-16-bit capture (Data PDU size contract), and gates
  the capture backend behind ironrdp-rdpsnd-native/capture.
  
  Depends on #1648 (playback).

- Add native MS-RDPEWA WebAuthn redirection ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb)) 

  Implement the RDPEWA dynamic channel with a Windows WebAuthn backend and
  wire RedirectWebAuthn for ActiveX, the optional client feature, and the
  viewer CLI.
  
  Prefer System32\webauthn.dll via the DVC COM plugin for MSTSC parity.
  The pure-Rust backend forwards ceremonies through a webauthn.dll IWTS
  oneshot so hash-only hosts that omit clientDataJSON still work; public
  WebAuthN* remains a fallback when JSON is present. Recreate
  WebAuthN_Channel opens through shared COM/listener factories because
  Windows opens and closes the channel around each RPC.
  
  Side effects:
  - New crates ironrdp-rdpewa and ironrdp-rdpewa-native
  - Config key redirectwebauthn; ActiveX ExtendedSettings property
  - ironrdp-daemon webauthn feature for ironrdp-agent
  - IRONRDP_WEBAUTHN_FORCE_NATIVE debug switch
  - Viewer --webauthn/--no-webauthn flags; .rdp redirectwebauthn default
  - ActiveX docs note no AdvancedSettings slot and no IPersist persistence

- Expose MS-RDPEI pen and multi-touch ([#1652](https://github.com/Devolutions/IronRDP/issues/1652)) ([51f8738ea1](https://github.com/Devolutions/IronRDP/commit/51f8738ea156daeea292c63bba09d9137791af57)) 

  Extend the agent RPC path beyond single-contact touch so multi-contact
  frames, pen frames, and dismiss-hovering can be driven end-to-end
  against a real host.
  
  Wire Pen/Dismiss through rpc, daemon, CLI, ActiveX control RPC, client
  input loop, and session encode helpers. Add pen contact flag legality
  checks and round-trip coverage.

- Plumb smartcard device into RDPDR backends ([#1656](https://github.com/Devolutions/IronRDP/issues/1656)) ([66831bbbba](https://github.com/Devolutions/IronRDP/commit/66831bbbbabe3bf36bedff769c3e62819f60d46b)) 

  Return immediate SvcMessage completions from handle_scard_call so
  backends can finish MS-RDPESC IRPs without blocking the channel path.
  
  Wire WindowsRdpdrBackendFactory::with_smartcard and a minimal
  ScardSession stub that answers decoded calls with
  SCARD_E_UNSUPPORTED_FEATURE. Allow smartcard-only RDPDR products (no
  drives). Full WinSCard work and product CLI/ActiveX enablement remain
  follow-ups.
  
  Depends on #1654.

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

- Try a direct connection before the RD Gateway ([#1713](https://github.com/Devolutions/IronRDP/issues/1713)) ([a67dbf47e4](https://github.com/Devolutions/IronRDP/commit/a67dbf47e429a73b29e2b36966b6dc5b870625cd)) 

- Resolve gateway credential sources ([#1740](https://github.com/Devolutions/IronRDP/issues/1740)) ([64375e78cc](https://github.com/Devolutions/IronRDP/commit/64375e78ccd796117e2a300a096f9b75ea6e01db)) 

  Resolve UseServerCredentials from RDP account fields while preserving
  explicit gateway credentials.
  
  Qualify bare server usernames with the configured domain for gateway
  authentication.
  
  Reject unavailable credential sources instead of silently substituting
  credentials.

- Add smart-card gateway authentication ([#1741](https://github.com/Devolutions/IronRDP/issues/1741)) ([761dae12b0](https://github.com/Devolutions/IronRDP/commit/761dae12b0128be0f9bae52ca59eae8a0ba0b02f)) 

  Add an opt-in Kerberos PKINIT path for HTTP Negotiate gateway
  authentication while retaining the password Negotiate, NTLM, and Basic
  flows.
  
  The public credentials type accepts an application-supplied UPN, redacts
  credentials, and rejects unsupported smart-card feature or challenge
  combinations without exposing them.

- Enforce gateway redirection policy ([#1760](https://github.com/Devolutions/IronRDP/issues/1760)) ([9a3617f46c](https://github.com/Devolutions/IronRDP/commit/9a3617f46c9190f86cf79814a37b77dc84497f70)) 

  Honor restrictive gateway redirection flags without enabling new
  channels.

- Apply certificate validation to gateways ([#1775](https://github.com/Devolutions/IronRDP/issues/1775)) ([312d4466e7](https://github.com/Devolutions/IronRDP/commit/312d4466e7cae15d45c73a4ffcb4ae730d5e6a30)) 

  Apply the RDP client's certificate-validation policy and callback to
  every
  gateway HTTPS connection.
  
  Existing gateway callers retain their compatibility default.

### <!-- 4 -->Bug Fixes

- Preserve bulk compression across reactivation ([#1474](https://github.com/Devolutions/IronRDP/issues/1474)) ([8fcffb9e8f](https://github.com/Devolutions/IronRDP/commit/8fcffb9e8f1a2c468321c05a56ec96144316c90a)) 

  Any session that reactivates (Deactivate All → re-activate) loses bulk
  decompression and dies right after. Windows consoles reactivate right
  after logon, and compression is on by default, so this hits pretty
  easily.
  
  The reactivation path rebuilt the FastPath processor with
  [`bulk_decompressor:
  None`](https://github.com/Devolutions/IronRDP/blob/079b4842/crates/ironrdp-client/src/rdp.rs#L988).
  After that every compressed update got parsed as a raw bitmap:
  
  ```
  Received compressed FastPath data but no decompressor is configured
  BitmapData decode NotEnoughBytes: received 1662, expected 17134
  ```

- [**breaking**] Always own a bulk decompressor for FastPath updates ([#1255](https://github.com/Devolutions/IronRDP/issues/1255)) ([0dc0194418](https://github.com/Devolutions/IronRDP/commit/0dc0194418375d504a8041b75ba250dc8eeb21ad)) 

  ## Summary
  
  - A compressed FastPath update is dropped whenever the client did not
  negotiate compression, because the decompressor is only built when a
  compression type was negotiated. Servers send compressed updates
  regardless, for example on a full-frame redraw after a resize, and the
  session then fails. Closes #1193.
  - The negotiated type is the wrong thing to condition on. It describes
  what the client would send, and nothing in `ironrdp-session`,
  `ironrdp-client`, `ironrdp-web` or the FFI ever compresses outbound. On
  the receive path `BulkCompressor` holds a context per algorithm and
  `decompress` selects one per update from the packet's own type bits, so
  a decompressor built with any type decodes all of them.
  - The `Processor` now owns the decompressor and builds it on the first
  update that needs one. `ProcessorBuilder` has no corresponding field, so
  there is no `None` a consumer can pass and no path that drops a
  compressed update.
  - On demand rather than at construction because `ironrdp-web` hardcodes
  `compression_type: None` in `build_config` and so never negotiates
  compression. Constructing eagerly would charge every web session for a
  full set of algorithm contexts, and the two XCRUSH history buffers alone
  are 2 MB each, for a decompressor most of those sessions never use. That
  consumer is also the one most exposed to this bug, for the same reason.
  - `BulkCompressor::new` is now infallible. Its only failure path was a
  self-check over NCRUSH's static Huffman tables, a compile-time
  invariant, now a `debug_assert`.
  
  ## Relationship to #1474
  
  #1474 is kept, not reverted. `ActiveStage::reactivate` is adopted as the
  reactivation entry point at all four call sites it introduced: native
  client, web, FFI and the e2e test.
  
  What this PR removes is the `compression_type` retained on `ActiveStage`
  and the `make_bulk_decompressor` helper, because an on-demand
  decompressor makes both unnecessary. `reactivate` keeps its behaviour
  and loses only the compression plumbing.
  
  #1474 closed the reactivation instance of #1193, where a rebuild passed
  `None` and silently disabled decompression for the rest of the session.
  The general case is still open on master: when compression was never
  negotiated the retained type is `None`, `make_bulk_decompressor` returns
  `None`, and every compressed update takes the drop path in
  `fast_path.rs` for the lifetime of the session. Conditioning on the
  negotiated type gates the ability to receive on what was negotiated to
  send, and nothing sends.
  
  The evidence that removing the field is safe is #1474's own test.
  `test_reactivation_processes_compressed_fastpath_updates` passes
  unchanged with `compression_type` gone from the builder: the rebuilt
  processor decompresses because every processor can, not because a type
  was carried across the rebuild.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  The gated regression test is
  `testsuite-core/tests/session/fast_path.rs`, which renders the same
  bitmap update plain and bulk-compressed through fresh processors and
  asserts identical framebuffers. #1474's
  `test_reactivation_processes_compressed_fastpath_updates` in
  `testsuite-extra` passes unchanged.
  
  There is also an inline test in `fast_path.rs` pinning the allocation
  invariant, that no contexts are built until an update needs them. Note
  that `ironrdp-session` sets `[lib] test = false`, so inline tests in
  this crate are not run by `cargo test --workspace`; it runs under `cargo
  test -p ironrdp-session --lib`.
  
  ## Notes
  
  - This addresses the four points from the 2026-06-24 review. Point 4,
  that the `Option` is misleading, is the shape of this change: it is gone
  from the public API, and the private one that remains carries no
  implication that a consumer could choose not to decompress. Point 1,
  whether a cold `Rdp61` context decodes `RDP40` and `RDP50` updates
  correctly, is a non-issue: `decompress` selects the algorithm per update
  through `CompressionType::from_flags` against per-algorithm receive
  contexts, so the construction-time type never constrains the receive
  path. Point 3, silent degradation if the constructor fails, is removed
  by making `new` infallible. Point 2 is the tests above.
  - Breaking across two crates, hence the `fix(bulk,session)!` scope:
  `ProcessorBuilder` loses `bulk_decompressor`, `ActiveStageBuilder` loses
  `compression_type`, and `ironrdp_bulk::BulkCompressor::new` returns
  `Self`.
  - Incidental: `ironrdp-session` no longer exposes any `ironrdp_bulk`
  type in its public API, so that dependency's lack of a `# public` marker
  in `Cargo.toml` is now correct.

- Make certificate validation explicit ([#1520](https://github.com/Devolutions/IronRDP/issues/1520)) ([f1d53c78d3](https://github.com/Devolutions/IronRDP/commit/f1d53c78d390de1c6778773cdc859d59901466f0)) 

  IronRDP deployments commonly use self-signed or private-CA certificates.
  This keeps the historical permissive behavior for unmodified callers
  while making platform-root and server-name validation an explicit
  opt-in.
  
  ## Approach
  
  - Keep `upgrade` and `ConfigBuilder` defaults compatible with existing
  self-signed endpoints, including the prior native-TLS SNI behavior.
  - Expose `CertificateValidation::Strict` for callers that require normal
  certificate-chain and hostname validation.
  - Retain the Rustls callback path for certificate pinning or other
  explicit exception decisions; configuring a callback selects strict
  validation before invoking it.
  - Preserve CredSSP's existing public-key binding and disabled TLS
  resumption behavior.
  
  MS-CSSP section 3.1.5 does not require a common trusted CA root and
  permits servers to use self-signed certificates, so strict verification
  cannot be introduced as a transparent default.
  
  ## Validation
  
  - `cargo xtask check fmt -v`
  - `cargo xtask check lints -v`
  - Focused Rustls default/strict/callback runtime test
  - Focused native-TLS default/strict runtime test
  
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

- [**breaking**] Accept the full documented range of Client Core Data keyboardType values ([#1689](https://github.com/Devolutions/IronRDP/issues/1689)) ([c74e7c5c94](https://github.com/Devolutions/IronRDP/commit/c74e7c5c9431c87e57a1550b037867d235c1d362)) 

  ## Summary
  
  KeyboardType (TS_UD_CS_CORE's keyboardType field, MS-RDPBCGR 2.2.1.3.2)
  was a closed enum covering discriminants 1 through 7, transcribed from
  the field's other documentation in 2.2.7.1.6 (TS_INPUT_CAPABILITYSET),
  which omits value 8 (Korean keyboard). The table this type actually
  decodes against, 2.2.1.3.2, documents 1 through 8. Any client outside
  the narrower range, including every genuine Korean keyboard, had its
  whole Client Core Data parse hard rejected before the connection
  started.
  
  Converted KeyboardType to a repr(transparent) struct KeyboardType(pub
  u32) with named constants for the eight documented values, mirroring
  RdpVersion one field above it in the same struct, and made decode
  infallible at all three sites that previously disagreed with each other:
  gcc::core_data::client hard rejected, rdp::capability_sets::input
  silently dropped to None, and ironrdp-activex's COM setter returned
  E_INVALIDARG.
  
  FreeRDP hit the identical gap in its own documentation-only enum until a
  two-line fix (FreeRDP#11035). Neither FreeRDP nor xrdp gate connection
  admission on this field at all; both decode it as a raw integer.
  Windows' own GetKeyboardType additionally documents 0x51 for generic HID
  keyboards, which a narrower fix adding only Korean would not survive.
  
  Marked as breaking since KeyboardType's public shape changes for any
  downstream consumer of ironrdp-pdu outside this workspace. Note that
  cargo-semver-checks in this repo's PR automation only runs against the
  facade ironrdp crate, which will not see a break confined to
  ironrdp-pdu's own API, so the automated breaking-change label may not
  apply here even though this is a real one.
  
  ## Validation
  
  cargo xtask check fmt/lints/tests/typos/locks all pass. Extended
  ironrdp-activex's existing inline COM boundary test to cover value 8
  round tripping and a negative value still being rejected.

- Forward the target host and port to the gateway ([#1710](https://github.com/Devolutions/IronRDP/issues/1710)) ([f43966cade](https://github.com/Devolutions/IronRDP/commit/f43966cadeb460b3bb02625532143d45447ca14a)) 

  The channel-create packet (HTTP_CHANNEL_PACKET) hardcoded port 3389, so
  non-3389 RDP targets and Hyper-V VMConnect (port 2179) could not be
  tunneled through an RD Gateway.

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b)) 



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-client-v0.1.0)] - 2026-07-10

Initial release.
