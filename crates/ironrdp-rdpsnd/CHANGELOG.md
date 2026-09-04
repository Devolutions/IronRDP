# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.9.0...ironrdp-rdpsnd-v0.10.0)] - 2026-09-04

### <!-- 0 -->Security

- Support advanced session settings ([#1781](https://github.com/Devolutions/IronRDP/issues/1781)) ([14ef4fd49f](https://github.com/Devolutions/IronRDP/commit/14ef4fd49fcf950169806866ee67db0c49662cfc)) 

  Wire administrative-session GCC data, opaque load-balance routing
  tokens, and RDPSND quality selection through the ActiveX settings
  objects and client configuration.
  
  Keep unrelated transport, cache, video, device, and security policy
  slots as explicit E_NOTIMPL failures.
  
  ---------

### <!-- 1 -->Features

- Add AUDIO_INPUT protocol crate ([#1645](https://github.com/Devolutions/IronRDP/issues/1645)) ([50fa88b29e](https://github.com/Devolutions/IronRDP/commit/50fa88b29e5d57c5c6353229bda8aa786a9906fd)) 

  Introduce `ironrdp-rdpeai` for MS-RDPEAI (AUDIO_INPUT) PDUs and client
  handler, plus the shared RDPSND format matching helper used during
  negotiation.
  
  The crate is workspace-internal (`publish = false`) with unit coverage
  in `ironrdp-testsuite-core`. Capture backends and ActiveX wiring land in
  a follow-up stacked PR.

- Harden Windows client playback path ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db)) 

  Keep client format order for wFormatNo, play pre-v8 Wave PDUs, and apply
  volume on a broader CPAL PCM offer so ActiveX mode 0 can redirect remote
  audio reliably.
  
  Also fix clippy noise in the RDPSND client suite and keep interleaved
  volume L/R phase stable across wave blocks. Volume scaling is a simple
  amplitude map, not a logarithmic MS-RDPEA model.

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

- Carry the capture timestamp on waves and surface wave confirms ([#1720](https://github.com/Devolutions/IronRDP/issues/1720)) ([160752fcf3](https://github.com/Devolutions/IronRDP/commit/160752fcf3293889f1ce1bdd3d2e7afc790a9cd2)) 

  MS-RDPEA 2.2.3.8 has the client answer a wave's wTimeStamp in the Wave
  Confirm
  PDU, set to "the same field of the originating WaveInfo PDU [...] plus
  the
  time, in milliseconds, between receiving the complete wave PDU from the
  network
  and sending this PDU". The server hardcoded that field to zero for both
  Wave
  and Wave2, so the client's answer was relative to nothing and the
  confirm was
  only logged, never delivered - an embedder chasing an audio/video offset
  had no
  way to tell "the delay is on my side of the socket" from "the client is
  buffering".
  
  Waves now carry the low bits of the same capture time the 32-bit
  dwAudioTimeStamp gets, and RdpsndServerHandler gains a defaulted
  wave_confirm
  method that receives the block number and that timestamp. Existing
  handlers are
  unaffected.
  
  The value is not an echo, and one block can be confirmed more than once.
  FreeRDP's client sends two confirms per wave: the first on receipt with
  the
  timestamp unchanged, "to determine the network latency", and a second
  after
  playback with the elapsed time added, "to determine the actual render
  latency"
  (channels/rdpsnd/client/rdpsnd_main.c, both comments citing 2.2.3.8). A
  handler
  therefore sees the two measurements as two calls with the same block_no,
  which
  the rustdoc now says.
  
  The trait method has no consumer inside this repository. Its consumer is
  hypr-rdp, which times its own PipeWire captures and needs the confirm to
  separate capture-to-wire delay from client-side buffering; without a
  hook there
  is no way to reach the PDU at all, since the server discards it after a
  debug
  log. Setting the field and surfacing the confirm are separable, but only
  in the
  sense that either alone leaves the measurement impossible: a timestamp
  nobody
  receives, or a confirm relative to zero.

### <!-- 4 -->Bug Fixes

- Isolate malformed encrypted waves ([#1514](https://github.com/Devolutions/IronRDP/issues/1514)) ([c87ab68e9c](https://github.com/Devolutions/IronRDP/commit/c87ab68e9c6adbf524cb0b2783ff4bd61178fb9b)) 

  ## Summary
  
  - Treat malformed RDPSND server-audio PDUs as recoverable channel input
  and ignore them without failing the desktop session.
  - Preserve the RDPSND state after a decode failure so valid subsequent
  audio continues normally.
  - Add a regression test for an encrypted wave missing its required v5
  signature.
  
  ## Testing
  
  - `cargo test -p ironrdp-testsuite-core --test integration_tests_core --
  rdpsnd::client`
  - `cargo fmt --all -- --check`
  
  ---------



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.8.1...ironrdp-rdpsnd-v0.9.0)] - 2026-07-10

### <!-- 1 -->Features

- [**breaking**] Misuse-resistant format negotiation for RdpsndServerHandler ([#1359](https://github.com/Devolutions/IronRDP/issues/1359)) ([2d3bdef1a7](https://github.com/Devolutions/IronRDP/commit/2d3bdef1a7167d2acdc478a92917cbb2f018960b)) 

  Move the negotiation into the crate and split selection from lifecycle:
  
  ```rust
  fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat>;
  fn start(&mut self, format: &NegotiatedFormat);
  ```



## [[0.8.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.8.0...ironrdp-rdpsnd-v0.8.1)] - 2026-06-05

### <!-- 6 -->Documentation

- Document RdpsndServerHandler::start wFormatNo contract ([#1343](https://github.com/Devolutions/IronRDP/issues/1343)) ([7894d9f093](https://github.com/Devolutions/IronRDP/commit/7894d9f093db3c80f7358af8e0d8beb18964ce45)) 

  Adds Rustdoc documentation to `RdpsndServerHandler`, focusing on the contract for `start()`’s `Option<u16>` return value so implementers correctly compute `wFormatNo` for Wave/Wave2 PDUs.



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.7.0...ironrdp-rdpsnd-v0.8.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

- Handle AudioFormat renegotiation in Ready state ([#1164](https://github.com/Devolutions/IronRDP/issues/1164)) ([2fe6fd0424](https://github.com/Devolutions/IronRDP/commit/2fe6fd04244a7031a19af5a321bdf44308f6df2d)) 

  Sometimes Windows Server re-sends `SNDC_FORMATS` during Ready state
  (e.g., after mute/unmute in remote browser). Previously this hit the
  wildcard branch, entering Stop and permanently killing audio.
    
  Add an `AudioFormat` arm in Ready state to close the current stream and
  restart negotiation.

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.4.0...ironrdp-rdpsnd-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Add support for client custom flags ([7bd92c0ce5](https://github.com/Devolutions/IronRDP/commit/7bd92c0ce5c686fe18c062b7edfeed46a709fc23)) 

  Client can support various flags, but always set ALIVE.

### <!-- 4 -->Bug Fixes

- Correct TrainingPdu wPackSize field ([abcc42e01f](https://github.com/Devolutions/IronRDP/commit/abcc42e01fda3ce9c8e1739524e0fc73b8778d83)) 

- Reply to TrainingPdu ([5dcc526f51](https://github.com/Devolutions/IronRDP/commit/5dcc526f513e8083ff335cad3cc80d2effeb7265)) 

- Lookup the associated format from the client list ([3d7bc28b97](https://github.com/Devolutions/IronRDP/commit/3d7bc28b9764b1f37b038bb2fbb676ec464ee5ee)) 

- Send client formats that match server (#742) ([a8b9614323](https://github.com/Devolutions/IronRDP/commit/a8b96143236ad457b5241f6a2f8acfaf969472b6)) 

  Windows seems to be confused if the client replies with more formats, or unknown formats (opus).

### Refactor

- [**breaking**] Pass format_no instead of AudioFormat ([4172571e8e](https://github.com/Devolutions/IronRDP/commit/4172571e8e061a6a120643393881b5e37f1e61ab)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.3.1...ironrdp-rdpsnd-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.3.0...ironrdp-rdpsnd-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.2.0...ironrdp-rdpsnd-v0.3.0)] - 2025-02-05

### <!-- 1 -->Features

- New required method `get_formats` for the `RdpsndClientHandler` trait (#661) ([ccf6348270](https://github.com/Devolutions/IronRDP/commit/ccf63482706ecfbbdc6038028ea2ee086d0e3640)) 


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.1.1...ironrdp-rdpsnd-v0.2.0)] - 2025-01-28

### <!-- 1 -->Features

- Support for volume setting (#641) ([a6c36511f6](https://github.com/Devolutions/IronRDP/commit/a6c36511f6584f67b8c6e795c34d5007ec2b24a4)) 

  Add server messages and API to support setting client volume.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.1.0...ironrdp-rdpsnd-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
