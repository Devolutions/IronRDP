# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.10.0...ironrdp-tokio-v0.10.1)] - 2026-10-23

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



## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.9.0...ironrdp-tokio-v0.10.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-async` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.8.0...ironrdp-tokio-v0.9.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Upgrade sspi


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.7.0...ironrdp-tokio-v0.8.0)] - 2025-12-18

### <!-- 1 -->Features

- Add MovableTokioFramed for Send+!Sync context ([#1033](https://github.com/Devolutions/IronRDP/issues/1033)) ([966ba8a53e](https://github.com/Devolutions/IronRDP/commit/966ba8a53e43a193271f40b9db80e45e495e2f24)) 

  The `ironrdp-tokio` crate currently provides the following two
  `Framed<S>` implementations using the standard `tokio::io` traits:
  - `type TokioFramed<S> = Framed<TokioStream<S>>` where `S: Send + Sync +
  Unpin`
  - `type LocalTokioFramed<S> = Framed<LocalTokioStream<S>>` where `S:
  Unpin`
  
  The former is meant for multi-threaded runtimes and the latter is meant
  for single-threaded runtimes.
  
  This PR adds a third `Framed<S>` implementation:
  
  `pub type MovableTokioFramed<S> = Framed<MovableTokioStream<S>>` where
  `S: Send + Unpin`
  
  This is a valid usecase as some implementations of the `tokio::io`
  traits are `Send` but `!Sync`. Without this new third type, consumers of
  `Framed<S>` who have a `S: Send + !Sync` trait for their streams are
  forced to downgrade to `LocalTokioFramed` and do some hacky workaround
  with `tokio::task::spawn_blocking` since the defined associated futures,
  `ReadFut` and `WriteAllFut`, are neither `Send` nor `Sync`.

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.5.1...ironrdp-tokio-v0.6.0)] - 2025-07-08

### Build

- Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.5.0...ironrdp-tokio-v0.5.1)] - 2025-07-08

### <!-- 1 -->Features

- Add async `ReqwestNetworkClient::send` method (#859) ([7e23a8bb97](https://github.com/Devolutions/IronRDP/commit/7e23a8bb97991d0e24e65d77a11d9854492ee024)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.4.0...ironrdp-tokio-v0.5.0)] - 2025-06-06

### <!-- 4 -->Bug Fixes

- [**breaking**] Adjust reqwest-related features (#812) ([9408789491](https://github.com/Devolutions/IronRDP/commit/9408789491b3e09b69e0aaa03fd215326b624ec0)) 

  - Remove `reqwest` from the default feature set.
  - Disable default TLS backend.
  - Add `reqwest-rustls-ring` to enable rustls + ring backend.
  - Add `reqwest-native-tls` to enable native-tls backend.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.3.0...ironrdp-tokio-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- Add reqwest feature (#734) ([032c38be92](https://github.com/Devolutions/IronRDP/commit/032c38be9229cfd35f0f6fc8eac5cccc960480d3)) 

## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.2...ironrdp-tokio-v0.2.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.1...ironrdp-tokio-v0.2.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.0...ironrdp-tokio-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
