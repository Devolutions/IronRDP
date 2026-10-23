# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.10.0...ironrdp-blocking-v0.11.0)] - 2026-10-23

### <!-- 0 -->Security

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



## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.9.0...ironrdp-blocking-v0.10.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-connector` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.8.0...ironrdp-blocking-v0.9.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- [**breaking**] Make Framed::read_exact crate-private ([#1247](https://github.com/Devolutions/IronRDP/issues/1247)) ([d02d24aad4](https://github.com/Devolutions/IronRDP/commit/d02d24aad44039c0425a022f1bd9677800706cea)) 


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.7.0...ironrdp-blocking-v0.8.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.3.1...ironrdp-blocking-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.3.0...ironrdp-blocking-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.2.1...ironrdp-blocking-v0.3.0)] - 2025-01-28

### <!-- 4 -->Changed

- Remove unmatched parameter from `Framed::read_by_hint` function ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3))

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.2.0...ironrdp-blocking-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
