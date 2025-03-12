# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


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
