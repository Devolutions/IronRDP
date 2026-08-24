# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.9.0...ironrdp-pdu-v0.10.0)] - 2026-08-24

### <!-- 0 -->Security

- Tolerate unknown security header flags in BasicSecurityHeader ([#1458](https://github.com/Devolutions/IronRDP/issues/1458)) ([a4acab488b](https://github.com/Devolutions/IronRDP/commit/a4acab488b0d549854ebe0e0e922fe7252e84c98)) 

  ## Summary
  
  Use `from_bits_truncate()` instead of `from_bits()` when decoding
  `BasicSecurityHeader` flags. Some servers (e.g., Windows Server 2019
  with RDS licensing / RD Connection Broker) send security header flag
  combinations that include bits not defined in the current bitflags enum.
  The strict `from_bits()` rejected these as invalid, causing connection
  failure during the `UpgradeLicense` license exchange phase.
  
  This matches FreeRDP behavior which masks for known flags without
  rejecting the PDU when unrecognized bits are present.
  
  ## What was tested
  
  - Existing unit tests pass (`cargo xtask check tests -v`)
  - Lints pass (`cargo xtask check lints -v`)

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

- Add ClearCodec client-side decode dispatch ([#1175](https://github.com/Devolutions/IronRDP/issues/1175)) ([714dce4662](https://github.com/Devolutions/IronRDP/commit/714dce46627e299c57d82f4f6a5c18067a95bffa)) 

  Follow-up to #1174. Supersedes #1195 (the standalone server-helper PR;
  its 46-line `send_clearcodec_frame()` is included here).
  
  Wires ClearCodec into the EGFX client's WireToSurface1 codec dispatch,
  matching the existing AVC420 and Uncompressed decode patterns.

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

- Support connection correlation info ([#1582](https://github.com/Devolutions/IronRDP/issues/1582)) ([c4483617ba](https://github.com/Devolutions/IronRDP/commit/c4483617ba05c31182b58c58be66bd41120a076d)) 

  Encode the optional 36-byte X.224 RDP_NEG_CORRELATION_INFO block and
  reject malformed negotiation records.

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

- Add RemoteApp protocol primitives ([#1636](https://github.com/Devolutions/IronRDP/issues/1636)) ([0161906731](https://github.com/Devolutions/IronRDP/commit/0161906731757356953cdb389a2cd6a42863deb2)) 

  Add portable RAIL wire types and a typed Remote Programs capability set.
  
  Validate the RAIL crate's bare `no_std` and allocation-backed
  configurations in the workspace feature matrix.
  
  Keep connection setup and windowing behavior outside this protocol
  layer.

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

### <!-- 4 -->Bug Fixes

- Tolerate unknown GCC user-data blocks instead of failing ([#1489](https://github.com/Devolutions/IronRDP/issues/1489)) ([629a8024f4](https://github.com/Devolutions/IronRDP/commit/629a8024f4832ed04247ef56597604bbb4b85017)) 

- Scope Font Map leniency ([#1506](https://github.com/Devolutions/IronRDP/issues/1506)) ([e496b7b8ea](https://github.com/Devolutions/IronRDP/commit/e496b7b8eaf60688fc0d507961713c6aabd0e05e)) 

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

- Harden framing and empty output handling ([#1515](https://github.com/Devolutions/IronRDP/issues/1515)) ([33506e6139](https://github.com/Devolutions/IronRDP/commit/33506e613923dae504f46451231dcee15a6320a2)) 

  Reject Fast-Path and TPKT frames whose declared length is smaller than
  their header or minimum packet size. Also tolerate the zero-length
  `totalLength` variation used by empty Update and Pointer output PDUs,
  while continuing to reject zero-length non-output data PDUs.
  
  Adds regression coverage for malformed frame lengths and empty output
  compatibility.

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

- Accept a zero-length connect-time bandwidth payload on decode ([#1511](https://github.com/Devolutions/IronRDP/issues/1511)) ([dffad79d61](https://github.com/Devolutions/IronRDP/commit/dffad79d6143f4d7e9b589768ad55fc98a04740c)) 

  ## What
  
  A connect-time Bandwidth Measure Stop with `payloadLength` of zero now
  decodes, as a present-but-empty payload. `Encode` still refuses to emit
  one.
  
  ## Why
  
  [MS-RDPBCGR] 2.2.14.1.4 says of `payloadLength`: "It MUST be present
  (and have a value greater than zero) if the value of the **requestType**
  field is set to 0x002B." #1491 read that as a rule for both directions
  and made encode and decode refuse a zero. The encode half is right. The
  decode half is not.
  
  The two directions answer different questions. Encoding asks what we are
  permitted to put on the wire, and a zero length has no conforming
  encoding, so refusing is correct. Decoding asks whether we can act on
  what a peer already sent. Here we can: `sequenceNumber` and
  `requestType` arrive intact, and those fully determine the Bandwidth
  Measure Results reply the PDU is asking for. The payload is random
  measurement filler per the same section, and its length is the only
  thing the reply reports about it.
  
  Rejecting therefore discards a PDU we could have answered without
  gaining any protection. FreeRDP-based servers, including
  gnome-remote-desktop, block in `AWAIT_BW_RESULT` until the results
  arrive, so a server that sends a zero length stalls the whole connection
  rather than getting a diagnostic.
  
  The fix #1491 was actually titled for, keying the optional fields off
  `requestType` instead of the `Option`, is untouched. Only the added
  zero-length rejection moves, and only on the receive side.
  
  ## Tests
  
  `connect_time_stop_with_a_zero_payload_length_is_rejected` becomes
  `..._is_accepted` and now asserts the decoded value: `payload:
  Some(vec![])`, present and empty rather than absent, since the wire
  carried a length field.
  
  Added `a_decoded_zero_length_stop_does_not_re_encode`, so the asymmetry
  is stated as a test rather than only as a comment.
  
  `connect_time_stop_without_a_payload_is_refused` is unchanged and still
  covers the encode side.
  
  ## Note
  
  This does not break the `pdu_round_trip` oracle in #1492: a failing
  `encode` after a successful `decode` is already tolerated there, and the
  re-decode assertion only fires on a successful encode.
  
  ## Verification
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. `cargo
  semver-checks` reports no update required for `ironrdp-pdu`.
  
  ## Unblocks #1465
  
  #1465 carries a regression test for a zero-`payloadLength` Stop, which
  cannot pass until this lands: #1491 made `AutoDetectRequest`'s decoder
  reject `payloadLength = 0`, so such a PDU is refused before it reaches
  the connector. Its
  `connect_time_bandwidth_answers_a_stop_carrying_an_empty_payload` is red
  today and that red is this dependency, not a defect there.
  
  Verified against current `master`: #1465 alone fails that one case out
  of 1032; #1465 with this applied passes all of them. Merging this first
  turns #1465 green with no change on its side.
  
  ## Rebased
  
  Rebased onto `master` on 2026-08-02 so the checks run against the
  current tree rather than the state before that day's merges. No
  conflicts, no content change.

- Keep auto-reconnect credential material out of Debug output ([#1496](https://github.com/Devolutions/IronRDP/issues/1496)) ([d0948faa18](https://github.com/Devolutions/IronRDP/commit/d0948faa187da673e9ded44e9e22cfbefc2c7f62)) 

- Batch Fast-Path input events ([#1630](https://github.com/Devolutions/IronRDP/issues/1630)) ([3818b48037](https://github.com/Devolutions/IronRDP/commit/3818b480375ec9411d5319d8e7af161f1d662cbf)) 

  Keep outgoing Fast-Path input frames within the 255-event protocol limit
  and preserve their order across FFI.

- Parse ClearCodec V-Bar offsets ([#1694](https://github.com/Devolutions/IronRDP/issues/1694)) ([c1db8608af](https://github.com/Devolutions/IronRDP/commit/c1db8608af35e3c1318a50fa812f3259ff69d800)) 

  Decode short V-Bar cache-miss offsets from their specified bit
  positions.

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

- Retain Progressive difference tiles ([#1698](https://github.com/Devolutions/IronRDP/issues/1698)) ([69e323ae47](https://github.com/Devolutions/IronRDP/commit/69e323ae473264e13b2bb3f70a356cd967debea8)) 

  Retain quantized DWT coefficients per Progressive tile so difference
  updates compose with their matching surface reference while progressive
  codec-context state remains isolated.
  
  Reject difference tiles that lack a retained reference instead of
  decoding them against zeros.
  
  Keep retained surface references across codec grid replacement and
  ResetGraphics; only deleting the surface releases them.

- Accept a 6-byte Share Control Header ([#1719](https://github.com/Devolutions/IronRDP/issues/1719)) ([d4b728a1d4](https://github.com/Devolutions/IronRDP/commit/d4b728a1d40b917dd94da6d2f8f12a32ce0965ec)) 

  MS-RDPBCGR 2.2.8.1.1.1.1 defines the Share Control Header as 6 bytes
  (totalLength, pduType, pduSource); shareId belongs to the PDU bodies.
  decode gates on FIXED_PART_SIZE (10), so a legal header-only PDU is
  rejected before any field is read:
  `ShareControlHeader::decode: not enough bytes provided to decode:
  received 6 bytes, expected 10`
  
  xrdp sends exactly such a Deactivate All PDU, which drops the session.
  FreeRDP 3.30.0 accepts it against the same server (xrdp 0.10.1-4.1,
  Ubuntu 26.04); ironrdp-pdu 0.9.0 does not.
  
  [#1130](https://github.com/Devolutions/IronRDP/pull/1130) relaxed
  ServerDeactivateAll::decode, but that runs after this gate — a 6-byte
  buffer never reaches it. This is the outer half suggested in
  [#314](https://github.com/Devolutions/IronRDP/issues/314) and never
  submitted.

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b)) 



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.8.0...ironrdp-pdu-v0.9.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 4 -->Bug Fixes

- Set COMPRESSION_USED on the FastPath update header when compressed ([#1382](https://github.com/Devolutions/IronRDP/issues/1382)) ([3f96d0029d](https://github.com/Devolutions/IronRDP/commit/3f96d0029d37d3cee84b419bbf4d53b5519e385d)) 

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.

- Adopt MCS and RDP header utilities relocated from ironrdp-connector ([#1419](https://github.com/Devolutions/IronRDP/issues/1419)) ([5c22f86a71](https://github.com/Devolutions/IronRDP/commit/5c22f86a7150bc10c26a3be39bfaebf84c67d781)) 

  Hosts the shared MCS and RDP security-header helpers previously living in ironrdp-connector's legacy modules.

- Decode MousePdu wheel rotation as two's complement, matching encode ([#1415](https://github.com/Devolutions/IronRDP/issues/1415)) ([9b4d01b403](https://github.com/Devolutions/IronRDP/commit/9b4d01b4038ede1cdd329fd9ea47a5d241480d1d)) 



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.7.0...ironrdp-pdu-v0.8.0)] - 2026-05-27

### <!-- 1 -->Features

- Add Initiate Multitransport Request/Response PDU types ([#1091](https://github.com/Devolutions/IronRDP/issues/1091)) ([5a50f4099b](https://github.com/Devolutions/IronRDP/commit/5a50f4099b8f8173c5c067089a0d372402dbb52d)) 

  Add MultitransportRequestPdu and MultitransportResponsePdu types for the
  sideband UDP transport bootstrapping PDUs defined in MS-RDPBCGR
  2.2.15.1 and 2.2.15.2. Needed to decode/encode the IO channel messages that
  initiate UDP transport setup.

- Add Auto-Detect Request and Response PDU types ([#1168](https://github.com/Devolutions/IronRDP/issues/1168)) ([6e5f08a1b9](https://github.com/Devolutions/IronRDP/commit/6e5f08a1b95f69b9d8182a75298b74aaf829ac39)) 

- [**breaking**] Route auto-detect PDUs through ShareDataPdu dispatch ([#1176](https://github.com/Devolutions/IronRDP/issues/1176)) ([e5f2f36e96](https://github.com/Devolutions/IronRDP/commit/e5f2f36e96dfb2036236c99a1ee83c5a36bf281f)) 

  Added Share Data PDU dispatch support for auto-detect PDUs, improving compatibility with Windows servers.

- Complete pixel format support for bitmap updates ([#1134](https://github.com/Devolutions/IronRDP/issues/1134)) ([a6b41093ce](https://github.com/Devolutions/IronRDP/commit/a6b41093ce4ece081d2538c157f6bc547c3b2607)) 

  Wires missing bitmap pixel formats (8/15/24bpp) into the session rendering
  pipeline so bitmap updates at those depths are rendered instead of being
  dropped, and adds fast-path palette update parsing to support 8bpp indexed
  color sessions.

- Add RemoteFX Progressive codec primitives ([#1196](https://github.com/Devolutions/IronRDP/issues/1196)) ([49099f0c31](https://github.com/Devolutions/IronRDP/commit/49099f0c3136c25b67801fb1b07f78542dc796de)) 

  Add wire-format types for RemoteFX Progressive Codec (MS-RDPRFX
  Progressive Extension) and the computational primitives required for progressive refinement.

- Handle slow-path graphics and pointer updates ([#1132](https://github.com/Devolutions/IronRDP/issues/1132)) ([9383380292](https://github.com/Devolutions/IronRDP/commit/938338029290f1be82a7f784d544bb77ac797aeb)) 

  Adds support for slow-path graphics and pointer updates to IronRDP, fixing connectivity issues with servers like XRDP that use slow-path output instead of fast-path. The implementation parses slow-path framing headers and routes the inner payload structures through the existing fast-path processing pipeline by extracting shared bitmap and pointer processing methods.

- Add progressive RFX decode and EGFX integration ([#1197](https://github.com/Devolutions/IronRDP/issues/1197)) ([a142799d1d](https://github.com/Devolutions/IronRDP/commit/a142799d1dcbdcd6546ec6e75173fbfe66f0ea67)) 

- Add ClearCodec bitmap compression codec ([#1174](https://github.com/Devolutions/IronRDP/issues/1174)) ([059ca902a5](https://github.com/Devolutions/IronRDP/commit/059ca902a5518113163042225bc5d2088869933a)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Remove unused legacy error types ([#1268](https://github.com/Devolutions/IronRDP/issues/1268)) ([df0bf9c69d](https://github.com/Devolutions/IronRDP/commit/df0bf9c69d88febaf6b82c479fdc7dcafe226567)) 

  Remove GccError, McsError, RdpError, SecurityDataError,
  ClusterDataError, NetworkDataError, CoreDataError, InputEventError,
  ClientInfoError, CapabilitySetsError, SessionError, and ChannelError.
  All encode/decode functions had already been migrated to use
  DecodeResult/EncodeResult from ironrdp-core, leaving these error types
  as dead code.

- Accept short Server Deactivate All PDU ([485d6c2f8d](https://github.com/Devolutions/IronRDP/commit/485d6c2f8d6f95bb06ca14cbfa4c56a27abbad0e)) 

  Some servers (XRDP, older Windows) send a Deactivate All PDU without
  the sourceDescriptor field. The decode previously required at least 3
  bytes, which caused a hard failure during deactivation-reactivation
  sequences with these servers.
  
  Treat the sourceDescriptor as optional: if the remaining data is
  shorter than the fixed part size, return successfully without
  reading the field. FreeRDP handles this the same way.

- Correct ShareDataHeader uncompressedLength calculation ([#1148](https://github.com/Devolutions/IronRDP/issues/1148)) ([c2688f464d](https://github.com/Devolutions/IronRDP/commit/c2688f464d8cbf239d35e5b43538195b1870eed8)) 

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

- [**breaking**] Remove ironrdp-egfx duplicates from ironrdp-pdu ([#1303](https://github.com/Devolutions/IronRDP/issues/1303)) ([491b91fd2f](https://github.com/Devolutions/IronRDP/commit/491b91fd2f33235e4b31dea5c4a215e67f734179)) 

- Cover BitmapCacheV3 in CapabilitySet encoder ([#1313](https://github.com/Devolutions/IronRDP/issues/1313)) ([a71567e35e](https://github.com/Devolutions/IronRDP/commit/a71567e35e47a6eba8493c00933e0b66e0c63d5b)) 

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.5.0...ironrdp-pdu-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- Implement `Default` trait on `ExtendedClientOptionalInfoBuilder` (#891) ([ae052ed835](https://github.com/Devolutions/IronRDP/commit/ae052ed83598ad1f4ad7038b153e3c5398d2a738)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Update timezone info to use i32 bias (#921) ([119c7077c9](https://github.com/Devolutions/IronRDP/commit/119c7077c98e4b43021619378c4f251c1f95ae17)) 

  Switches `bias` from an unsigned to a signed integer.
  This matches the updated specification from Microsoft.

### <!-- 7 -->Build

- Bump thiserror to 2.0 ([b4fb0aa0c7](https://github.com/Devolutions/IronRDP/commit/b4fb0aa0c79aa409d1b6a5f43ab23448eede4e51)) 

- Bump der-parser to 10.0 ([03cac54ada](https://github.com/Devolutions/IronRDP/commit/03cac54ada50fae13d085b855a9b8db37d615ba8)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.4.0...ironrdp-pdu-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Make client_codecs_capabilities() configurable ([783702962a](https://github.com/Devolutions/IronRDP/commit/783702962a2e842f9d5046ac706048ba124e1401)) 

- BitmapCodecs struct ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 

### <!-- 4 -->Bug Fixes

- Fix possible out of bound indexing in RFX module (#724) ([9f4e6d410b](https://github.com/Devolutions/IronRDP/commit/9f4e6d410b631d8a6b0c09c2abc0817a83cf042b)) 

  An index bound check was missing in the RFX module. Found by fuzzer.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.3.1...ironrdp-pdu-v0.4.0)] - 2025-03-12

### <!-- 4 -->Bug Fixes

- TS_RFX_CHANNELT width/height SHOULD be within range ([097cdb66f9](https://github.com/Devolutions/IronRDP/commit/097cdb66f965700caeea5659ff7fe4a129b84838)) 

  According to the specification, the value does not need to be in the range:
  https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/4060f07e-9d73-454d-841e-131a93aca675
  
  (the ironrdp-server can send larger values)

### Refactor

- [**breaking**] Remove RfxChannelWidth and RfxChannelHeight structs ([7cb1ac99d1](https://github.com/Devolutions/IronRDP/commit/7cb1ac99d189cdcaa17fa17e51f95be630e9982e)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.3.0...ironrdp-pdu-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.2.0...ironrdp-pdu-v0.3.0)] - 2025-03-07

### <!-- 4 -->Bug Fixes

- Make AddressFamily parsing resilient (#672) ([6b4af94071](https://github.com/Devolutions/IronRDP/commit/6b4af94071bfb0adff482cc33b75e6c37ff6e10f)) 

- Fix FastPathHeader minimal size (#687) ([3b9d558e9c](https://github.com/Devolutions/IronRDP/commit/3b9d558e9c958297d9654861df515e2a8658bf8b)) 

  The minimal_size() logic didn't properly take into account the overall
  PDU size.
  
  This fixes random error/disconnect in client.

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.1.2...ironrdp-pdu-v0.2.0)] - 2025-01-28

### <!-- 1 -->Features

- ClientLicenseInfo and other license PDU-related adjustments (#634) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.1.1...ironrdp-pdu-v0.1.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
