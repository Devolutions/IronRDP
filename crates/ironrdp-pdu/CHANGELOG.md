# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.5.0...ironrdp-pdu-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- [**breaking**] Add server_codecs_capabilities() ([d3aaa43c23](https://github.com/Devolutions/IronRDP/commit/d3aaa43c23b252077b8720bb8ecfeceaaf7b7a7f)) 

  Teach the server to support customizable codecs set. Use the same
  logic/parsing as the client codecs configuration.
  
  Replace "with_remote_fx" with "codecs".

- Add QOI image codec ([613fd51f26](https://github.com/Devolutions/IronRDP/commit/613fd51f26315d8212662c46f8e625c541e4bb59)) 

  The Quite OK Image format ([1]) losslessly compresses images to a
  similar size of PNG, while offering 20x-50x faster encoding and 3x-4x
  faster decoding.
  
  Add a new QOI codec (UUID 4dae9af8-b399-4df6-b43a-662fd9c0f5d6) for
  SetSurface command. The PDU data contains the QOI header (14 bytes) +
  data "chunks" and the end marker (8 bytes).
  
  Some benchmarks showing interesting results (using ironrdp/perfenc)

- Add QOIZ image codec ([87df67fdc7](https://github.com/Devolutions/IronRDP/commit/87df67fdc76ff4f39d4b83521e34bf3b5e2e73bb)) 

  Add a new QOIZ codec (UUID 229cc6dc-a860-4b52-b4d8-053a22b3892b) for
  SetSurface command. The PDU data contains the same data as the QOI
  codec, with zstd compression.
  
  Some benchmarks showing interesting results (using ironrdp/perfenc)

- Improve `ExtendedClientOptionalInfoBuilder` API (#891) ([ae052ed835](https://github.com/Devolutions/IronRDP/commit/ae052ed83598ad1f4ad7038b153e3c5398d2a738)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Update timezone info to use i32 bias (#921) ([119c7077c9](https://github.com/Devolutions/IronRDP/commit/119c7077c98e4b43021619378c4f251c1f95ae17)) 

  Switches `bias` from an unsigned to a signed integer.
  This matches the updated specification from Microsoft.

### <!-- 7 -->Build

- Bump thiserror to 2.0 ([b4fb0aa0c7](https://github.com/Devolutions/IronRDP/commit/b4fb0aa0c79aa409d1b6a5f43ab23448eede4e51)) 

- Bump der-parser to 10.0 ([03cac54ada](https://github.com/Devolutions/IronRDP/commit/03cac54ada50fae13d085b855a9b8db37d615ba8)) 



## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.4.0...ironrdp-pdu-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Make client_codecs_capabilities() configurable ([783702962a](https://github.com/Devolutions/IronRDP/commit/783702962a2e842f9d5046ac706048ba124e1401)) 

- BitmapCodecs struct ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 

### <!-- 4 -->Bug Fixes

- Fix possible out of bound indexing in RFX module (#724) ([9f4e6d410b](https://github.com/Devolutions/IronRDP/commit/9f4e6d410b631d8a6b0c09c2abc0817a83cf042b)) 

  An index bound check was missing in the RFX module. Found by fuzzer.


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.3.1...ironrdp-pdu-v0.4.0)] - 2025-03-12

### <!-- 4 -->Bug Fixes

- TS_RFX_CHANNELT width/height SHOULD be within range ([097cdb66f9](https://github.com/Devolutions/IronRDP/commit/097cdb66f965700caeea5659ff7fe4a129b84838)) 

  According to the specification, the value does not need to be in the range:
  https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/4060f07e-9d73-454d-841e-131a93aca675
  
  (the ironrdp-server can send larger values)

### Refactor

- [**breaking**] Remove RfxChannelWidth and RfxChannelHeight structs ([7cb1ac99d1](https://github.com/Devolutions/IronRDP/commit/7cb1ac99d189cdcaa17fa17e51f95be630e9982e)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.3.0...ironrdp-pdu-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.2.0...ironrdp-pdu-v0.3.0)] - 2025-03-07

### <!-- 4 -->Bug Fixes

- Make AddressFamily parsing resilient (#672) ([6b4af94071](https://github.com/Devolutions/IronRDP/commit/6b4af94071bfb0adff482cc33b75e6c37ff6e10f)) 

- Fix FastPathHeader minimal size (#687) ([3b9d558e9c](https://github.com/Devolutions/IronRDP/commit/3b9d558e9c958297d9654861df515e2a8658bf8b)) 

  The minimal_size() logic didn't properly take into account the overall
  PDU size.
  
  This fixes random error/disconnect in client.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.1.2...ironrdp-pdu-v0.2.0)] - 2025-01-28

### <!-- 1 -->Features

- ClientLicenseInfo and other license PDU-related adjustments (#634) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.1.1...ironrdp-pdu-v0.1.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
