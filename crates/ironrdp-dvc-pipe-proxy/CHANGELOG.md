# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.5.0...ironrdp-dvc-pipe-proxy-v0.5.1)] - 2026-10-23

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

- Send pipe proxy data on its own channel ([#2209](https://github.com/Devolutions/IronRDP/issues/2209)) ([10199db3b0](https://github.com/Devolutions/IronRDP/commit/10199db3b0aeca629adc02234ffdbb570bb77d43)) 

  `DvcNamedPipeProxy` bridges a dynamic virtual channel to a local named
  pipe. When the server opens the channel, `DvcProcessor::start` receives
  its channel ID and starts a worker for it. The worker reads what a local
  client writes to the pipe, encodes it as DVC data with
  `encode_dvc_messages(channel_id, ...)`, and hands the encoded messages
  to the write callback given to `DvcNamedPipeProxy::new`, which takes a
  channel ID and the messages.
  
  The worker encoded the messages for the real channel ID but called the
  callback with channel ID 0. `ironrdp-client` uses the callback's channel
  ID to route the messages to the transport that carries that channel,
  either the main connection or a multitransport tunnel selected by
  Soft-Sync. Before routing, it wraps them with
  `DvcMessageBatch::try_new(channel_id, messages)`, which decodes each
  message and checks that it is DVC data addressed to that channel. With 0
  passed alongside messages encoded for another channel, the check fails
  with "DVC message channel ID does not match its batch", the error is
  returned from the session loop, and the RDP session ends. Any pipe proxy
  therefore dropped the connection as soon as its pipe client wrote its
  first message; in particular, `ironrdp-agent now` commands could not
  reach the remote agent over `Devolutions::Now::Agent`.
  
  The worker now passes the channel ID it was started with, which is the
  ID its messages are encoded for, so the batch check accepts them and
  they are routed with the channel. The other write callbacks wired into
  `ironrdp-client`, the DVC COM plugin host and the RDPEWA client, already
  pass the channel ID they encode with.
  
  The pipe proxy tests in `ironrdp-testsuite-extra` checked only that pipe
  data reached the callback as at least one message. A new test starts the
  proxy on a non-zero channel, writes to the pipe, and checks that the
  callback receives that channel ID and that `DvcMessageBatch::try_new`
  accepts the messages. The existing tests now also decode the forwarded
  DVC data PDUs and check that they carry exactly the bytes written to the
  pipe. Like the existing pipe proxy tests, these run on Windows only.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.4.1...ironrdp-dvc-pipe-proxy-v0.5.0)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.

- Remove trailing punctuation from log messages ([#1380](https://github.com/Devolutions/IronRDP/issues/1380)) ([f38554277a](https://github.com/Devolutions/IronRDP/commit/f38554277a1af3085c1fa5739cda515939d09abf)) 

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.4.0...ironrdp-dvc-pipe-proxy-v0.4.1)] - 2026-06-05



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.3.0...ironrdp-dvc-pipe-proxy-v0.4.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` and `ironrdp-svc` public dependencies

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.2.0...ironrdp-dvc-pipe-proxy-v0.2.1)] - 2025-09-24

### <!-- 4 -->Bug Fixes

- Change dvc proxy pipe mode from Message to Byte on Windows (#986) ([5f52a44b84](https://github.com/Devolutions/IronRDP/commit/5f52a44b840dd71eae6a355be00f1c4c671b3b58)) 

- Add blocking logic for sending dvc pipe messages ([3182a018e2](https://github.com/Devolutions/IronRDP/commit/3182a018e2972eb77c52ea248387c96a9eb6a6a6)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.1.0...ironrdp-dvc-pipe-proxy-v0.2.0)] - 2025-08-29

### <!-- 1 -->Features

- Make dvc named pipe proxy cross-platform (#896) ([166b76010c](https://github.com/Devolutions/IronRDP/commit/166b76010cbd8f8674e6e8d4801fee5cda1ad9e5)) 

  - Make dvc named pipe proxy cross-platform (Unix implementation via
  `tokio::net::unix::UnixStream`)
  - Removed unsafe code for Windows implementation, switched to
  `tokio::net::windows::named_pipe`
