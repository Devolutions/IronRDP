# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.7.0...ironrdp-pdu-v0.8.0)] - 2026-04-10

### <!-- 1 -->Features

- Add Initiate Multitransport Request/Response PDU types ([#1091](https://github.com/Devolutions/IronRDP/issues/1091)) ([5a50f4099b](https://github.com/Devolutions/IronRDP/commit/5a50f4099b8f8173c5c067089a0d372402dbb52d)) 

  Add MultitransportRequestPdu and MultitransportResponsePdu types for the
  sideband UDP transport bootstrapping PDUs defined in MS-RDPBCGR
  2.2.15.1 and 2.2.15.2. Needed to decode/encode the IO channel messages that
  initiate UDP transport setup.

- Add Auto-Detect Request and Response PDU types ([#1168](https://github.com/Devolutions/IronRDP/issues/1168)) ([6e5f08a1b9](https://github.com/Devolutions/IronRDP/commit/6e5f08a1b95f69b9d8182a75298b74aaf829ac39)) 

- [**breaking**] Route auto-detect PDUs through ShareDataPdu dispatch ([#1176](https://github.com/Devolutions/IronRDP/issues/1176)) ([e5f2f36e96](https://github.com/Devolutions/IronRDP/commit/e5f2f36e96dfb2036236c99a1ee83c5a36bf281f)) 

  Added Share Data PDU dispatch support for auto-detect PDUs, improving compatibility with Windows servers.

- Complete pixel format support for bitmap updates ([#1134](https://github.com/Devolutions/IronRDP/issues/1134)) ([a6b41093ce](https://github.com/Devolutions/IronRDP/commit/a6b41093ce4ece081d2538c157f6bc547c3b2607)) 

  Wires missing bitmap pixel formats (8/15/24bpp) into the session rendering
  pipeline so bitmap updates at those depths are rendered instead of being
  dropped, and adds fast-path palette update parsing to support 8bpp indexed
  color sessions.

- Add RemoteFX Progressive codec primitives ([#1196](https://github.com/Devolutions/IronRDP/issues/1196)) ([49099f0c31](https://github.com/Devolutions/IronRDP/commit/49099f0c3136c25b67801fb1b07f78542dc796de)) 

  Add wire-format types for RemoteFX Progressive Codec (MS-RDPRFX
  Progressive Extension) and the computational primitives required for progressive refinement.

- Handle slow-path graphics and pointer updates ([#1132](https://github.com/Devolutions/IronRDP/issues/1132)) ([9383380292](https://github.com/Devolutions/IronRDP/commit/938338029290f1be82a7f784d544bb77ac797aeb)) 

  Adds support for slow-path graphics and pointer updates to IronRDP, fixing connectivity issues with servers like XRDP that use slow-path output instead of fast-path. The implementation parses slow-path framing headers and routes the inner payload structures through the existing fast-path processing pipeline by extracting shared bitmap and pointer processing methods.

### <!-- 4 -->Bug Fixes

- Accept short Server Deactivate All PDU ([485d6c2f8d](https://github.com/Devolutions/IronRDP/commit/485d6c2f8d6f95bb06ca14cbfa4c56a27abbad0e)) 

  Some servers (XRDP, older Windows) send a Deactivate All PDU without
  the sourceDescriptor field. The decode previously required at least 3
  bytes, which caused a hard failure during deactivation-reactivation
  sequences with these servers.
  
  Treat the sourceDescriptor as optional: if the remaining data is
  shorter than the fixed part size, return successfully without
  reading the field. FreeRDP handles this the same way.

- Correct ShareDataHeader uncompressedLength calculation ([#1148](https://github.com/Devolutions/IronRDP/issues/1148)) ([c2688f464d](https://github.com/Devolutions/IronRDP/commit/c2688f464d8cbf239d35e5b43538195b1870eed8)) 

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

### <!-- 6 -->Documentation

- Establish the MSRV policy (current is 1.89) ([#1157](https://github.com/Devolutions/IronRDP/issues/1157)) ([c10e6ff16c](https://github.com/Devolutions/IronRDP/commit/c10e6ff16cc45f094b24e87ed1d46eb88b4a0419)) 

  The MSRV is the oldest stable Rust release that is at least 6 months
  old, bounded by the Rust version available in Debian stable-backports
  and Fedora stable.



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.5.0...ironrdp-pdu-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- Implement `Default` trait on `ExtendedClientOptionalInfoBuilder` (#891) ([ae052ed835](https://github.com/Devolutions/IronRDP/commit/ae052ed83598ad1f4ad7038b153e3c5398d2a738)) 

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
