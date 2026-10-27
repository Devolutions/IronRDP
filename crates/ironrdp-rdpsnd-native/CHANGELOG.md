# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.7.0...ironrdp-rdpsnd-native-v0.8.0)] - 2026-10-27

### <!-- 1 -->Features

- Harden Windows client playback path ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db)) 

  Keep client format order for wFormatNo, play pre-v8 Wave PDUs, and apply
  volume on a broader CPAL PCM offer so ActiveX mode 0 can redirect remote
  audio reliably.
  
  Also fix clippy noise in the RDPSND client suite and keep interleaved
  volume L/R phase stable across wave blocks. Volume scaling is a simple
  amplitude map, not a logarithmic MS-RDPEA model.

- Wire MS-RDPEAI capture into Windows client and ActiveX ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d)) 

  Add the client MS-RDPEAI capture path on top of hardened RDPSND
  playback: connector CFG + static channel wiring, CPAL PCM capture
  backend, ironrdp-client --audio-capture, and ActiveX
  AudioCaptureRedirectionMode.
  
  PCM capture only accepts encode formats that match the Open capture
  stream, rejects non-16-bit capture (Data PDU size contract), and gates
  the capture backend behind ironrdp-rdpsnd-native/capture.
  
  Depends on #1648 (playback).

- [**breaking**] Compressed audio over AUDIO_PLAYBACK_DVC ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483)) 

  Windows 8 and later servers stream audio over the AUDIO_PLAYBACK_DVC and
  AUDIO_PLAYBACK_LOSSY_DVC dynamic channels, preferring AAC or ADPCM on
  constrained links, but IronRDP declined those channels and could only
  play PCM and Opus. Wave Confirm PDUs echoed the server timestamp,
  playback dropped audio when its fixed 500 ms ring filled, and devices
  could not be chosen.
  
  - `ironrdp-rdpsnd` gains a pure `codec` module (PCM, G.711 A-law and
  µ-law, IMA and Microsoft ADPCM decoders and encoders, bit-exact with the
  Windows ACM codecs), an `RdpsndDvc` processor whose `RdpsndDvcState`
  lets both dynamic channels share one negotiation (Windows negotiates on
  the reliable channel and sends AAC on the lossy one), the UDP Wave
  datagrams, and a `playout` jitter-buffer policy. Wave Confirm PDUs
  report the hold time plus the handler's playback latency, and handlers
  pick the formats to advertise from the server's list.
  - The CPAL backend decodes every offered format it can, including AAC
  through Media Foundation on Windows, plays through the jitter buffer
  with a configurable latency target, reports buffered plus device
  latency, follows the default device or a selected one, and maps the
  server volume to the Windows audio session. Microphone capture encodes
  to PCM, G.711 or ADPCM and selects its device.
  - `ironrdp-client` registers the dynamic channels next to the static one
  on a shared backend and adds `AudioConfig` (`ironrdp_audiooutputdevice`,
  `ironrdp_audioinputdevice`, `ironrdp_audiolatency`); the viewer adds
  `--audio-output-device`, `--audio-input-device`, `--audio-latency` and
  `--list-audio-devices`.

### <!-- 4 -->Bug Fixes

- Stop blocking the real-time playback callback ([#1803](https://github.com/Devolutions/IronRDP/issues/1803)) ([850f654673](https://github.com/Devolutions/IronRDP/commit/850f6546730cff1a6b858864f6fbf9cb7d6264fd)) 



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.6.0...ironrdp-rdpsnd-native-v0.7.0)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.

- [**breaking**] Replace anyhow with typed RdpsndNativeError ([#1277](https://github.com/Devolutions/IronRDP/issues/1277)) ([37483ebd9b](https://github.com/Devolutions/IronRDP/commit/37483ebd9b7628325666f434e1679e7f885fb289)) 



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.5.0...ironrdp-rdpsnd-native-v0.6.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Allocate Opus PCM buffer as Vec<i16> to avoid alignment panic ([#1256](https://github.com/Devolutions/IronRDP/issues/1256)) ([905a148604](https://github.com/Devolutions/IronRDP/commit/905a148604e7bac67cdcb2e915e3cacd29693f57)) 

### <!-- 7 -->Build

- Bump cpal from 0.16.0 to 0.17.1 ([#1071](https://github.com/Devolutions/IronRDP/issues/1071)) ([71245d58cc](https://github.com/Devolutions/IronRDP/commit/71245d58ccfb35dcc403628000ac2649b4bf9697)) 

- Bump opus2 from 0.3.3 to 0.4.0 ([#1204](https://github.com/Devolutions/IronRDP/issues/1204)) ([1eaf333057](https://github.com/Devolutions/IronRDP/commit/1eaf333057bec13778d78be2b2e71ca429733ee9)) 


## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.4.0...ironrdp-rdpsnd-native-v0.4.1)] - 2025-09-24

### <!-- 7 -->Build

- Replace `opus` by `opus2` (#985) ([e5042a7d81](https://github.com/Devolutions/IronRDP/commit/e5042a7d81b864e78ccf19d6b358d94458f951d0)) 

  `opus` is unmaintained and points to a 4-year-old commit of the opus C
  library. This does not compile anymore on our CI, because their
  CMakeList.txt requires an older version of CMake that is not available
  in the runners we use. `opus2` is a fork that points to a more recent
  version of it.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.3.1...ironrdp-rdpsnd-native-v0.4.0)] - 2025-08-29

### <!-- 7 -->Build

- Bump cpal to 0.16 ([eeac1fee1f](https://github.com/Devolutions/IronRDP/commit/eeac1fee1fed4858f4776d86072790bc074e34eb)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.3.0...ironrdp-rdpsnd-native-v0.3.1)] - 2025-06-27

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 3 updates (#816) ([5c5f441bdd](https://github.com/Devolutions/IronRDP/commit/5c5f441bdd514d3fe6a29b4df872709167a9916d)) 

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.3...ironrdp-rdpsnd-native-v0.1.4)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.2...ironrdp-rdpsnd-native-v0.1.3)] - 2025-02-05

### <!-- 1 -->Features

- Add Opus audio client decoding (#661) ([ccf6348270](https://github.com/Devolutions/IronRDP/commit/ccf63482706ecfbbdc6038028ea2ee086d0e3640)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.1...ironrdp-rdpsnd-native-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.1.0...ironrdp-rdpsnd-native-v0.1.1)] - 2024-12-15

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
