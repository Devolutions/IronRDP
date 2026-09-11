# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpeudp-tokio-v0.1.0)] - 2026-09-11

### <!-- 0 -->Security

- Add the async driver for the RDP-UDP transport ([#1687](https://github.com/Devolutions/IronRDP/issues/1687)) ([9aa8a4b063](https://github.com/Devolutions/IronRDP/commit/9aa8a4b063d71a1b98f2306de045c32c08bc70f9)) 

  Fifth of six. **Stacked on the connection state machine PR**, and needs
  the
  RDPEMT PR too.
  
  Wires the sans-I/O connection and the RDPEMT tunnel to a real socket. A
  single
  task owns the `UdpSocket` and the `RdpeudpConnection` and runs a select
  loop over
  three sources: datagrams arriving, plaintext the TLS layer wants to
  send, and
  timer expiry. It is the only place in the stack that reads a clock.
  
  `RdpeudpStream` gives tokio-rustls something to wrap, bridging the
  driver to TLS
  through a pair of buffers so the TLS layer never learns it is running
  over UDP.
  `connect_udp` and `accept_udp` run the whole sequence: RDP-UDP
  handshake, TLS,
  RDPEMT tunnel, then a data pump. `UdpTransport` implements `FramedRead`
  and
  `FramedWrite`, so the session layer reaches it through the same traits
  as the TCP
  path.
  
  ## Three things this layer has to get right
  
  **A spawned task that is dropped keeps running.** All three handles
  (driver,
  read pump, write pump) live in a guard that aborts on drop and can be
  taken
  back when the task is meant to be awaited. That matters because the TLS
  upgrade and the tunnel handshake both sit between spawning the driver
  and
  returning the transport, and both propagate with `?`. Making the guard
  structural means each early return cleans up without every path having
  to
  remember. `shutdown()` follows the same three-task shape in reverse: the
  read
  pump gets EOF once the shared I/O bridge is marked closed, the write
  pump
  exits on its own once the send channel is dropped, and the driver is
  aborted
  last since it may still be sitting in its select loop.
  
  **The read buffer is bounded.** Ceasing to read from the socket is the
  backpressure the protocol expects rather than a drop policy: unread
  datagrams
  stay in the socket buffer, acknowledgements stop, and the receive window
  closes
  on the sender, which is what [MS-RDPEUDP2] 3.1.1.2.2 has it for. The
  half that is
  easy to leave out is resuming, so `poll_read` wakes the driver once it
  has taken
  bytes out.
  
  **An empty tunnel PDU is not end of stream.** `Framed` reads a
  zero-length result
  as EOF, and [MS-RDPEMT] 2.2.2.3 puts no minimum on `HigherLayerData`.
  [MS-RDPBCGR] 1.3.9 sends the four Continuous Auto-Detection messages
  "encapsulated in the RDP_TUNNEL_SUBHEADER structure ... over the
  sideband
  channels that are in active use", and those carriers have nothing behind
  the
  subheaders.
  
  ## On the cookie hash
  
  The SHA-256 the version 3 SYN carries is derived here from the tunnel
  config,
  which already holds the security cookie. That keeps `ironrdp-rdpeudp`
  free of a
  cryptographic dependency and means a caller cannot supply a hash over a
  different
  cookie than the tunnel will present.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks` plus `cargo xtask wasm
  check`
  
  Unit tests for the stream adapter, the framing adapter, task lifetime,
  backpressure, and clean-vs-truncated tunnel PDU framing, plus nine
  full-stack
  loopback tests in `ironrdp-testsuite-extra` that run a client and server
  through handshake, TLS, tunnel and bidirectional data over localhost,
  including IPv4 and IPv6, an oversized-payload rejection, a
  caller-supplied
  certificate verifier, and a failed-reconnect state check.
  
  One caveat on those loopback tests, since it caught me out: localhost
  carries an
  oversized datagram perfectly well, so they cannot detect an MTU
  violation. The
  datagram sizes are asserted in the connection tests instead.

### <!-- 1 -->Features

- MS-RDPEUDP version 1/2 reliable data transfer ([#1919](https://github.com/Devolutions/IronRDP/issues/1919)) ([8fd2b2ed3e](https://github.com/Devolutions/IronRDP/commit/8fd2b2ed3ec19a2560badfc0f36b3f32015aa066)) 

  Every Windows host tested on a LAN answers the RDPUDP SYN with uUdpVer
  0x0002: they speak MS-RDPEUDP (version 1/2), not MS-RDPEUDP2 (version
  3). A client that only implements the version 3 framing therefore always
  times out on the UDP handshake and falls back to TCP, so the
  multitransport path never carries data against Windows.
  
  This adds the version 1/2 reliable data path to the sans-I/O connection
  state machine, negotiated from the SYN+ACK and reusing the existing
  send/receive windows, RTT estimator, loss detection and timers:
  
  - pdu: RDPUDP_SOURCE_PAYLOAD_HEADER (2.2.2.4) and the data payload on a
  version 1/2 Source Packet. DATA datagrams decode; FEC (lossy) datagrams
  are rejected.
  - connection: NegotiatedParams records the wire format. Version 1/2 uses
  a single Source sequence space from ISN+1; snCoded numbers each
  transmission, snSourceAck drives an ACK vector walked down from the
  highest received sequence (3.1.1.4), ACK-of-ACKs resets the receiver's
  vector base (2.2.2.6), a Congestion Notify is answered with CWR at most
  once per RTT (3.1.1.8), and ACKDELAYED marks timer-driven ACKs
  (3.1.6.3). Timers take the version's floors.
  - The 6-bit ACK-vector run length is encoded as count minus one on the
  wire, which MS-RDPEUDP 2.2.2.7.1 leaves ambiguous but Windows requires:
  element 0x00 acknowledges one Source Packet and 0x02 acknowledges three.
  Reading it literally leaves the first Source Packet unacknowledged
  forever, collapsing the congestion window.
  - seq: 32-bit wrap-aware sequence reconstruction and narrowing.
  - ConnectionConfig::offer_version lets a client offer version 2 or 1
  outright (no cookieHash), which is what the measured hosts answer
  immediately.
  - RdpeudpConnection::v1_stats exposes retransmit, ACK and datagram
  counters for observability.
  
  Verified against the spec's 4.2.1 example capture and new state-machine
  tests; the existing tests are unchanged.
  
  **Note on MS-RDPEUDP 4.2.3.** The spec's worked example annotates ACK
  vector element 0x04 as "4 datagrams received," a literal reading of the
  six-bit run length. Captures against Windows Server show the field is
  the run length minus one: after one Source Packet the server answers
  `snSourceAck = 1` with element 0x00, after four it answers `snSourceAck
  = 4` with 0x03. Read literally, the first is an empty run and the second
  never acknowledges seq 1, which stalls the sender. This implementation
  follows the wire, not the example. Evidence:
  https://gist.github.com/AKolenda/51931b094dd454ef0eb1430503629c8a

### <!-- 4 -->Bug Fixes

- Detect driver exit during the async handshake wait ([#1704](https://github.com/Devolutions/IronRDP/issues/1704)) ([f24685cde6](https://github.com/Devolutions/IronRDP/commit/f24685cde66bd290a3f7d35f506b9d8840452b92)) 

  ## Summary
  - connect_udp and accept_udp_inner both waited on connected_notify
    without racing it against the driver task's own completion
  - a driver that exits early with a real error (socket failure, fatal
    protocol error) went unnoticed until the configured handshake
    timeout elapsed, or until the caller's outer accept_timeout
    canceled the whole accept sequence for accept_udp_inner, which had
    no timeout of its own on this wait at all
  - either way the caller got a generic HandshakeTimeout instead of the
    driver's actual error
  - both call sites now race connected_notify against the driver's
    JoinHandle directly, via a shared driver_exit_during_handshake
    helper that turns a join result into the right UdpTransportError
  - added AbortOnDrop::handle_mut to borrow the wrapped JoinHandle for
    a select! branch without taking ownership of it
  
  ## Validation
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Four new
  tests cover the shared helper's three outcomes (real driver error,
  clean exit treated as failure, panic) and the select! race itself
  (a driver that dies immediately is detected well inside a 5-second
  bound instead of falling through to a 60-second stand-in timeout).
  
  ## Notes
  Surfaced by Copilot's second review pass on #1687, posted 11 minutes
  before that PR merged; not addressed there.

- [**breaking**] Thread server_cert_verifier through MultitransportBootstrap::connect ([#1706](https://github.com/Devolutions/IronRDP/issues/1706)) ([0047a28740](https://github.com/Devolutions/IronRDP/commit/0047a287407bbf06e3c1f8331a299294d4eddf4c)) 

  ## Summary
  - connect_udp already accepted a server_cert_verifier to opt into real
    TLS certificate validation, added in #1687's own first review round
  - MultitransportBootstrap builds its UdpTransportConfig internally
    and never set that field, so a caller going through the high-level
    bootstrap API had no way to reach the verification connect_udp
    itself already supported
  - connect() now takes server_cert_verifier as a fourth parameter and
    forwards it the same way connection_config already is
  - breaking change to this crate's public API (adds a required
    parameter); the crate has no external consumers yet
  
  ## Validation
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Added a
  full-stack test proving the verifier is actually consulted through
  the bootstrap path, reusing the AlwaysRejectVerifier double from the
  sibling connect_udp test; mutation-verified by temporarily reverting
  the fix and confirming the new test fails.
  
  ## Notes
  Surfaced by Copilot's second review pass on #1687, posted 11 minutes
  before that PR merged; not addressed there. Third of three follow-up
  PRs (see #1704 and #1705 for the other two).

- [**breaking**] Treat a full send buffer as backpressure, not a fatal error ([#1705](https://github.com/Devolutions/IronRDP/issues/1705)) ([c0267494ab](https://github.com/Devolutions/IronRDP/commit/c0267494aba535088d74f3bfed6929a176f2f01c)) 

  ## Summary
  - RdpeudpConnection::send's SendBufferFull is documented as transient,
    but the async driver treated it as fatal, tearing down an otherwise
    healthy connection under sustained write load
  - send() took the payload by value with no way to hand it back on
    error, so the rejected bytes were also gone by the time the error
    propagated
  - send()'s signature now returns the rejected data alongside the
    error on every rejection, via a new SendError type; breaking change
    to this crate's public API, but it has no external consumers yet
  - the driver holds a backpressured write in a new pending_write field
    instead of dropping it, and retries once poll_transmit has moved
    entries out of the send buffer (an incoming ACK or a retransmit
    both qualify)
  - the write-data branch of the driver's select loop stops pulling new
    data out of the shared write buffer while a write is already
    pending, so a retry never gets interleaved out of order
  
  ## Validation
  `cargo xtask check fmt/lints/tests/typos/locks` all pass. Four new
  driver-level tests build a real, in-memory-established connection and
  mutation-verify the fix: filling the send buffer to its bound no
  longer errors, the rejected payload comes back intact, a retry
  delivers once drain_transmits frees room, and a retry into an
  already-closed connection does not turn a clean shutdown into an
  error.
  
  ## Notes
  Surfaced by Copilot's second review pass on #1687, posted 11 minutes
  before that PR merged; not addressed there. Second of three follow-up
  PRs (see #1704 for the first).

- Harden TLS sideband setup ([#1811](https://github.com/Devolutions/IronRDP/issues/1811)) ([7292970955](https://github.com/Devolutions/IronRDP/commit/729297095564a9640183556078cf8726dd3afeed)) 

  Extract the reusable rustls verifier and client-config builder so the
  RDPEUDP2 reliable-UDP TLS sideband shares the primary transport's
  certificate-validation policy and callback semantics without selecting a
  TLS stream backend.
  
  Build callback-free client configurations on the blocking pool, and run
  callback-based handshakes on a dedicated blocking thread, so synchronous
  platform trust-store access and certificate decisions cannot starve a
  current-thread RDPEUDP driver. Bound the TLS and RDPEMT establishment
  phases with dedicated timeout errors. A TLS timeout cancels the
  connection attempt but cannot cancel a synchronous certificate callback
  that is already running.
  
  Close `SharedIo` and wake parked readers when a driver is dropped before
  releasing its stream, preventing aborted connection attempts from
  stranding detached TLS work.
  
  Pass `UdpTlsConfig` directly through `MultitransportBootstrap::connect`,
  and document that `S_OK` is sent only after Soft-Sync negotiation.


