# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.10.0...ironrdp-connector-v0.11.0)] - 2026-08-08

### <!-- 0 -->Security

- [**breaking**] Implement multitransport bootstrapping handshake ([#1098](https://github.com/Devolutions/IronRDP/issues/1098)) ([e45fbfe0f5](https://github.com/Devolutions/IronRDP/commit/e45fbfe0f597011706e77fc174ca14e5e9d435b9)) 

  ## Summary
  
  Makes the `MultitransportBootstrapping` state functional instead of a
  no-op
  pass-through. After licensing the server may send 0, 1, or 2 Initiate
  Multitransport Request PDUs before capabilities exchange. Each one is
  surfaced
  to the application, which establishes UDP transport (RDPEUDP2 + TLS +
  RDPEMT)
  or declines, and the connector reports the outcome back to the server.
  
  ## API
  
  Mirrors the existing `should_perform_X()` pause-point pattern used by
  TLS
  upgrade and CredSSP, but uses `complete_X()` / `skip_X()` rather than
  `mark_X_as_done()` because completion carries result data:
  
  - `should_perform_multitransport()`: true while a request awaits an
  outcome
  - `multitransport_request()`: the request awaiting an outcome, or `None`
  - `complete_multitransport(result, output)`: report the outcome, resume
  - `skip_multitransport(output)`: decline, resume
  
  `complete_multitransport` accepts a `MultitransportResult` (a `Success`
  /
  `Failure(hresult)` enum) rather than a caller-built response PDU. The
  connector
  builds the response internally from the stored request ID.
  
  Requests are surfaced one at a time rather than as a batch. There is no
  end
  marker for the set, and MS-RDPBCGR 3.2.5.15.1 requires the client to act
  on a
  request as soon as it decodes one, so waiting to learn how many are
  coming is
  not an option the protocol offers. `should_perform_multitransport()` can
  therefore come round twice; the caller answers reliable and lossy
  separately.
  
  ## Approach
  
  **Routing.** Requests arrive on the negotiated MCS message channel
  (2.2.15.1) and the Demand Active on the I/O channel, so the channel
  decides
  which is which. The message channel also carries NetworkAutoDetect since
  #1348,
  so a decode still confirms what arrived there, but the I/O channel is
  never
  speculatively decoded as multitransport. A PDU on neither channel is an
  error.
  
  For the decode to be a sound confirmation the request decoder must
  reject a
  Demand Active, so this PR also tightens `MultitransportRequestPdu` to
  require
  the exact `SEC_TRANSPORT_REQ` security-header flag.
  
  **Yielding.** Each request is surfaced the moment it decodes. Responding
  returns the connector to `MultitransportBootstrapping` to read whatever
  comes
  next, which may be a second request or the Demand Active. Nothing is
  buffered
  and nothing is replayed: when the request is surfaced the Demand Active
  has not
  arrived yet.
  
  **Soft-Sync.** The Initiate Multitransport Response is the Soft-Sync
  signalling path (2.2.15.2), permitted only when both peers advertised
  `SOFTSYNC_TCP_TO_UDP` in their GCC `MultiTransportChannelData`. The
  server's
  block is retained from the GCC exchange and checked against the client's
  configured flags. One rule covers both paths:
  
  - Soft-Sync negotiated: always respond, `S_OK` or `E_ABORT`, including
  on
    `skip_multitransport()`, which 3.2.5.15.1 requires. Both the async and
  blocking drivers skip automatically, so without this every default
  client
    leaves a compliant server waiting.
  - Not negotiated: never respond. The outcome is reported in band on the
  new
  transport, and putting anything on the main channel would be the
  violation.
  
  The response goes on the message channel per 2.2.15.2 and 3.2.5.15.2. If
  Soft-Sync was negotiated but no message channel exists the connector
  errors
  rather than falling back to the I/O channel, and that check runs before
  the
  pending state is taken, so the caller is left with a connector it can
  still
  inspect or decline from.
  
  ## Wire behaviour
  
  On the wire TCP and UDP negotiation happen in parallel: the UDP
  transport is
  established alongside the ongoing TCP handshake, and its completion
  signals the
  dynamic-channel layer that subsequent channels may migrate to UDP. The
  connector's API yield point here is a Rust affordance, not a
  spec-mandated TCP
  pause. Thanks to @hardening for the correction.
  
  ## Tests
  
  Connector state-machine tests in `ironrdp-testsuite-core` drive the
  public API
  with the shared `SERVER_DEMAND_ACTIVE` fixture:
  
  - a request is surfaced on arrival, without waiting for a following PDU
    (regression test for the stall);
  - responding returns to bootstrapping so a second request is read
  normally;
  - a third request is rejected per the 2.2.15.1 cap;
  - a Demand Active on the I/O channel ends bootstrapping;
  - the response targets the message channel, decoded back off the wire;
  - a `Failure` result is carried through;
  - `skip` sends `E_ABORT` under Soft-Sync, and nothing without it;
  - `complete` emits nothing without Soft-Sync but still resumes;
  - a failed response leaves the connector in `MultitransportPending`,
  still able
    to report or decline, rather than `Consumed`;
  - `complete` / `skip` outside `MultitransportPending` error;
  - a Demand Active's user data does not decode as a
  `MultitransportRequestPdu`
    (regression test for the decoder tightening above).

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

- Surface ShareDataPdu variant in unexpected-PDU errors ([#1329](https://github.com/Devolutions/IronRDP/issues/1329)) ([df1f7e7faa](https://github.com/Devolutions/IronRDP/commit/df1f7e7faaf068435bfbbe1efcb4a8800ebb3d9f)) 

  ## Summary
  
  - Addresses ask 1 of #1232: when the server sends a
  `ShareControlPdu::Data` wrapping an unexpected `ShareDataPdu`, the three
  error sites in `headers.rs` and `connection_activation.rs` now drill
  into the `Data` wrapper and surface the inner variant name instead of
  reporting only `"Data"`.
  - For `ServerSetErrorInfo` specifically (the asker's high-value case),
  the existing `ErrorInfo::description()` is appended so callers can see
  why the server rejected the session without substring matching on the
  `Reason` string.
  - New `pub fn describe_unexpected_share_control_pdu` in `headers.rs`
  centralizes the formatting; `decode_share_data`, `decode_io_channel`,
  and `ConnectionActivation::CapabilitiesExchange` all route through it.
  - Non-`Data` variants continue to use the outer `as_short_name()`, so
  diagnostics for `ServerDeactivateAll` and `ClientConfirmActive` are
  preserved verbatim.
  
  ## Validation
  
  - Three unit tests in `headers::tests` cover the helper: a non-`Data`
  variant (`ServerDeactivateAll`), a `Data` wrapper around a
  non-SetErrorInfo inner (`Update(Vec::new())`), and a `Data` wrapper
  around `ServerSetErrorInfo` carrying
  `ProtocolIndependentCode::ServerDeniedConnection`.
  - `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  - Helper is `pub`, not `pub(crate)`: it has to be, since
  `ironrdp-connector`'s `connection_activation.rs` calls it cross-crate.
  That adds
  `ironrdp_pdu::rdp::headers::describe_unexpected_share_control_pdu` to
  `ironrdp-pdu`'s public surface. Additive and non-breaking, confirmed by
  `cargo semver-checks --baseline-rev <merge-base>`: no update required
  for either `ironrdp-pdu` or `ironrdp-connector`.
  - Ask 2 from #1232 (an optional structured `ConnectorErrorKind` variant
  for "server rejected at capabilities phase") is intentionally deferred.
  The asker framed it as optional and the wire-level information is now
  available in the `Reason` string.
  - `Refs #1232` rather than `Closes` so the issue stays open while you
  decide on ask 2.

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

### <!-- 4 -->Bug Fixes

- Surface server error-info disconnect during reactivation ([#1467](https://github.com/Devolutions/IronRDP/issues/1467)) ([f57d38ff74](https://github.com/Devolutions/IronRDP/commit/f57d38ff74624b99ebcc1369c220b646dd261b71)) 

  ## Summary
  
  After a Deactivate-All, a server may end the session (MS-RDPBCGR
  1.3.1.3) instead of reactivating, sending a Set Error Info PDU that
  carries the disconnect reason. `ConnectionActivationSequence`'s
  Capabilities Exchange step only recognized `ServerDemandActive` (and
  skipped `ServerDeactivateAll`), so the Error Info PDU fell through to a
  generic "unexpected Share Control PDU" error and the real reason was
  lost.
  
  - Handle `ServerSetErrorInfo` in Capabilities Exchange the way
  `ConnectionFinalizationSequence` already does: return a `reason` error
  carrying the error-info description.
  - `ERRINFO_NONE` is informational, so it is skipped (stay in
  Capabilities Exchange, await Demand Active) rather than treated as
  fatal, matching the finalization sequence.
  
  Found while validating the client against GNOME Remote Desktop ([#1446](https://github.com/Devolutions/IronRDP/issues/1446)):
  grd ends the session right after activation when its backend screencast
  session cannot be created (for example a locked desktop), and the client
  surfaced only an opaque error at that point.
  
  ## Validation
  
  `cargo xtask check fmt`, `lints`, `tests`, `typos`, `locks` all pass.
  Two new integration tests in `tests/session/connection_activation.rs`
  cover the disconnect-reason path and the benign `ERRINFO_NONE` path.
  
  ## Notes
  
  No wire-format or public-API change: this only improves the error
  surfaced on an existing failure path.

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

- Answer connect-time Bandwidth Measure to unblock FreeRDP servers ([#1465](https://github.com/Devolutions/IronRDP/issues/1465)) ([f736b4e8b9](https://github.com/Devolutions/IronRDP/commit/f736b4e8b901f8889435449c26421dd45ec05bf4)) 

  ## Summary
  
  - The connector answers only the RTT auto-detect request at connect time
  and returns nothing for the Bandwidth Measure Stop, on the assumption
  that skipping it does not stall the sequence.
  - That assumption fails for FreeRDP-based servers: GNOME Remote Desktop
  blocks in its `AWAIT_BW_RESULT` state until it receives a Bandwidth
  Measure Results reply and never proceeds to licensing, so the connection
  hangs right after the Client Info PDU. Windows servers tolerate the
  omission, which hid it.
  - Reply to a connect-time `BandwidthMeasureStop` with a
  `BandwidthMeasureResults` PDU carrying the payload size the server
  handed us, over a nominal interval.
  
  ## Scope
  
  This is the unblock alone. The reported interval is nominal
  (`time_delta_ms: 1`) because the sans-I/O layer has no time source:
  nothing reaches the connector that says when the bytes arrived. The
  figure is an informational QoS hint and the server proceeds on receipt,
  which is what unsticks the connection.
  
  Measuring it properly needs an arrival time threaded down from the I/O
  driver, which is a breaking change to `Sequence::step` and does not
  belong in a fix aimed at getting FreeRDP servers connecting. It is
  #1530, stacked on this branch: it introduces `MonotonicInstant`, has
  `Framed` record when each read completed, and replaces the nominal
  figure here with the real Start-to-Stop interval and accumulated byte
  count.
  
  Split out at @CBenoit's suggestion in review.
  
  ## Validation
  
  - `cargo xtask check fmt/typos/lints/tests/locks` all pass on the pinned
  toolchain.
  - Regression test: a connect-time Bandwidth Measure Stop produces a
  response frame and the auto-detect phase continues.
  - Reproduced and fixed live against gnome-remote-desktop 49: before, the
  connector stalled after Client Info with grd in `AWAIT_BW_RESULT`;
  after, grd proceeds through licensing and DEMAND_ACTIVE to an active
  session.
  
  ## Notes
  
  - Found while bringing up the client-side Graphics Pipeline against grd
  ([#1446](https://github.com/Devolutions/IronRDP/issues/1446)), but it is an independent connector bug affecting any connection
  to a FreeRDP-based server, not specific to EGFX.
  - Previously depended on #1511 for a zero-`payloadLength` regression
  test. #1511 has merged, and that test moved out with the rest of the
  measurement work, so there is no dependency left here.



## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.9.0...ironrdp-connector-v0.10.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 4 -->Bug Fixes

- Stay in CapabilitiesExchange when activation handles DeactivateAll ([#1371](https://github.com/Devolutions/IronRDP/issues/1371)) ([a4fde9fc50](https://github.com/Devolutions/IronRDP/commit/a4fde9fc50f41d1534f32e619bbe0bbbddc64f25)) 

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.

- Reduce dependency on ironrdp-connector ([#1419](https://github.com/Devolutions/IronRDP/issues/1419)) ([5c22f86a71](https://github.com/Devolutions/IronRDP/commit/5c22f86a7150bc10c26a3be39bfaebf84c67d781)) 

  Removes the leftover legacy modules and moves actually useful utilities to ironrdp-pdu crate.

- [**breaking**] Rework the connection activation API ([#1435](https://github.com/Devolutions/IronRDP/issues/1435)) ([c6a0286dcb](https://github.com/Devolutions/IronRDP/commit/c6a0286dcb49d9ac54c65c4f9325b41e05d541b8)) 

  Introduces a ConnectionActivationFactory (exposed on ConnectionResult)
  that builds a fresh ConnectionActivationSequence per
  Deactivation-Reactivation, replacing ConnectionActivationSequence::reset_clone,
  and turns Deactivate-All handling into a bare signal so consumers own the
  activation sequence.

### <!-- 7 -->Build

- Align sspi and picky dependencies ([#1385](https://github.com/Devolutions/IronRDP/issues/1385)) ([0a461b5d36](https://github.com/Devolutions/IronRDP/commit/0a461b5d366677fd2f0f664a4f0074e4ab697c42)) 



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.8.0...ironrdp-connector-v0.9.0)] - 2026-05-27

### <!-- 1 -->Features

- Add alternate_shell and work_dir configuration support ([#1095](https://github.com/Devolutions/IronRDP/issues/1095)) ([a33d27fe67](https://github.com/Devolutions/IronRDP/commit/a33d27fe6771a5a155161ef40a04de88803dd84c)) 

  Add support for configuring `alternate_shell` and `work_dir` fields in
  ClientInfoPdu, which are used by:
    - CyberArk PSM (Privileged Session Manager) for session tokens
    - Remote application scenarios (RemoteApp)
    - Custom shell configurations

- Dispatch multitransport PDUs on IO channel ([#1096](https://github.com/Devolutions/IronRDP/issues/1096)) ([7853e3cc6f](https://github.com/Devolutions/IronRDP/commit/7853e3cc6f26acaf3da000c6177ca3cef6ef85fd)) 

  `decode_io_channel()` assumes all IO channel PDUs begin with
  a `ShareControlHeader`. Multitransport Request PDUs use a
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

  Add support for bulk compression negotiation and payload decoding,
  including connector plumbing, CLI configuration flags, and integration
  updates across tests/examples/FFI/web.

- Advertise multitransport channel in GCC blocks ([#1092](https://github.com/Devolutions/IronRDP/issues/1092)) ([4f5fdd3628](https://github.com/Devolutions/IronRDP/commit/4f5fdd3628f4d0d2c2a4116e4e45269d802740f1)) 

  Add multitransport_flags config option to populate the
  MultiTransportChannelData GCC block during connection negotiation.
  When None (the default), behavior is unchanged.

### <!-- 4 -->Bug Fixes

- Propagate negotiated share_id to all outgoing ShareDataPdu ([#1147](https://github.com/Devolutions/IronRDP/issues/1147)) ([2b24e9664d](https://github.com/Devolutions/IronRDP/commit/2b24e9664dd05620ff63a24d092377477fdde863)) 

- Advertise all colour depths per FreeRDP pattern ([#1231](https://github.com/Devolutions/IronRDP/issues/1231)) ([2fa7c648cb](https://github.com/Devolutions/IronRDP/commit/2fa7c648cb4a2fc9c75d967ac878f817900dc1b8)) 

  Replace the per-depth supportedColorDepths bitmask with an unconditional
  BPP32 | BPP24 | BPP16 | BPP15, following FreeRDP's approach of treating
  the field as a capability set rather than a preferred-depth indicator
  (libfreerdp/core/settings.c).
  
  The preferred depth is expressed via the two dedicated fields:
  - highColorDepth: now derived from the configured depth (15 →
  Rgb555Bpp16 / 0x0F, 16 → Rgb565Bpp16 / 0x10, else Bpp24 / 0x18),
  matching FreeRDP's ColorDepthToHighColor()
  - WANT_32_BPP_SESSION earlyCapabilityFlag: unchanged, set only for 32bpp
  
  Previously, a client configured for 24bpp advertised BPP24 only. Modern
  Windows hosts (Server 2012+) dropped 24bpp RDP support and reset the
  connection instead of negotiating down, leaving no usable depth. With
  all four bits always advertised the server can freely negotiate to the
  highest depth it supports.

- Surface actual PDU type when an unexpected Share Control PDU arrives ([#1236](https://github.com/Devolutions/IronRDP/issues/1236)) ([78effb3f91](https://github.com/Devolutions/IronRDP/commit/78effb3f9144a482395be738b2c9fd4d909b7b89)) 

- Handle ServerDeactivateAll during CapabilitiesExchange ([#1254](https://github.com/Devolutions/IronRDP/issues/1254)) ([9cb5439b4a](https://github.com/Devolutions/IronRDP/commit/9cb5439b4a78c4a7facc854464894c7893f6a926)) 

  Some RDP servers (notably GNOME Remote Desktop / grd) send a
  ServerDeactivateAll PDU before ServerDemandActive during the initial
  Capabilities Exchange phase. This is valid per MS-RDPBCGR §1.3.1.3
  (Deactivation-Reactivation Sequence).
  
  Previously this caused a hard error:
  "unexpected Share Control Pdu (expected ServerDemandActive)"
  
  Now the connector skips the DeactivateAll and waits for the next PDU.

### <!-- 5 -->Performance

- Reduce connection latency when Kerberos is disabled ([#1107](https://github.com/Devolutions/IronRDP/issues/1107)) ([b1b0289e00](https://github.com/Devolutions/IronRDP/commit/b1b0289e0067228dbc973d3edb0e27136f7ca52a)) 

### <!-- 7 -->Build

- Upgrade to sspi 0.21 and picky rc.23 ([#1296](https://github.com/Devolutions/IronRDP/issues/1296)) ([d5b3fa7db8](https://github.com/Devolutions/IronRDP/commit/d5b3fa7db8a4ce74ac9a9aaff3064faf6cb6c920)) 


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.7.1...ironrdp-connector-v0.8.0)] - 2025-12-18

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.7.0...ironrdp-connector-v0.7.1)] - 2025-09-04

### <!-- 1 -->Features

- Add API to retrieve registered SVC processors (#938) ([17833fe009](https://github.com/Devolutions/IronRDP/commit/17833fe009279823c4076d3e2e0c7d063fd24a43)) 

## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.6.0...ironrdp-connector-v0.7.0)] - 2025-08-29

### <!-- 1 -->Features

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a similar size
  of PNG, while offering 20x-50x faster encoding and 3x-4x faster decoding.

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec for SetSurface command. The PDU data contains the same
  data as the QOI codec, with zstd compression.

- Add an option to specify a timezone (#917) ([6fab9f8228](https://github.com/Devolutions/IronRDP/commit/6fab9f8228578b3c78db131b3c2e0526352116a9)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Rename option no_server_pointer into enable_server_pointer ([218fed03c7](https://github.com/Devolutions/IronRDP/commit/218fed03c7993af0f958453e3944c58bcf9f43cb)) 

- [**breaking**] Rename option no_audio_playback into enable_audio_playback ([5d8a487001](https://github.com/Devolutions/IronRDP/commit/5d8a487001c1280cbaf9f581f2a9a2f47d187bf0)) 

### <!-- 7 -->Build

- Bump rand to 0.9 ([de0877188c](https://github.com/Devolutions/IronRDP/commit/de0877188cbb3692c3ce0d9a72f6e96d515cde1f)) 

- Bump picky from 7.0.0-rc.16 to 7.0.0-rc.17 (#941) ([fe31cf2c57](https://github.com/Devolutions/IronRDP/commit/fe31cf2c574e0b06177a931db4cac95ea9cfbe7e)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.5.1...ironrdp-connector-v0.6.0)] - 2025-07-08

### Build

- [**breaking**] Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.5.0...ironrdp-connector-v0.5.1)] - 2025-07-03

### <!-- 7 -->Build

- Bump picky to v7.0.0-rc.15 (#850) ([eca256ae10](https://github.com/Devolutions/IronRDP/commit/eca256ae10c52c4a42e7e77d41c0a1d6c180ebf3)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.4.0...ironrdp-connector-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Add no_audio_playback flag to Config struct ([9f0edcc4c9](https://github.com/Devolutions/IronRDP/commit/9f0edcc4c9c49d59cc10de37f920aae073e3dd8a)) 

  Enable audio playback on the client.

### <!-- 4 -->Bug Fixes

- [**breaking**] Fix name of client address field (#754) ([bdde2c76de](https://github.com/Devolutions/IronRDP/commit/bdde2c76ded7315f7bc91d81a0909a1cb827d870)) 

- Inject socket local address for the client addr (#759) ([712da42ded](https://github.com/Devolutions/IronRDP/commit/712da42dedc193239e457d8270d33cc70bd6a4b9)) 

  We used to inject the resolved target server address, but that is not
  what is expected. Server typically ignores this field so this was not a
  problem up until now.

### Refactor

- [**breaking**] Add supported codecs in BitmapConfig ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.2...ironrdp-connector-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.3.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.1...ironrdp-connector-v0.3.2)] - 2025-03-07

### Build

- Update dependencies


## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.0...ironrdp-connector-v0.3.1)] - 2025-01-30

### <!-- 4 -->Bug Fixes

- Decrease log verbosity for license exchange ([#655](https://github.com/Devolutions/IronRDP/issues/655)) ([c8597733fe](https://github.com/Devolutions/IronRDP/commit/c8597733fe9998318764064c3682506bf82026d2)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.2...ironrdp-connector-v0.3.0)] - 2025-01-28

### <!-- 1 -->Features

- Support license caching ([#634](https://github.com/Devolutions/IronRDP/issues/634)) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

  Adds support for license caching by storing the license obtained
  from SERVER_UPGRADE_LICENSE message and sending
  CLIENT_LICENSE_INFO if a license requested by the server is already
  stored in the cache.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo ([#631](https://github.com/Devolutions/IronRDP/issues/631)) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

### <!-- 7 -->Build

- Bump picky from 7.0.0-rc.11 to 7.0.0-rc.12 ([#639](https://github.com/Devolutions/IronRDP/issues/639)) ([a16a131e43](https://github.com/Devolutions/IronRDP/commit/a16a131e4301e0dfafe8f3b73e1a75a3a06cfdc7)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.1...ironrdp-connector-v0.2.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
