# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-native-v0.7.0...ironrdp-rdpsnd-native-v0.8.0)] - 2026-10-27

### <!-- 1 -->Features

- [**breaking**] Decode every server format the backend supports, including AAC through Media Foundation on Windows behind the default `aac` feature, and remove `DecodeStream` ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Play through a jitter buffer with a configurable latency target (`PlaybackConfig`, `DEFAULT_PLAYBACK_LATENCY`) and report buffered plus device latency ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Select playback and capture devices with `DeviceSelector`, and list them with `list_devices` ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Map the server volume to the Windows audio session ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Add MS-RDPEAI microphone capture behind the `capture` feature: `RdpeaiCaptureBackend` captures 16-bit PCM ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d))

- Capture encodes to PCM, G.711 or ADPCM ([#2203](https://github.com/Devolutions/IronRDP/issues/2203)) ([45b67591e9](https://github.com/Devolutions/IronRDP/commit/45b67591e92930a8c2159c3e70a0ed228a8f7483))

- Keep the client's format order, play Wave PDUs from servers older than RDPSND version 8, and apply the server volume to more PCM formats ([#1648](https://github.com/Devolutions/IronRDP/issues/1648)) ([2d9a9bf114](https://github.com/Devolutions/IronRDP/commit/2d9a9bf114dcf41a1ddc7343f564bc2e8d1d06db))

### <!-- 4 -->Bug Fixes

- Stop blocking the real-time playback callback ([#1803](https://github.com/Devolutions/IronRDP/issues/1803)) ([850f654673](https://github.com/Devolutions/IronRDP/commit/850f6546730cff1a6b858864f6fbf9cb7d6264fd))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- Add the optional `ironrdp-rdpeai` 0.2 public dependency, enabled by `capture`

- [**breaking**] Update `ironrdp-rdpsnd` public dependency to 0.11



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
