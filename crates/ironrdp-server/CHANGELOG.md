# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.14.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.13.0...ironrdp-server-v0.14.0)] - 2026-10-27

### <!-- 0 -->Security

- Bump clap from 4.6.4 to 4.6.5 in the patch group across 1 directory ([#1537](https://github.com/Devolutions/IronRDP/issues/1537)) ([5f779f5caf](https://github.com/Devolutions/IronRDP/commit/5f779f5caf937183bcef49359475eb91591ca5a2)) 

  Bumps the patch group with 1 update in the / directory:
  [clap](https://github.com/clap-rs/clap).
  
  Updates `clap` from 4.6.4 to 4.6.5
  <details>
  <summary>Release notes</summary>
  <p><em>Sourced from <a
  href="https://github.com/clap-rs/clap/releases">clap's
  releases</a>.</em></p>
  <blockquote>
  <h2>v4.6.5</h2>
  <h2>[4.6.5] - 2026-07-31</h2>
  <h3>Fixes</h3>
  <ul>
  <li><em>(help)</em> Correctly mark which <code>value_names</code> are
  optional with <code>num_args</code></li>
  </ul>
  </blockquote>
  </details>
  <details>
  <summary>Changelog</summary>
  <p><em>Sourced from <a
  href="https://github.com/clap-rs/clap/blob/master/CHANGELOG.md">clap's
  changelog</a>.</em></p>
  <blockquote>
  <h2>[4.6.5] - 2026-07-31</h2>
  <h3>Fixes</h3>
  <ul>
  <li><em>(help)</em> Correctly mark which <code>value_names</code> are
  optional with <code>num_args</code></li>
  </ul>
  </blockquote>
  </details>
  <details>
  <summary>Commits</summary>
  <ul>
  <li><a
  href="https://github.com/clap-rs/clap/commit/[[c8c935504a3f1a015470199ae82c3cb124b5b1d5](https://github.com/Devolutions/IronRDP/commit/c8c935504a3f1a015470199ae82c3cb124b5b1d5)](https://github.com/Devolutions/IronRDP/commit/[c8c935504a3f1a015470199ae82c3cb124b5b1d5](https://github.com/Devolutions/IronRDP/commit/c8c935504a3f1a015470199ae82c3cb124b5b1d5))"><code>c8c9355</code></a>
  chore: Release</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/[[af74def7141738c51ace956f58eb32dd94e998ab](https://github.com/Devolutions/IronRDP/commit/af74def7141738c51ace956f58eb32dd94e998ab)](https://github.com/Devolutions/IronRDP/commit/[af74def7141738c51ace956f58eb32dd94e998ab](https://github.com/Devolutions/IronRDP/commit/af74def7141738c51ace956f58eb32dd94e998ab))"><code>af74def</code></a>
  docs: Update changelog</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/[[c96f222c35c4ef4bd3ab9927809b2724532a8f6e](https://github.com/Devolutions/IronRDP/commit/c96f222c35c4ef4bd3ab9927809b2724532a8f6e)](https://github.com/Devolutions/IronRDP/commit/[c96f222c35c4ef4bd3ab9927809b2724532a8f6e](https://github.com/Devolutions/IronRDP/commit/c96f222c35c4ef4bd3ab9927809b2724532a8f6e))"><code>c96f222</code></a>
  Merge pull request <a
  href="https://redirect.github.com/clap-rs/clap/issues/6368">#6368</a>
  from truffle-dev/fix/fish-env-escaping</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/[[49a05cdc99c2151cdd48d5ec4c974151d21c026e](https://github.com/Devolutions/IronRDP/commit/49a05cdc99c2151cdd48d5ec4c974151d21c026e)](https://github.com/Devolutions/IronRDP/commit/[49a05cdc99c2151cdd48d5ec4c974151d21c026e](https://github.com/Devolutions/IronRDP/commit/49a05cdc99c2151cdd48d5ec4c974151d21c026e))"><code>49a05cd</code></a>
  fix(complete): Two-pass quote fish env-completer</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/[[e791004a5bbbfe5418fe573a4db777df24d72490](https://github.com/Devolutions/IronRDP/commit/e791004a5bbbfe5418fe573a4db777df24d72490)](https://github.com/Devolutions/IronRDP/commit/[e791004a5bbbfe5418fe573a4db777df24d72490](https://github.com/Devolutions/IronRDP/commit/e791004a5bbbfe5418fe573a4db777df24d72490))"><code>e791004</code></a>
  test(complete): Snapshot fish env quoting cases</li>
  <li>See full diff in <a
  href="https://github.com/clap-rs/clap/compare/clap_complete-v4.6.4...clap_complete-v4.6.5">compare
  view</a></li>
  </ul>
  </details>
  <br />
  
  
  [![Dependabot compatibility
  score](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=clap&package-manager=cargo&previous-version=4.6.4&new-version=4.6.5)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)
  
  Dependabot will resolve any conflicts with this PR as long as you don't
  alter it yourself. You can also trigger a rebase manually by commenting
  `@dependabot rebase`.

- Send periodic Heartbeat PDUs on the message channel ([#1842](https://github.com/Devolutions/IronRDP/issues/1842)) ([8f57691a9a](https://github.com/Devolutions/IronRDP/commit/8f57691a9a6e497388dc2824bffe93eeed7bb698)) 

  The server never sends Server Heartbeat PDUs (MS-RDPBCGR 2.2.16.1), even
  though the client half of the feature is in place: HeartbeatPdu
  encode/decode landed with #1814 and clients decode and ignore them. This
  adds the send half, opt-in.

- Recognize the Initiate Multitransport Response on the message channel ([#1964](https://github.com/Devolutions/IronRDP/issues/1964)) ([6aebe6d0b1](https://github.com/Devolutions/IronRDP/commit/6aebe6d0b11c4aa3c99252bd986e138e43756a6d)) 

  ## Summary
  
  handle_message_channel_data unconditionally decoded every PDU received
  on the MCS message channel as an AutoDetectRspPdu, per a comment
  claiming the channel "currently carries only the auto-detect response."
  That is no longer accurate: once a server sends an Initiate
  Multitransport Request, the client answers on this same channel with an
  Initiate Multitransport Response (MS-RDPBCGR 2.2.15.2) to report whether
  it could establish the sideband transport.
  
  MultitransportResponsePdu already has full Encode/Decode support in
  ironrdp-pdu (with success()/abort() constructors), but has zero
  consumers anywhere in ironrdp-server. In practice every such response
  from a real client failed to decode as AutoDetectRspPdu (its
  securityHeader carries SEC_TRANSPORT_RSP, not SEC_AUTODETECT_RSP) and
  was dropped with an "Unhandled MCS message channel PDU" warning:
  legitimate protocol traffic logged as an error.
  
  Reproduced against a real Windows client (mstsc) that offered UDP
  multitransport but could not establish it: the raw bytes it sent decode
  exactly as MultitransportResponsePdu { security_header: TRANSPORT_RSP,
  request_id, hr_response: E_ABORT }.
  
  ## Changes
  
  - Added `ironrdp_pdu::rdp::message_channel::ClientMessageChannelPdu`, an
  enum with Encode/Decode covering the two PDUs a client sends on this
  channel. Decode peeks at the Basic Security Header flags and dispatches:
  SEC_TRANSPORT_RSP to MultitransportResponsePdu, anything else to
  AutoDetectRspPdu, so a malformed PDU is reported against the type its
  header names.
  - handle_message_channel_data decodes that type and has a Multitransport
  arm that logs the response at debug level (ordinary protocol traffic,
  not an error), with the same wording the acceptor uses for this PDU,
  instead of falling through to the warn!.
  - Six tests in ironrdp-testsuite-core (tests/pdu/message_channel.rs):
  recognition of both PDUs, a round trip, a truncated transport response
  reported as one, a payload without SEC_TRANSPORT_RSP handed to the
  auto-detect decoder, and too-short input.
  
  ## Test plan
  
  cargo xtask check fmt/lints/tests/typos/locks all pass.

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

- Apply the connection policy to connections an embedder accepts ([#2116](https://github.com/Devolutions/IronRDP/issues/2116)) ([350520684c](https://github.com/Devolutions/IronRDP/commit/350520684c028f9d50343f64f6afb93475b7ac33)) 

  This is the follow-up I described on #1934 (comment 5815037145).
  
  ConnectionPolicy only reaches connections that RdpServer::run accepts on
  the TCP socket it binds. An embedder that accepts elsewhere and calls
  run_connection or run_connection_with gets none of it. lamco-rdp-server
  accepts on TCP, Unix sockets, AF_VSOCK and a WebSocket listener that
  terminates TLS itself, so a second client waits unanswered behind a live
  session, and a takeover can't wait for CredSSP the way run's does,
  because the negotiation that does the gating is private.
  
  What changes:
  
  ConnectionSource is a trait with one method, accept, that returns an
  IncomingConnection. That is the stream, how its TLS is handled
  (TransportTls), the peer and local addresses when the transport has
  them, and a tag the embedder can use to find its own record of the
  connection again.

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

- Let an authenticated connection preempt an existing session ([#1476](https://github.com/Devolutions/IronRDP/issues/1476)) ([5198cde0f7](https://github.com/Devolutions/IronRDP/commit/5198cde0f73485f3fbd999ea7c49293c86b80387)) 

  Adds opt-in session preemption so a new RDP client can replace an active connection.

- Add ConnectionPolicy to reject a second connection ([#1913](https://github.com/Devolutions/IronRDP/issues/1913)) ([f7732a1639](https://github.com/Devolutions/IronRDP/commit/f7732a16392b5857a698182ffd37221da1010a2a)) 

- Add ironrdp-server integration for AUDIO_INPUT ([#1946](https://github.com/Devolutions/IronRDP/issues/1946)) ([5b139ba3df](https://github.com/Devolutions/IronRDP/commit/5b139ba3df92b05154bcedc5bc13b2c0b61866d8)) 

  ## Summary
  
  RdpeaiServerFactory (ironrdp-server/src/rdpeai.rs) is a thin factory
  trait mirroring RdpdrServerFactory and SoundServerFactory:
  build_backend() returns the RdpeaiServerBackend the embedding
  application supplies. attach_channels registers RdpeaiServer on the DVC
  stack alongside AInput, DisplayControl, Echo, and RdpeiServer when a
  factory is configured. RdpServerBuilder::with_rdpeai_factory() follows
  with_rdpdr_factory's shape.

- Add bandwidth-measure generation counter ([#1963](https://github.com/Devolutions/IronRDP/issues/1963)) ([ded2caa118](https://github.com/Devolutions/IronRDP/commit/ded2caa1184fd3b2671b9725ff26052383a0be16)) 

  ## Summary
  
  - `autodetect_bandwidth_handle()` ([#1734](https://github.com/Devolutions/IronRDP/issues/1734)) exposes the latest Bandwidth
    Measure figure, but a consumer doing its own smoothing/filtering on
    top of it (e.g. rejecting noisy low samples unless real demand
    existed) needs to know when a *new* measurement window has closed,
    not just what the figure currently reads. The figure itself is a bad
    freshness signal: a quiet link reads the same low value for several
    consecutive windows in a row, so diffing it cannot tell "fresh
    window, same result" apart from "stale, no new window yet".
  - Adds `autodetect_bandwidth_generation`, mirroring
    `autodetect_bandwidth`'s exact shape: an `Arc<AtomicU32>` field,
    `autodetect_bandwidth_generation_handle()` accessor, and
    `with_autodetect_bandwidth_generation_handle()` builder method. It
    increments on every completed Bandwidth Measure transaction,
    successful or not: a completed-but-unusable window still needs to
    advance the generation so a consumer's own stale-demand bookkeeping
    gets cleared instead of carrying over into the next window. The
    server increments it with `Release` after storing the figure, so a
    consumer that loads it with `Acquire` reads a bandwidth value at
    least as new as the generation it saw.
  - Along the way, upgrades the existing bandwidth-measured debug log
    from just the computed `bandwidth_kbps` to the whole Bandwidth
    Measure Results response, which carries the raw
    `byte_count`/`time_delta_ms` inputs. Useful on its
    own for anyone diagnosing why the figure looks noisy against a
    bursty traffic source (as this crate's own damage-driven server
    usage does): the raw inputs make it immediately visible whether a
    low reading came from a genuinely idle window or a short one that
    happened to close early.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. testsuite-core
  covers the generation handle's default and injection; the increment
  itself
  runs in the private connection loop, which no harness reaches.
  
  ## Notes
  
  No public API break: both changes are purely additive to
  `RdpServerBuilder` and `RdpServer`.

- Let RdpServerDisplay report its monitor count ([#1918](https://github.com/Devolutions/IronRDP/issues/1918)) ([25331598f3](https://github.com/Devolutions/IronRDP/commit/25331598f37f49573205c54ee45c878d291317f2)) 

  ## Summary
  
  - DisplayControlBackend (ironrdp-server's internal DisplayControlHandler
  implementor) never overrode capabilities(), so it fell through to the
  single-monitor default regardless of how many monitors the consumer's
  RdpServerDisplay actually serves.
  - Added a default RdpServerDisplay::monitor_count() method (returns 1,
  matching today's behavior), fetched once alongside size() before the
  dynamic channels are attached.
  - Threaded the count into DisplayControlBackend so its capabilities()
  now reports the real value instead of the hardcoded literal.
  - A consumer serving more than one monitor can override monitor_count()
  to report the real total.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  - Builds on #1917 (DisplayControlHandler::capabilities()), now merged;
  rebased onto master.
  - Additive only: one new trait method with a default body, one new
  field, one call-site change. No existing implementor is affected unless
  it opts in.

- Default ConnectionPolicy to Preempt under Hybrid ([#1934](https://github.com/Devolutions/IronRDP/issues/1934)) ([9cb20336f6](https://github.com/Devolutions/IronRDP/commit/9cb20336f6f586cd5c9c06b6907d1b3f2e51b405)) 

- [**breaking**] Post-authentication connection binding for multi-user servers ([#1413](https://github.com/Devolutions/IronRDP/issues/1413)) ([15bed4ee0e](https://github.com/Devolutions/IronRDP/commit/15bed4ee0ebef4b851c024edfcd4b1675d6e9ee8)) 

  ## Summary
  
  This replaces and consolidates #1409, #1410, #1411, and #1412 into a
  single branch from `rcarmo:wrdp`.
  
  The split PR heads use `wrdp/*` branch names, but the fork now uses the
  canonical `wrdp` branch. Because Git cannot keep both `refs/heads/wrdp`
  and `refs/heads/wrdp/*` at the same time, this PR carries the updated
  commits and review fixes together.
  
  ## Why
  
  `wrdp` follows the `xrdp-sesman` multi-user architecture model, so it
  needs a way to hand authenticated credentials from the protocol
  handshake into the per-user session binder.
  
  ## Changes
  
  - Adds an async post-auth connection binder hook in `ironrdp-server`.
  - Scopes reactivation credential reuse to the current TCP connection.
  - Exposes CredSSP delegated credentials to server consumers via the same
  `Credentials` shape used by ClientInfo.
  - Restores the client-facing denial PDU on credential
  reject/backend-error paths before returning the error.
  - Clarifies binder docs so they refer to authenticated credentials being
  available, not only to configured validator acceptance.
  - Skips binder re-execution during deactivation/reactivation so
  resize/reactivation does not restart per-user binding or replace
  handlers mid-session.
  
  ## Copilot review follow-up
  
  This includes fixes for the Copilot comments from the superseded PRs:

- Let the embedder decide when a connection gets an auto-reconnect cookie ([#2046](https://github.com/Devolutions/IronRDP/issues/2046)) ([bf019ca502](https://github.com/Devolutions/IronRDP/commit/bf019ca502d1e52616853a5ded68c885cec1e322)) 

  - The server sends an auto-reconnect cookie (MS-RDPBCGR 2.2.4.2) to
  every connection at activation and reissues it hourly (3.3.6.2).
  MS-RDPBCGR 5.5 has the cookie sent as soon as the user has been
  authenticated, but an embedder that accepts a connection before the user
  has proved anything, for instance to draw its own logon screen after a
  credential validator hands off, gives that connection a cookie it can
  later present in place of credentials and skip the screen.
  - `RdpServerBuilder::with_auto_reconnect_on_request` and
  `RdpServer::set_auto_reconnect_on_request` switch to issuing cookies
  only on request: Through `AutoReconnectCookieHandle::set` once the
  embedder has authenticated the user, or to a client that came back with
  a cookie the server verified. The hourly reissue then goes only to a
  connection that holds a cookie. Off by default, so nothing changes
  unless an embedder opts in.
  - `ConnectionInfo::auto_reconnect` tells the embedder that a connection
  came back with a verified cookie, so it can admit it without asking
  again.
  - With cookies on request, the embedder vouches for each authenticated
  connection. The cookie it issues replaces the previous one at once,
  which is how the regeneration on connect in MS-RDPBCGR 5.5 is met for
  that connection. A connection the embedder never vouches for neither
  receives a cookie nor invalidates one, so for such a connection the
  regeneration doesn't happen. That is deliberate, because a connection
  that has proved nothing must not be able to invalidate the legitimate
  user's cookie. A vouch belongs to the connection that was current when
  it was sent. If that connection is replaced before the server reads the
  vouch, the vouch is dropped and the replacement gets no cookie until the
  embedder vouches for it. `RdpServer::set_auto_reconnect_on_request`
  documents this contract.

- Let the display decline the Display Control channel ([#1985](https://github.com/Devolutions/IronRDP/issues/1985)) ([7b976a6965](https://github.com/Devolutions/IronRDP/commit/7b976a6965c96c76ff949313e174ea55dada8c44)) 

  ## Summary
  
  - A server that cannot resize, for example one mirroring a fixed-size
  physical screen, had no way to stop ironrdp-server from offering the
  Display Control channel. MS-RDPEDISP section 1.3 defines no reject
  message: When the requested configuration is not possible, the server
  simply does not update the session. A client that sends a layout the
  server cannot apply is left waiting.
  - Added a default RdpServerDisplay::offers_display_control() method
  (returns true, matching today's behavior), queried once per connection
  before the dynamic channels are attached.
  - When it returns false, attach_channels skips DisplayControlServer, so
  the channel is not offered on that connection.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Two end-to-end
  tests in ironrdp-testsuite-extra connect a real client that registers a
  Display Control channel and an echo channel: A display that offers
  Display Control gets the channel created on the client, and a display
  that declines it does not. The declining test fails if the gate is
  removed.
  
  ## Notes
  
  - This is the rule gnome-remote-desktop applies: It creates its Display
  Control channel only in extend (virtual monitor) mode.
  - Additive only: One new trait method with a default body. No existing
  implementor is affected unless it opts in.
  - #2116 adds tests in ironrdp-testsuite-extra/tests/e2e.rs that build
  TestDisplay in two places, and this PR adds the offers_display_control
  field to it. The two apply cleanly as text, but whichever lands second
  needs that field set to true in those two initializers. I'd like this
  one to land first, and I'll rebase #2116 afterward.

- Pass the client's monitors to the connection handler ([#2129](https://github.com/Devolutions/IronRDP/issues/2129)) ([da166aa488](https://github.com/Devolutions/IronRDP/commit/da166aa4883a4868638d3c65ecf44b8b3895acfb)) 

  The acceptor decodes a client's GCC Client Monitor Data block and keeps
  nothing from it, so a server that can present a variable number of
  outputs has no way to present as many as the client has. The Bitmap
  capability that request_initial_size sees doesn't help: it reported the
  server's own 1280x800 for a client that asked for 2560x800. A server
  that presents every output of its host to a client with one monitor ends
  up with a canvas twice that client's size.
  
  This keeps the list in AcceptorResult::client_monitors, in the order the
  client sent it with each monitor's position and primary flag, and passes
  it on in ConnectionInfo::client_monitors, which
  ConnectionHandler::on_connection_info already receives once per
  connection. The list is empty when the client sent no block.
  ConnectionInfo is non_exhaustive and ConnectionInfo::new leaves the
  field empty, so nothing that builds it or reads it today changes.
  
  The tests drive the acceptor from the Connection Request to Connected
  and check that AcceptorResult::client_monitors keeps the list in the
  order sent, and that a client without the block announces none. They're
  in ironrdp-testsuite-core, so the normal test run covers them.

- Measure bandwidth on the UDP tunnel when EGFX uses it ([#2031](https://github.com/Devolutions/IronRDP/issues/2031)) ([cb7ca37680](https://github.com/Devolutions/IronRDP/commit/cb7ca3768092fcc43a09a1b401cdd8abb873db42)) 

  ## Summary
  
  - Once EGFX moves to the UDP transport, no large write crosses TCP any
  more, so the bracketed bandwidth measurement never runs again and the
  reported figure stays at its pre-migration value for the rest of the
  session.
  - Large EGFX batches on the tunnel are now bracketed the same way, with
  Bandwidth Measure Start and Stop (0x0014 / 0x0429) in
  RDP_TUNNEL_SUBHEADERs (MS-RDPBCGR 1.3.9, 2.2.14.1.2). Each sub-header is
  the request itself: Its SubHeaderLength and SubHeaderType are the
  request's headerLength and headerTypeId (MS-RDPEMT 2.2.1.1.1), so the
  request's first two bytes are not repeated in SubHeaderData. Each goes
  in a Tunnel Data PDU with no data of its own, so the client's count
  (only data after the tunnel header, 3.2.5.14) is exactly the graphics
  between them.
  - Bandwidth Measure Results the client returns in tunnel sub-headers are
  handled like the ones on the message channel; they are decoded from the
  whole sub-header, which is the response structure; the shared handling
  moved into one method.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Tests in
  `ironrdp-testsuite-core` cover the sub-header encoding of Start and the
  decoding of results arriving in tunnel sub-headers into the auto-detect
  manager, through a private `__test` feature on `ironrdp-server` that
  exposes those two functions, as well as the sub-header wire layout
  against the spec.
  
  ## Notes
  
  Builds on #2021 (bracketed measurement), #1954 and #2030, which have all
  merged.

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

- [**breaking**] Stop deriving pipe MaximumPacketSize from device speed ([#1871](https://github.com/Devolutions/IronRDP/issues/1871)) ([6a5d7c23fe](https://github.com/Devolutions/IronRDP/commit/6a5d7c23fe312b6c7b01f25c6a8e4c436309df00)) 

  `interface_information` derived
  `TS_USBD_PIPE_INFORMATION.MaximumPacketSize` from
  the announced device speed. The client opens the pipe and reports the
  size it
  actually established in `TS_USBD_PIPE_INFORMATION_RESULT`, so the
  request field
  only states a preference, and a Windows server sends zero.
  `select_configuration`
  and `select_interface` now send zero and no longer take a speed.
  
  `USB_DEVICE_CAPABILITIES` ([MS-RDPEUSB] 2.2.11) expresses only full and
  high speed,
  so the derived value was also unreachable for anything faster.
  
  `ironrdp-server` tracked the announced speed only to feed those two
  calls, so
  `UsbCapabilities` and its accessor go with them; nothing in its public
  API changes.
  
  Observed against a Windows server redirecting a USB 3.1 flash drive from
  FreeRDP:
  it sends `MaximumPacketSize` 0 for both bulk pipes and receives 1024
  back for each.

- Bound accept_finalize so a wedged client cannot hold the server ([#1890](https://github.com/Devolutions/IronRDP/issues/1890)) ([a8b4b2a7ed](https://github.com/Devolutions/IronRDP/commit/a8b4b2a7ed6f86a16a31e304538823520db0d830)) 

- A clipboard failure must not disconnect the session ([#1980](https://github.com/Devolutions/IronRDP/issues/1980)) ([124406d881](https://github.com/Devolutions/IronRDP/commit/124406d881af377cb339e8c131831f69e3cf7e45)) 

  Two halves of the same problem: a rejected clipboard operation is both
  invisible to the backend waiting on it and fatal to the session.
  
  ## `ironrdp-server`: a clipboard error ends `client_loop`
  
  `dispatch_server_events` propagates every `CliprdrServer` error with
  `?`:
  
  ```rust
  ClipboardMessage::SendFileContentsRequest(request) => cliprdr.request_file_contents(request),
  ...
  }
  .map_err_kind("failed to send clipboard event", ServerErrorKind::Pdu)?;
  ```
  
  That error unwinds through `dispatch_events` → `client_loop` →
  `client_accepted` → `accept_finalize`, and the accept loop logs
  `"Connection error"`, resets the static channels and calls
  `on_disconnected`. **The session is torn down.**
  
  The things that can trigger it are all ordinary: a file contents request
  that became stale because the remote clipboard changed under it, a
  capability that was never negotiated, `MAX_PENDING_FILE_REQUESTS`
  backpressure, a malformed flag combination. None is a reason to
  disconnect a peer whose display, audio and input are fine.
  
  Three lines above, `ClipboardMessage::Error` already takes the opposite
  view:
  
  ```rust
  ClipboardMessage::Error(error) => {
      error!(?error, "Handling clipboard event");
      continue;
  }
  ```
  
  This makes the two paths agree.
  
  ## `ironrdp-cliprdr`: a rejection strands its caller
  
  `request_file_contents` has ten rejection paths and all of them return a
  bare `PduError`. The error carries no stream id, so an embedder that
  surfaces it has no way to map it back to the transfer that failed —
  whatever is awaiting that stream just hangs until an unrelated timeout
  fires.
  
  The crate already solves this elsewhere. `FormatListResponse::Fail`
  does:
  
  ```rust
  // Notify backend for each pending request so it can clean up
  // (e.g. reject pending download promises in WASM).
  for stream_id in stream_ids {
      self.backend.on_file_contents_response(FileContentsResponse::new_error(stream_id));
  }
  ```
  
  Every rejection in `request_file_contents` now does the same before
  returning. **The `Err` is unchanged** — this is strictly additive for
  existing callers.
  
  ## Tests
  
  -
  `crates/ironrdp-testsuite-core/tests/clipboard/file_contents_state_machine.rs`
  — `a_rejected_request_fails_its_stream_on_the_backend` (out-of-bounds
  index) and
  `a_request_rejected_before_ready_fails_its_stream_on_the_backend`
  (`require_ready`, the path most likely to strand a caller). Both assert
  the error is still returned *and* that the backend saw an error response
  for the right stream id.
  - `crates/ironrdp-server/src/server.rs` —
  `a_refused_clipboard_message_does_not_end_the_client_loop` drives
  `dispatch_server_events` with a request the channel refuses and asserts
  `RunState::Continue`.
  
  All 211 existing clipboard tests and the 15 `ironrdp-server` lib tests
  pass unchanged.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- Drop RDPSND waves that arrive before the channel negotiates ([#1990](https://github.com/Devolutions/IronRDP/issues/1990)) ([2e872b085b](https://github.com/Devolutions/IronRDP/commit/2e872b085b504cf3b9ce9994237270f71a35d13a)) 

  The server event channel outlives a single connection. When a client
  with audio disconnects, waves its sound handler already queued stay in
  the channel and become the first events the next connection dispatches,
  before that connection's RdpsndServer has received the client's formats.
  wave() then fails in version() with "client format not yet received",
  and the map_err_kind at the dispatch site ends the new client loop, so a
  disconnect of an audio-enabled client can turn into a failed connect for
  whoever comes next. The per-batch WAVE_KEEP trim only removes waves
  beyond the newest four, so it does not cover this.
  
  This adds RdpsndServer::is_ready() and drops waves and volume changes
  with a debug log while the channel is not ready. A volume change fails
  the same way before negotiation, since set_volume() needs the client's
  format flags. A wave has nowhere to go before negotiation, and after a
  failed negotiation (no common format, or the handler's start failing) it
  never will.
  
  A test drives the handshake and checks is_ready() before negotiation,
  after a successful one, with no common format, and when start fails. It
  also checks that wave() and set_volume() fail before negotiation, and
  that wave() goes through once is_ready() holds.

- Keep ConnectionHandler reachable during a Preempt race ([#2065](https://github.com/Devolutions/IronRDP/issues/2065)) ([458545e6d5](https://github.com/Devolutions/IronRDP/commit/458545e6d5c5963a3673e7f686c0bdee3848b58b)) 

- Drop the previous session's queued events before every connection ([#2034](https://github.com/Devolutions/IronRDP/issues/2034)) ([2c08bda7c5](https://github.com/Devolutions/IronRDP/commit/2c08bda7c5f9ad490e01f123b3be5521cf3c6e9d)) 

  ## Summary
  
  - `discard_stale_session_events` ran only before serving a preemption
  winner. After an ordinary disconnect, anything the session had queued
  but not consumed (EGFX frames, RDPSND waves, clipboard messages) stayed
  on the server-global event channel. `run()`'s idle select discards what
  it reads between connections, but a client reconnecting within
  milliseconds can win that race, and an embedder driving connections
  through `run_connection` has no idle loop at all.
  - Seen on a live server: a client dropped mid-write and reconnected 45
  ms later. The new connection's first write, 6 ms after its client loop
  started, was the previous session's 8.4 MB EGFX frame, framed for that
  session's graphics DVC while the new DRDYNVC had not opened any channel
  yet. The client failed with "access to non existing DVC channel" and
  dropped again.
  - The discard now also runs at the start of every connection, before
  negotiation. Events the new session produces come after its channels
  start, so none of them are affected. The same allowlist applies: `Quit`,
  `GetLocalAddr`, `SetCredentials`, `SetAutoReconnectCookie` survive.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. New end-to-end
  test `a_new_connection_starts_without_the_previous_sessions_events` in
  `ironrdp-testsuite-extra` queues a `Disconnect` event before the
  connection, serves one connection through `run_connection` with no
  `run()` idle loop in between, and checks that a real client still
  completes an echo round trip; it fails without the change.
  
  ## Notes
  
  An embedder that queues a per-session event before calling
  `run_connection` now has it dropped along with the stale ones; only the
  lifecycle allowlist carries into a connection, as it already did for a
  preemption winner.

- [**breaking**] Split per-connection state ([#2059](https://github.com/Devolutions/IronRDP/issues/2059)) ([0e9dc1f712](https://github.com/Devolutions/IronRDP/commit/0e9dc1f71237c75ba1f856b1996f5994931dd31e)) 

  Per-connection state moves out of `RdpServer` fields into a
  `ConnectionState`, created in `finalize_negotiated` (both entry paths
  reach it) and dropped with the connection. It holds the static channels,
  the heartbeat flag, the auto-detect manager, and the URBDRC interface
  allocator and request router. The manual resets go away, and the
  auto-detect baseline RTT no longer carries over between connections.

- Respect client frame acknowledgement limits ([#2102](https://github.com/Devolutions/IronRDP/issues/2102)) ([e4143f2630](https://github.com/Devolutions/IronRDP/commit/e4143f26306a6639a889d80c1c484ca69c365a8f)) 

  A client advertising a one-frame ACK window can currently be sent frames
  using the server's larger configured window. An AVC producer also
  receives the same `None` for a full window as for a permanent submission
  error, which makes ordinary backpressure difficult to handle without
  replacing the encoder.
  
  This PR is based directly on `master` and is independent of #1413. It
  contains no credential, binder or acceptor changes.
  
  * Apply the nonzero core Frame Acknowledge capability as a ceiling on
  the operator-configured EGFX window, including legacy graphics
  factories. Configured zero remains unlimited only when the client has no
  ceiling; resize preserves the negotiated ceiling.
  * Reject unknown/stale ACKs before they can suspend or mutate tracking.
  Preserve valid suspension/resumption for frames issued in the current
  channel generation, and clear channel-owned state on close.
  * Report client queue depth as `Unavailable`: our client counts frames,
  while the protocol field measures bytes. This is distinct from
  `Suspend`.
  * Add typed AVC420, AVC444 and AVC444v2 submission errors, including
  `Backpressured`, while retaining the existing `Option` methods as
  compatibility wrappers. Mixed-codec send APIs are unchanged.
  
  WRDP exposed the configured-three/client-one case during desktop
  dragging. These changes let the producer retain its encoder and
  retry/recover after an ACK instead of treating backpressure as encoder
  failure.
  
  Focused checks passed locally (Rust 1.96; the repository pins 1.94.1):
  
  ```
  cargo test -p ironrdp-egfx --lib                               # 55 passed
  cargo test -p ironrdp-testsuite-core --test integration_tests_core -- egfx::server  # 92 passed
  cargo test -p ironrdp-server --lib --features egfx -- frame_ack # 2 passed
  cargo test -p ironrdp-server --lib --features egfx -- gfx::tests # 2 passed
  cargo fmt --all
  git diff --check
  ```
  
  Regressions cover window/zero semantics, activation clamps, resize,
  legacy factories, stale suspend/resume ACKs across close/reopen,
  restored tracking after valid resume, and typed rejection without queued
  output. A read-only review caught the legacy-factory and suspended-ACK
  resume paths; both are corrected and covered here.
  
  Happy to adjust the additive error API to the project's preferred shape
  during review. This PR can be reviewed and merged separately from the
  session-binding work.
  
  ---------

- Measure bandwidth across a single large graphics write ([#2021](https://github.com/Devolutions/IronRDP/issues/2021)) ([b5bbd8a598](https://github.com/Devolutions/IronRDP/commit/b5bbd8a59829450cb85ca834836b2e030fb53dde)) 

  ## Summary
  
  - Continuous bandwidth measurement opened a fixed window of RTT ticks
  (Start, four ticks, Stop). The client counts the ordinary traffic
  between Start and Stop (MS-RDPBCGR 3.2.5.14), so a window over a quiet
  desktop timed idle time: on a 2 ms LAN the server reported a few hundred
  kbps, and mstsc told the user the network was slow.
  - When an EGFX write of at least 10 KiB goes out, the server now sends
  Bandwidth Measure Start immediately before it and Stop immediately
  after, on the same stream, so the client times a burst the link actually
  carried. At most one such measurement per second, and never while one is
  outstanding. gnome-remote-desktop measures the same way.
  - The tick window takes over whenever no bracketed measurement has
  started for eight RTT ticks, so a session without EGFX, or one whose
  only large frame was the initial render, keeps measuring.
  - A bandwidth measurement the client never answers, or a bracket whose
  Stop is never sent, is abandoned after the same 30 s as an unanswered
  RTT probe (`expire_stale_probes`), instead of stopping bandwidth
  measurement for the rest of the session. This also covers the tick
  window, which could already get stuck that way.
  - A Bandwidth Measure Results with a zero time delta now counts as one
  millisecond instead of discarding the figure: The client's timer has
  millisecond resolution, and a burst on a fast link routinely completes
  within one. A result with no bytes counted still clears the stored
  figure.
  - New public API: `AutoDetectManager::begin_bandwidth_measure` and
  `AutoDetectManager::end_bandwidth_measure`. Additive;
  `build_bandwidth_measure` is unchanged.
  
  ## Validation
  
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Nine new tests
  in `ironrdp-testsuite-core` cover the bracket, the size threshold, the
  pacing, the tick window staying closed while brackets recur and resuming
  when they stop, the expiry of an unended bracket and of an unanswered
  measurement, a slow tick window still reaching its Stop, and the
  zero-delta rule. The existing test that treated a zero time delta as
  unusable now checks a zero byte count instead.

- Send an AND mask with 32 bpp pointers ([#2108](https://github.com/Devolutions/IronRDP/issues/2108)) ([fb25b47c0c](https://github.com/Devolutions/IronRDP/commit/fb25b47c0ce22233b4992ec0e70560fb56b8f200)) 

  `UpdateEncoder` sends `RGBAPointer` and `LargePointer` shapes with an
  empty AND mask (`lengthAndMask = 0`). MS-RDPBCGR 2.2.9.1.1.4.4 makes the
  AND mask part of every color pointer, and Windows App for macOS (build
  68651) drops a 32 bpp pointer without one: its pointer never changes
  from the default arrow, whatever shape the server sends. mstsc and
  FreeRDP accept the empty mask, which is probably why this went
  unnoticed.
  
  This PR builds the mask from the XOR data's alpha channel:
  
  - A bit is set for each fully transparent pixel (alpha 0) and clear
  elsewhere. With a 32 bpp XOR mask the alpha channel already carries
  transparency, so the AND mask only has to agree with it.
  - Each scan line is padded to a 2-byte boundary, as the encoder's own
  `check_masks_alignment` requires.
  - Rows are in the same order as the XOR data. `ironrdp-graphics`'s
  `DecodedPointer` reads both masks in the same row order, as clients do
  generally.
  
  The helper lives in a new `encoder/pointer_mask.rs`, so the test suite
  can `include!` it the way it already does `encoder/fast_path.rs`. The
  `LargePointer` doc comment, which said "no AND mask", is updated.

- Drive clipboard channel timeouts ([#1979](https://github.com/Devolutions/IronRDP/issues/1979)) ([3db08ba869](https://github.com/Devolutions/IronRDP/commit/3db08ba86983ce35c9d36ad33f59ceb4b607dce8)) 

- [**breaking**] Pass a DisplayContext to RdpServerDisplay::updates ([#2073](https://github.com/Devolutions/IronRDP/issues/2073)) ([9e85975616](https://github.com/Devolutions/IronRDP/commit/9e85975616c14fc2f4ad422d3a53255a4fbee345)) 

- Avoid hang on empty QOIZ updates ([#2147](https://github.com/Devolutions/IronRDP/issues/2147)) ([b343a6f9d3](https://github.com/Devolutions/IronRDP/commit/b343a6f9d3fedd2306204f668f745bfc44411ade)) 

  The QOIZ decompressor allocated a zero-length output buffer for empty
  input and kept doubling it, so a malformed update could spin the client
  indefinitely.
  Start with a non-empty buffer and return only the decompressed bytes.
  
  Keep the existing zstd-safe backend and streaming format.
  Extract the compressor and decompressor behind test-only exports, and
  cover consecutive flushed updates, cross-update history, large payloads,
  empty input, and fixed streams from both encoders.
  Defer the pure-Rust backend migration until it can preserve these
  streaming and interoperability guarantees.

- [**breaking**] Pass a GfxContext to the GfxServerFactory build methods ([#2104](https://github.com/Devolutions/IronRDP/issues/2104)) ([7bca756755](https://github.com/Devolutions/IronRDP/commit/7bca756755ef856bd5aaaff97612bea875dbad5c)) 

  Follow-up to #2073, stack on it.
  
  Initial discussion:
  https://github.com/Devolutions/IronRDP/pull/2073#issuecomment-6021678246
  
  Channel backends are now built in `finalize_negotiated`, once a
  connection has authenticated, together with its `ConnectionState`.
  `GfxServerFactory`'s build methods take a `GfxContext` carrying the same
  auto-detect handles the display gets, and the connection's EGFX server
  reaches the display through `DisplayContext::gfx_handle`, replacing
  `RdpServer::gfx_handle()`.

- Advertise horizontal mouse wheel input ([#2236](https://github.com/Devolutions/IronRDP/issues/2236)) ([2110ab2572](https://github.com/Devolutions/IronRDP/commit/2110ab25728ca0b2f1ad79aa90b834af2d3de5c6)) 

  The server now sets `TS_INPUT_FLAG_MOUSE_HWHEEL` in its Input Capability
  Set (MS-RDPBCGR 2.2.7.1.6). A client MUST NOT send `PTRFLAGS_HWHEEL` to
  a server that does not set it (MS-RDPBCGR 2.2.8.1.1.3.1.1.3), so mstsc
  dropped horizontal wheel input although the server already decodes it
  into `MouseEvent::HorizontalScroll`.

- Do not cap the EGFX window with the core frame ack limit by default ([#2238](https://github.com/Devolutions/IronRDP/issues/2238)) ([0de133426b](https://github.com/Devolutions/IronRDP/commit/0de133426b8f7423f78dd6c63d8eba4f4d2365eb)) 

  The client's `maxUnacknowledgedFrameCount` from the core Frame
  Acknowledge Capability Set no longer caps the EGFX frame window by
  default. MS-RDPRFX 2.2.1.3 defines that value for surface-command frames
  acknowledged with TS_FRAME_ACKNOWLEDGE_PDU; EGFX has its own
  acknowledgement and `queueDepth` (MS-RDPEGFX 2.2.2.13, 3.2.5.13).
  Windows 10 mstsc sends 1, which allowed one EGFX frame per round trip.

- Send RemoteFX in Stream Surface Bits ([#2229](https://github.com/Devolutions/IronRDP/issues/2229)) ([e20a214703](https://github.com/Devolutions/IronRDP/commit/e20a2147038b8226701d9cf6b59d2c895fa51480)) 

  RemoteFX data now goes in a Stream Surface Bits command, as MS-RDPRFX
  3.1.8.3.1 requires, when the client sets `STREAM_SURFACE_BITS` in its
  Surface Commands capability (MS-RDPBCGR 2.2.7.2.9). Otherwise it keeps
  Set Surface Bits.
  
  ImageRemoteFX keeps Set Surface Bits: Windows 10 mstsc reports "An
  internal error has occurred" when ImageRemoteFX data comes in Stream
  Surface Bits. Other codecs do not change.
  
  Because the two codecs now use different commands, the server picks
  RemoteFX over ImageRemoteFX whatever the order of the client's Bitmap
  Codecs list; before, the last one in the list won.



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
