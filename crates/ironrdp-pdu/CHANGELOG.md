# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


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
