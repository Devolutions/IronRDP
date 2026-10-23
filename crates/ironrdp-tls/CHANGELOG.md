# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.2...ironrdp-tls-v0.2.3)] - 2026-10-23

### <!-- 1 -->Features

- Apply certificate validation to gateways ([#1775](https://github.com/Devolutions/IronRDP/issues/1775)) ([312d4466e7](https://github.com/Devolutions/IronRDP/commit/312d4466e7cae15d45c73a4ffcb4ae730d5e6a30)) 

  Apply the RDP client's certificate-validation policy and callback to
  every
  gateway HTTPS connection.
  
  Existing gateway callers retain their compatibility default.

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

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b)) 



## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.1...ironrdp-tls-v0.2.2)] - 2026-07-10

### <!-- 1 -->Features

- Expose negotiated TLS version and cipher suite ([#1384](https://github.com/Devolutions/IronRDP/issues/1384)) ([8f76260ea7](https://github.com/Devolutions/IronRDP/commit/8f76260ea753f546a577ad7a1176a5740adc94cf)) 

  Adds a backend-neutral way to query the TLS parameters negotiated for an established
  ironrdp-tls::TlsStream, enabling downstream diagnostic tooling to report the negotiated
  protocol version and cipher suite alongside the existing certificate information.

- Gate native backends behind Cargo features ([#1338](https://github.com/Devolutions/IronRDP/issues/1338)) ([f7e6106e0f](https://github.com/Devolutions/IronRDP/commit/f7e6106e0f293c1e0f8129be82aa2d86737ba92a)) 


- Make the rustls crypto provider selectable ([#1387](https://github.com/Devolutions/IronRDP/issues/1387)) ([d767d99032](https://github.com/Devolutions/IronRDP/commit/d767d990325448bf3385974da7ea9b6dcc477673)) 

  Makes the ironrdp-tls rustls backend’s crypto provider selectable at compile time by restructuring Cargo features, avoiding forcing a single provider onto downstreams via tokio-rustls default features.



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.0...ironrdp-tls-v0.2.1)] - 2026-05-27

### <!-- 7 -->Build

- Bump tokio from 1.50.0 to 1.52.1 ([#1219](https://github.com/Devolutions/IronRDP/issues/1219)) ([d3e673b455](https://github.com/Devolutions/IronRDP/commit/d3e673b455ec817df7590cef27e598c8517828ae)) ([#1223](https://github.com/Devolutions/IronRDP/issues/1223)) ([8bf140f49d](https://github.com/Devolutions/IronRDP/commit/8bf140f49d3bee952e395ffeb514a27c4725eb15)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.4...ironrdp-tls-v0.2.0)] - 2025-12-18

### <!-- 1 -->Features

- [**breaking**] Return x509_cert::Certificate from upgrade() ([#1054](https://github.com/Devolutions/IronRDP/issues/1054)) ([bd2aed7686](https://github.com/Devolutions/IronRDP/commit/bd2aed76867f4038c32df9a0d24532ee40d2f14c)) 

  This allows client applications to verify details of the certificate,
  possibly with the user, when connecting to a server using TLS.

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.3...ironrdp-tls-v0.1.4)] - 2025-08-29

### <!-- 7 -->Build

- Bump tokio from 1.46.1 to 1.47.0 (#893) ([5d513dcf09](https://github.com/Devolutions/IronRDP/commit/5d513dcf099505d4d52fe25884dc019590bc751e)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.1...ironrdp-tls-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

### <!-- 7 -->Build

- Bump tokio from 1.42.0 to 1.43.0 (#650) ([ff6c6e875b](https://github.com/Devolutions/IronRDP/commit/ff6c6e875b4c2dce7ec109c3721739f86a808a31)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.0...ironrdp-tls-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
