# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.8.0...ironrdp-async-v0.8.1)] - 2026-03-01



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.7.0...ironrdp-async-v0.8.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

## [[0.3.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.3.1...ironrdp-async-v0.3.2)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.3.0...ironrdp-async-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.2.1...ironrdp-async-v0.3.0)] - 2025-01-28

### <!-- 4 -->Changed

- Remove unmatched parameter from `Framed::read_by_hint` function ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-async-v0.2.0...ironrdp-async-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
