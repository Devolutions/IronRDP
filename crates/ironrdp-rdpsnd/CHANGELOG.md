# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.9.0...ironrdp-rdpsnd-v0.10.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Play pre-v8 Wave PDUs: `WavePdu` now holds the WaveInfo fields, gaining `data_prefix` and `audio_length`, and `WaveDataPdu` replaces `SndWavePdu` for the wave payload that follows ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db))

- [**breaking**] Pass the negotiated `AudioFormat` to `RdpsndClientHandler::wave` instead of a format index ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db))

- Add `AudioFormat::matches_for_negotiation` to compare formats the way negotiation does ([#1645](https://github.com/Devolutions/IronRDP/issues/1645)) ([50fa88b29e](https://github.com/Devolutions/IronRDP/commit/50fa88b29e5d57c5c6353229bda8aa786a9906fd))

- Add `Rdpsnd::with_quality_mode` to choose the quality mode sent to the server ([#1781](https://github.com/Devolutions/IronRDP/issues/1781)) ([14ef4fd49f](https://github.com/Devolutions/IronRDP/commit/14ef4fd49fcf950169806866ee67db0c49662cfc))

- Receive audio over the `AUDIO_PLAYBACK_DVC` and `AUDIO_PLAYBACK_LOSSY_DVC` dynamic channels with `RdpsndDvc` and `RdpsndDvcState`, and over UDP with `Rdpsnd::with_datagram_port`, `Rdpsnd::process_datagram` and the UDP wave PDUs ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Add the `codec` module, with PCM, G.711 and ADPCM decoders and encoders, and the `playout` jitter-buffer module ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Add `RdpsndClientHandler::choose_formats` and `pdu::retain_offered` so handlers pick the formats to advertise from the server's list ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Report the hold time plus `RdpsndClientHandler::playback_latency` in Wave Confirm PDUs instead of echoing the server timestamp, and add `Rdpsnd::with_clock` ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Send the capture time in the `wTimeStamp` of server waves instead of zero, and add `RdpsndServerHandler::wave_confirm` to receive the client's Wave Confirm PDUs ([#1720](https://github.com/Devolutions/IronRDP/issues/1720)) ([160752fcf3](https://github.com/Devolutions/IronRDP/commit/160752fcf3293889f1ce1bdd3d2e7afc790a9cd2))

- Add `RdpsndServer::is_ready` to tell whether negotiation finished with an accepted format ([#1990](https://github.com/Devolutions/IronRDP/issues/1990)) ([2e872b085b](https://github.com/Devolutions/IronRDP/commit/2e872b085b504cf3b9ce9994237270f71a35d13a))

- Add `RdpsndServer::stats` and `StreamStats` to count sent waves and their confirms ([#1991](https://github.com/Devolutions/IronRDP/issues/1991)) ([966a842f7e](https://github.com/Devolutions/IronRDP/commit/966a842f7e4ca6ff045a860a9fec68eb9206615b))

- Report byte offsets in RDPSND PDU decode and encode errors ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

### <!-- 4 -->Bug Fixes

- Ignore malformed RDPSND PDUs instead of failing the client channel ([#1514](https://github.com/Devolutions/IronRDP/issues/1514)) ([c87ab68e9c](https://github.com/Devolutions/IronRDP/commit/c87ab68e9c6adbf524cb0b2783ff4bd61178fb9b))

- Resolve `wFormatNo` against the formats the client advertised, in the order it advertised them, instead of the server list ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9



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
