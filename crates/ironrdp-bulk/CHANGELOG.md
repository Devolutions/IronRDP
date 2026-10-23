# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-bulk-v0.1.1...ironrdp-bulk-v0.1.2)] - 2026-10-23

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

- Decompress the compressed data PDUs of DRDYNVC version 3 ([#2173](https://github.com/Devolutions/IronRDP/issues/2173)) ([c3dd17e607](https://github.com/Devolutions/IronRDP/commit/c3dd17e6071342aa5f92b743a89a38382665eb64)) 

  ## Problem
  
  `DrdynvcClient` echoes the server's capabilities version. When a server
  offers version 3, the client therefore claims support for
  DYNVC_DATA_FIRST_COMPRESSED (0x06) and DYNVC_DATA_COMPRESSED (0x07),
  MS-RDPEDYC 2.2.3.3 and 2.2.3.4. It could not decode either:
  `DrdynvcServerPdu::decode` had no arm for them. A server that used them
  would break the DRDYNVC channel.
  
  ## Fix
  
  This PR implements version 3 rather than answering with version 2. The
  first commit downgraded the answer; the second replaces the downgrade
  with real support.
  
  - **`ironrdp-bulk`: new `rdp8` module.** An RDP 8.0 bulk decompressor
  (MS-RDPEGFX 3.1.9.1) that is `no_std` (alloc only) and has no
  dependencies. It accepts `PACKET_COMPR_TYPE_RDP8` (0x04) and
  `PACKET_COMPR_TYPE_RDP8_LITE` (0x06), the type DVC segments carry. It
  has two profiles. `Rdp8Decompressor::new()` is the graphics profile: a
  2.5 MB history, 64 MiB per segment and 256 MiB per multipart message,
  matching the existing decoder. `Rdp8Decompressor::new_lite()` is the RDP
  8.0 Lite profile defined in MS-RDPEDYC 2.2.3.3: an 8,192-byte history
  and maximum match distance, at most 8,192 uncompressed bytes per
  segment, and a single segment per message. The existing ZGFX decoder in
  `ironrdp-graphics` couldn't be reused: it needs `std` and `num-derive`
  (a proc-macro), which core-tier `ironrdp-dvc` cannot depend on, and it
  rejects 0x06.
  - **`ironrdp-dvc`: decode and reassemble.** New `DataFirstCompressedPdu`
  and `DataCompressedPdu` types, added to the `#[non_exhaustive]`
  `DrdynvcDataPdu`, decode on both the client and server side.
  `CompleteData` decompresses each block before reassembly, so one message
  can mix compressed and uncompressed blocks (MS-RDPEDYC 2.2.3). Each
  channel gets its own RDP 8.0 Lite decompression context (MS-RDPEDYC
  3.1.5.2.5), created on the channel's first compressed block. A block
  that makes a message longer than its declared `Length` is now an error
  for both compressed and uncompressed data; before, the uncompressed path
  logged it and dropped the message. The `DataFirstCompressedPdu` and
  `DataCompressedPdu` constructors document the Lite payload format, so
  the graphics pipeline's RDP 8.0 output is not mistaken for a valid
  payload.
  - **Client capabilities.** A version 3 offer is answered with version 3
  again. `DrdynvcServer` still offers version 2 and now tolerates
  compressed data from clients.
  
  ## Tests
  
  - **Spec samples:** the MS-RDPEDYC 4.3.3 and 4.3.4 sample PDUs decode
  and encode byte for byte (`dvc::data_compressed`). The client
  reassembles them into one message (`dvc::client`). The spec's samples
  decompress to 3,192 bytes while declaring 3,195, so the test supplies
  the last 3 bytes in an uncompressed DYNVC_DATA block.
  - **Per-channel history:** a test checks that one channel's compressed
  data cannot refer back to another channel's history.
  - **Length overruns:** tests check that a compressed first block, or a
  later uncompressed block, that runs past the declared `Length` is an
  error.
  - **Lite limits:** `ironrdp-bulk` unit tests check that the Lite profile
  rejects a match distance of 8,193 (accepted by RDP 8.0), keeps exactly
  the last 8,192 bytes of history, caps segments at 8,192 bytes, and
  rejects multipart messages.
  - **Against the EGFX decoder:**
  `bulk::rdp8_decodes_windows_egfx_captures` decodes the Windows ZGFX
  captures already in `ironrdp-graphics` and gets identical output.
  `bulk::rdp8_decodes_ironrdp_compressor_output` round-trips IronRDP's
  ZGFX compressor output, including multipart messages.
  - **Fuzzing:** a new `bulk_rdp8` fuzz target and `bulk_decompress_rdp8`
  oracle require that any input the graphics decoder accepts decodes
  identically with the new decompressor's RDP 8.0 profile. The same input
  also runs through the Lite profile. Regression seeds are under
  `test_data/fuzz_regression/bulk_decompress_rdp8`. A local run of 300,000
  mutated captures and spec samples found no disagreement.
  - **Unit tests:** `ironrdp-bulk` unit tests cover literals, matches, raw
  byte runs, RDP 8.0 Lite, multipart messages, history carried across
  messages, and malformed input.
  
  ## Checks
  
  - **Tests:** `cargo test -p ironrdp-testsuite-core -p ironrdp-bulk`
  passes: 1897 and 154 tests.
  - **Format:** `cargo xtask check fmt -v` passes.
  - **Lints:** `cargo clippy -D warnings` is clean on `ironrdp-bulk`,
  `ironrdp-dvc`, `ironrdp-testsuite-core` and `ironrdp-fuzzing` (all
  targets), and on the `no_std` builds of `ironrdp-bulk` and
  `ironrdp-dvc`. `fuzz/` builds with the new target. The workspace-wide
  `cargo xtask check lints` could not finish locally because the
  `libopus_sys` build script fails on this machine.
  
  ## Not verified / follow-up
  
  - **Live server:** not tested against a live Windows server sending
  compressed DVC data.
  - **Duplicate decoder:** EGFX still uses its own decoder in
  `ironrdp-graphics`. Moving it onto `ironrdp_bulk::rdp8` would remove the
  duplication, but it is a performance-sensitive path and should come with
  benchmarks in a separate change.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-bulk-v0.1.0...ironrdp-bulk-v0.1.1)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Gate alloc-dependent modules behind the alloc feature ([#1279](https://github.com/Devolutions/IronRDP/issues/1279)) ([18a430a51a](https://github.com/Devolutions/IronRDP/commit/18a430a51aca07aa45db4642df5c932ef65d2016)) 

- Off-by-one in forward match loop causes panic during compression ([#1293](https://github.com/Devolutions/IronRDP/issues/1293)) ([0dd7c94ba2](https://github.com/Devolutions/IronRDP/commit/0dd7c94ba22e9bd11b4ea36fd03af3bfcccecab8)) 

### <!-- 7 -->Build

- Bump criterion from 0.5.1 to 0.8.1 ([#1184](https://github.com/Devolutions/IronRDP/issues/1184)) ([d92dd382b3](https://github.com/Devolutions/IronRDP/commit/d92dd382b3fbaa163f355f6489db45ca8a3e7498)) 


