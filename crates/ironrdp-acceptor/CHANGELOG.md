# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.8.0...ironrdp-acceptor-v0.8.1)] - 2026-03-01



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.7.0...ironrdp-acceptor-v0.8.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.5.0...ironrdp-acceptor-v0.6.0)] - 2025-07-08

### <!-- 1 -->Features

- [**breaking**] Support for server-side Kerberos (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.4.0...ironrdp-acceptor-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Make the CredsspSequence type public ([5abd9ff8e0](https://github.com/Devolutions/IronRDP/commit/5abd9ff8e0da8ea48c6747526c4b703a39bf4972)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.3.1...ironrdp-acceptor-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.3.0...ironrdp-acceptor-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.2.1...ironrdp-acceptor-v0.3.0)] - 2025-01-28

### <!-- 0 -->Security

- Allow using basic RDP/no security ([7c72a9f9bb](https://github.com/Devolutions/IronRDP/commit/7c72a9f9bbe726d6f9f2377c19e9a672d8d086d5)) 

### <!-- 4 -->Bug Fixes

- Drop unexpected PDUs during deactivation-reactivation ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

  The current behavior of handling unmatched PDUs in fn read_by_hint()
  isn't good enough. An unexpected PDUs may be received and fail to be
  decoded during Acceptor::step().
  
  Change the code to simply drop unexpected PDUs (as opposed to attempting
  to replay the unmatched leftover, which isn't clearly needed)

- Reattach existing channels ([c4587b537c](https://github.com/Devolutions/IronRDP/commit/c4587b537c7c0a148e11bc365bc3df88e2c92312)) 

  I couldn't find any explicit behaviour described in the specification,
  but apparently, we must just keep the channel state as they were during
  reactivation. This fixes various state issues during client resize.

- Do not restart static channels on reactivation ([82c7c2f5b0](https://github.com/Devolutions/IronRDP/commit/82c7c2f5b08c44b1a4f6b04c13ad24d9e2ffa371)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.2.0...ironrdp-acceptor-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
