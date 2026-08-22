# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpeudp-v0.1.0)] - 2026-08-22

### <!-- 0 -->Security

- [**breaking**] Add the connection state machine ([#1681](https://github.com/Devolutions/IronRDP/issues/1681)) ([1723385068](https://github.com/Devolutions/IronRDP/commit/1723385068ee3a4be635c199c752eb2bbd183f1e)) 

  Fourth of six. Filed against master directly; #1626, #1627 and #1679
  have
  all merged.
  
  Adds the sans-I/O `RdpeudpConnection` and the machinery it drives: send
  and
  receive windows, loss detection, NewReno congestion control, an RFC 6298
  RTT
  estimator, the timer table, the reliability controller that matches
  retransmissions to the packets they replace, and sequence-number
  reconstruction
  from 16-bit wire values.
  
  This is the largest PR in the stack. I looked at splitting the state
  machine
  from the reliability primitives, but `connection.rs` drives all of them
  and the
  tests span both, so the seam would have been artificial.
  
  ## Sans-I/O
  
  No clock, no socket. Time arrives as a `MonotonicInstant` argument and
  outgoing
  packets leave as `Transmit` values. That keeps it testable without a
  network,
  and avoids `std::time::Instant`, whose `now` panics on
  `wasm32-unknown-unknown`.
  
  **`MonotonicInstant` is defined here, and #1530 adds one to
  `ironrdp-connector`
  for the same reason.** Two copies of one abstraction is a wart.
  `ironrdp-core`
  looks like the right home to me, and I raised it on #140; happy to move
  it
  wherever you prefer, in this PR or a follow-up.
  
  ## Behaviour that is required rather than chosen
  
  **Version 3 in the SYN.** That is the version [MS-RDPEUDP] 1.3.2.2 and
  the
  2.2.2.9 table tie to the MS-RDPEUDP2 data transfer. Version 2 selects
  the
  MS-RDPEUDP one, which this crate does not implement. Version 3 requires
  the
  SHA-256 of the `securityCookie` in the client's SYN (2.2.2.9), so
  `ConnectionConfig` carries it, `connect` refuses without it, and the
  server
  performs the check 3.1.5.1.1 asks for.
  
  **The handshake retransmits.** 1.3.1 delivers the SYN, SYN+ACK and ACK
  by
  persistent retransmits whatever mode the transport runs in; 3.1.5.4.1
  gives up
  after between three and five unanswered tries. A repeated datagram from
  the peer
  draws a repeat of ours rather than an error, including a SYN+ACK
  arriving after
  we have moved on to v2, which is how a client learns its final ACK was
  lost.
  
  **The delayed-ACK timer tracks the RTT instead of a fixed duration.**
  MS-RDPEUDP2 3.1.5.2 gives half the round trip time as the receiver's
  default.
  The handshake round trip seeds the existing RFC 6298 estimator (skipped
  when
  the handshake datagram was retransmitted, per Karn's algorithm), and the
  computed timeout is clamped to the 50-200ms band [MS-RDPEUDP] 3.1.6.3
  gives a
  version-2 connection, since MS-RDPEUDP2 itself states no floor or cap of
  its
  own.
  
  **`UdpVersion` widens from a closed enum to a newtype carrying the raw
  wire
  value.** This is a breaking change to the public API #1627 shipped.
  MS-RDPEUDP
  1.7 and 3.1.5.1.3 require a responder to negotiate down to a version it
  supports when the peer advertises one it does not recognize;
  hard-failing
  decode on an unrecognized value made that MUST clause unsatisfiable.
  Every
  call site already used the named constants, so this is the only
  consequential
  part of the change.
  
  **`error.rs` and its `Cargo.toml`/README wiring are back.** #1627
  correctly
  dropped them at its own narrower, PDU-only scope; this PR's state
  machine is
  what actually needs them.
  
  **The receive window advances on both events in 3.1.1.2.2**, not only on
  AckOfAcks. The first one is what fires on a connection that is losing
  nothing;
  without it the window fills one window in and stops accepting.
  
  **AckOfAcks carries our own lowest unacknowledged sequence number**
  (2.2.1.2.4,
  3.1.5.3). It is the only thing that can move a receiver past a packet
  the sender
  gave up on, since the retransmission carries a fresh `DataSeqNum` and
  the
  original is never filled.
  
  **Writes are split to the MTU.** MS-RDPEUDP2 does not segment:
  3.1.1.2.4.2
  forwards each packet's payload straight up and nothing marks a first or
  last
  fragment. `ChannelSeqNum` looks like it would serve but 3.1.5.5 gives it
  a
  different job, matching a retransmission to the packet it replaces. So
  anything
  longer than one packet is split before it becomes packets.
  
  **Dummy packets** are accounted for by the transport and their contents
  dropped,
  per 3.1.1.1.5.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks`
  
  179 rdpeudp tests in `ironrdp-testsuite-core` with this PR applied, plus
  179
  inline unit tests across the crate's components. The connection tests
  cover a
  clean transfer longer than the window, a loss in the middle of one,
  handshake
  retransmission to the give-up limit, the version and cookie negotiation
  paths,
  decoding an unrecognized `uUdpVer`, the ack-delay timeout's RTT-tracking
  and
  clamping behaviour, and the review round's findings: an out-of-range ACK
  no
  longer discards outstanding data, the ACK vector respects its 127-entry
  wire
  limit, `log_window_size` is validated, the ACK builders encode real
  timestamps and gaps, the handshake's Karn's-algorithm check is ordered
  correctly, the retransmit timer restarts on real progress, `accept`
  completes
  the negotiate-down behaviour, and `send` enforces a buffer bound.

### <!-- 1 -->Features

- Add the RDP-UDP crate and the v1 handshake PDUs ([#1627](https://github.com/Devolutions/IronRDP/issues/1627)) ([0f252c2715](https://github.com/Devolutions/IronRDP/commit/0f252c2715490841793924f989b92e32fe54bfa0)) 

  # feat(rdpeudp): add the RDP-UDP crate and the v1 handshake PDUs
  
  Second of six, and the first of the RDP-UDP transport
  @mamoreau-devolutions
  asked for on #140. Independent of the RDPEMT PR; the two can land in
  either
  order.
  
  Adds `ironrdp-rdpeudp` with the structures [MS-RDPEUDP] section 2.2
  defines for
  the three-way handshake: the FEC header and flags, the SYN and extended
  SYN
  payloads that negotiate initial sequence numbers, MTU and protocol
  version, the
  ACK vector with its run-length encoding, the AckOfAcks header, the
  correlation
  ID payload, and the composite datagram that assembles them in the order
  2.2.2
  requires.
  
  ## The convention that matters when reading this
  
  Everything here is **big-endian**, and the diagrams number bits **most
  significant first**. Section 2.2: "all of the messages written to the
  network or
  read from the network MUST be in network byte order."
  
  MS-RDPEUDP2, which the data transfer switches to, is little-endian and
  numbers
  diagram bits least significant first. The two documents are opposite on
  both
  counts. That is why the two wire formats are split across two PRs rather
  than
  reviewed together.
  
  ## Two readings I would particularly like checked
  
  **The ACK flag does not always announce an ACK vector.** 2.2.2.1 defines
  it that
  way, but 3.1.5.1.3 builds the SYN+ACK as a plain SYN with the flag set
  and
  `snSourceAck` filled in, and the capture in 4.1.2 confirms it: `uFlags`
  is
  `0x0005` and the SYNDATA payload follows the 8-byte header directly,
  with no
  vector between them. So on a SYN the flag says only that `snSourceAck`
  is
  meaningful. `encode` refuses a SYN carrying a vector rather than writing
  bytes a
  peer would read as the start of SYNDATA.
  
  **The ACK vector's element layout and padding** were settled against the
  ACK
  packet capture in 4.2.3 (`00 01 04 00`): a big-endian `uAckVectorSize`,
  the
  state in the top two bits of each element, and padding out to a DWORD
  boundary.
  
  Both captures are decoded byte by byte in `pdu_v1_datagram.rs` rather
  than only
  round tripped against our own encoder. A mistake made symmetrically in
  encode
  and decode is invisible to a round trip, so the captures are the only
  thing that
  can catch it.
  
  ## What is not here
  
  No connection state machine, no v2 data transfer format. Those are the
  next two
  PRs in the stack.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks`
  
  43 tests in `ironrdp-testsuite-core`, including the section 4.1.1, 4.1.2
  and
  4.2.3 captures decoded whole.

- Add the RDP-UDP2 data transfer PDUs and packet framing ([#1679](https://github.com/Devolutions/IronRDP/issues/1679)) ([0eeed99a9a](https://github.com/Devolutions/IronRDP/commit/0eeed99a9ac186fd324360c1f39a541d5f8eea21)) 

  # feat(rdpeudp): add the RDP-UDP2 data transfer PDUs and packet framing
  
  Third of six. Builds on the RDPEMT tunnel crate ([#1626](https://github.com/Devolutions/IronRDP/issues/1626)) and the v1
  handshake
  PDUs ([#1627](https://github.com/Devolutions/IronRDP/issues/1627)), both merged, adding files to the crate the second one
  creates.
  
  Adds the structures [MS-RDPEUDP2] section 2.2.1 defines for data
  transfer, which
  is where a connection goes once the handshake settles on protocol
  version 3: the
  packet header and flags, the ACK and acknowledgment vector payloads, the
  OverheadSize, DelayAckInfo and AckOfAcks control payloads, DataHeader
  and
  DataBody, the composite packet, and the PacketPrefixByte framing from
  2.2.1.3.
  
  ## Why this is a separate PR from the v1 PDUs
  
  The two documents take **opposite conventions**. MS-RDPEUDP is
  big-endian and
  numbers diagram bits most significant first; MS-RDPEUDP2 is
  little-endian and
  numbers them least significant first. Reading a v2 diagram with v1
  habits
  produces a plausible and wrong layout for several fields, so I would
  rather each
  be reviewed against one document at a time.
  
  Four fields where that difference bites, all settled against worked
  examples
  rather than against the diagrams:
  
  **The prefix byte.** `Packet_Type_Index` occupies bits 1 to 4 and
  `Short_Packet_Length` bits 5 to 7. The worked example in 3.1.1.1.5.1
  gives
  `PacketPrefixByte = 0x10` for a 10-byte packet, which is
  `Packet_Type_Index = 8`
  (dummy) only under least-significant-first numbering. Read the other way
  round
  the same byte says something else entirely.
  
  **The ACK payload nibbles.** 2.2.1.2.1 puts `numDelayedAcks` at bits
  48-51 and
  `delayAckTimeScale` at 52-55, so the count is the low nibble.
  
  **The acknowledgment vector field order.** 2.2.1.2.6 places `TimeStamp`
  (bits
  24-47) before `SendAckTimeGapInMs` (48-55).
  
  **DelayAckInfo.** `MaxDelayedAcks` is a whole byte, not a nibble.
  
  ## On the flag set
  
  The header carries exactly the six flags 2.2.1.1 lists, every one of
  them
  announcing a payload. There is deliberately no `CN`, `CWR` or `DUMMY`
  here: the
  first two belong to MS-RDPEUDP's own `RDPUDP_FEC_HEADER`, and a dummy
  packet is
  marked by `Packet_Type_Index` 8 in the prefix byte, one layer down. With
  no
  standalone flags the field is derived entirely from which payloads are
  present,
  so a caller cannot set a bit that describes nothing.
  
  ## What is not here
  
  No connection state machine. That is the next PR.
  
  ## Test plan
  
  `cargo xtask check fmt/lints/tests/typos/locks`
  
  122 rdpeudp tests in `ironrdp-testsuite-core` with this PR applied, plus
  44
  inline `#[cfg(test)]` tests in the crate itself covering packet-prefix
  framing
  and header/flags internals, including the 3.1.1.1.5.1 worked example.

- Add fuzz targets for the RDP-UDP transport and RDPEMT tunnel ([#1707](https://github.com/Devolutions/IronRDP/issues/1707)) ([2191cf47a1](https://github.com/Devolutions/IronRDP/commit/2191cf47a1027e59527ab23bf8d7d63d610cd6c6)) 

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

### <!-- 4 -->Bug Fixes

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


