# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.8.0...ironrdp-connector-v0.9.0)] - 2026-03-01

### <!-- 0 -->Security

- Add alternate_shell and work_dir configuration support ([#1095](https://github.com/Devolutions/IronRDP/issues/1095)) ([a33d27fe67](https://github.com/Devolutions/IronRDP/commit/a33d27fe6771a5a155161ef40a04de88803dd84c)) 

  Add support for configuring `alternate_shell` and `work_dir` fields in
  ClientInfoPdu, which are used by:
    - CyberArk PSM (Privileged Session Manager) for session tokens
    - Remote application scenarios (RemoteApp)
    - Custom shell configurations

- Dispatch multitransport PDUs on IO channel ([#1096](https://github.com/Devolutions/IronRDP/issues/1096)) ([7853e3cc6f](https://github.com/Devolutions/IronRDP/commit/7853e3cc6f26acaf3da000c6177ca3cef6ef85fd)) 

  `decode_io_channel()` assumes all IO channel PDUs begin with
  a`ShareControlHeader`. Multitransport Request PDUs use a
  `BasicSecurityHeader` with `SEC_TRANSPORT_REQ` instead ([MS-RDPBCGR]
  2.2.15.1).
  
  This adds a peek-based dispatch: check the first `u16`
  for`TRANSPORT_REQ`, decode as `MultitransportRequestPdu` if set,
  otherwise fall through to the existing `decode_share_control()` path
  unchanged.
  
  The new variant is propagated through `ProcessorOutput` and
  'ActiveStageOutput` so applications can handle multitransport requests.
  Client and web consumers log the request (no UDP transport yet).

### <!-- 1 -->Features

- Add bulk compression and wire negotiation ([ebf5da5f33](https://github.com/Devolutions/IronRDP/commit/ebf5da5f3380a3355f6c95814d669f8190425ded)) 

  - add ironrdp-bulk crate with MPPC/NCRUSH/XCRUSH, bitstream, benches, and metrics
  - advertise compression in Client Info and plumb compression_type through connector
  - decode compressed FastPath/ShareData updates using BulkCompressor
  - update CLI to numeric compression flags (enabled by default, level 0-3)
  - extend screenshot example with compression options and negotiated logging
  - refresh tests, FFI/web configs, typos, and Cargo.lock

- Advertise multitransport channel in GCC blocks ([#1092](https://github.com/Devolutions/IronRDP/issues/1092)) ([4f5fdd3628](https://github.com/Devolutions/IronRDP/commit/4f5fdd3628f4d0d2c2a4116e4e45269d802740f1)) 

  Add multitransport_flags config option to populate the
  MultiTransportChannelData GCC block during connection negotiation.
  When None (the default), behavior is unchanged.

### <!-- 4 -->Bug Fixes

- Make fields of Error private ([#1074](https://github.com/Devolutions/IronRDP/issues/1074)) ([e51ed236ce](https://github.com/Devolutions/IronRDP/commit/e51ed236ce5d55dc1a4bc5f5809fd106bdd2e834)) 

### <!-- 5 -->Performance

- Reduce connection latency when Kerberos is disabled ([#1107](https://github.com/Devolutions/IronRDP/issues/1107)) ([b1b0289e00](https://github.com/Devolutions/IronRDP/commit/b1b0289e0067228dbc973d3edb0e27136f7ca52a)) 



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.7.1...ironrdp-connector-v0.8.0)] - 2025-12-18

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.7.0...ironrdp-connector-v0.7.1)] - 2025-09-04

### <!-- 1 -->Features

- Add API to retrieve registered SVC processors (#938) ([17833fe009](https://github.com/Devolutions/IronRDP/commit/17833fe009279823c4076d3e2e0c7d063fd24a43)) 

## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.6.0...ironrdp-connector-v0.7.0)] - 2025-08-29

### <!-- 1 -->Features

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a similar size
  of PNG, while offering 20x-50x faster encoding and 3x-4x faster decoding.

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec for SetSurface command. The PDU data contains the same
  data as the QOI codec, with zstd compression.

- Add an option to specify a timezone (#917) ([6fab9f8228](https://github.com/Devolutions/IronRDP/commit/6fab9f8228578b3c78db131b3c2e0526352116a9)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Rename option no_server_pointer into enable_server_pointer ([218fed03c7](https://github.com/Devolutions/IronRDP/commit/218fed03c7993af0f958453e3944c58bcf9f43cb)) 

- [**breaking**] Rename option no_audio_playback into enable_audio_playback ([5d8a487001](https://github.com/Devolutions/IronRDP/commit/5d8a487001c1280cbaf9f581f2a9a2f47d187bf0)) 

### <!-- 7 -->Build

- Bump rand to 0.9 ([de0877188c](https://github.com/Devolutions/IronRDP/commit/de0877188cbb3692c3ce0d9a72f6e96d515cde1f)) 

- Bump picky from 7.0.0-rc.16 to 7.0.0-rc.17 (#941) ([fe31cf2c57](https://github.com/Devolutions/IronRDP/commit/fe31cf2c574e0b06177a931db4cac95ea9cfbe7e)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.5.1...ironrdp-connector-v0.6.0)] - 2025-07-08

### Build

- [**breaking**] Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.5.0...ironrdp-connector-v0.5.1)] - 2025-07-03

### <!-- 7 -->Build

- Bump picky to v7.0.0-rc.15 (#850) ([eca256ae10](https://github.com/Devolutions/IronRDP/commit/eca256ae10c52c4a42e7e77d41c0a1d6c180ebf3)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.4.0...ironrdp-connector-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Add no_audio_playback flag to Config struct ([9f0edcc4c9](https://github.com/Devolutions/IronRDP/commit/9f0edcc4c9c49d59cc10de37f920aae073e3dd8a)) 

  Enable audio playback on the client.

### <!-- 4 -->Bug Fixes

- [**breaking**] Fix name of client address field (#754) ([bdde2c76de](https://github.com/Devolutions/IronRDP/commit/bdde2c76ded7315f7bc91d81a0909a1cb827d870)) 

- Inject socket local address for the client addr (#759) ([712da42ded](https://github.com/Devolutions/IronRDP/commit/712da42dedc193239e457d8270d33cc70bd6a4b9)) 

  We used to inject the resolved target server address, but that is not
  what is expected. Server typically ignores this field so this was not a
  problem up until now.

### Refactor

- [**breaking**] Add supported codecs in BitmapConfig ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.2...ironrdp-connector-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.3.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.1...ironrdp-connector-v0.3.2)] - 2025-03-07

### Build

- Update dependencies



## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.3.0...ironrdp-connector-v0.3.1)] - 2025-01-30

### <!-- 4 -->Bug Fixes

- Decrease log verbosity for license exchange ([#655](https://github.com/Devolutions/IronRDP/issues/655)) ([c8597733fe](https://github.com/Devolutions/IronRDP/commit/c8597733fe9998318764064c3682506bf82026d2)) 



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.2...ironrdp-connector-v0.3.0)] - 2025-01-28

### <!-- 1 -->Features

- Support license caching ([#634](https://github.com/Devolutions/IronRDP/issues/634)) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

  Adds support for license caching by storing the license obtained
  from SERVER_UPGRADE_LICENSE message and sending
  CLIENT_LICENSE_INFO if a license requested by the server is already
  stored in the cache.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo ([#631](https://github.com/Devolutions/IronRDP/issues/631)) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

### <!-- 7 -->Build

- Bump picky from 7.0.0-rc.11 to 7.0.0-rc.12 ([#639](https://github.com/Devolutions/IronRDP/issues/639)) ([a16a131e43](https://github.com/Devolutions/IronRDP/commit/a16a131e4301e0dfafe8f3b73e1a75a3a06cfdc7)) 



## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-connector-v0.2.1...ironrdp-connector-v0.2.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
