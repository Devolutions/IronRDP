# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.2.2...ironrdp-rdcleanpath-v0.2.3)] - 2026-09-04

### <!-- 1 -->Features

- Support Hyper-V connection ordering ([#1505](https://github.com/Devolutions/IronRDP/issues/1505)) ([5c1816244e](https://github.com/Devolutions/IronRDP/commit/5c1816244e83187a04249e9d9c240d096cb78f55)) 

  Hyper-V over RDCleanPath needs PCB → TLS on the proxy, then CredSSP →
  X.224 on the client. Ordinary RDCleanPath stays X.224-first.
  
  Still VERSION_1 with the same DER fields. An explicit VMConnect request
  carries a Unicode PCB payload in `preconnection_blob` with no X.224; the
  proxy encodes the binary PCB. Generic PCB requests keep their existing
  X.224-first behavior.
  
  Gateway reference implementation:
  [Devolutions/devolutions-gateway#1372](https://github.com/Devolutions/devolutions-gateway/pull/1372)
  
  Checked locally: Rust builds, formatting, Svelte typecheck, and .NET
  build. Real nested Hyper-V E2E through Gateway: Native rendered 18
  frames, Avalonia connected and rendered its first frame, and Web
  rendered a non-empty 1280×720 canvas.
  
  ---------

### <!-- 7 -->Build

- Bump the crypto group across 1 directory with 3 updates ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b)) 



## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.2.1...ironrdp-rdcleanpath-v0.2.2)] - 2026-06-05

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.2.0...ironrdp-rdcleanpath-v0.2.1)] - 2025-10-02

### <!-- 1 -->Features

- Human-readable descriptions for RDCleanPath errors (#999) ([18c81ed5d8](https://github.com/Devolutions/IronRDP/commit/18c81ed5d8d3bf13b3d10fe15209233c0c10bb62)) 

  More munging to give human-readable webclient-side errors for
  RDCleanPath general/negotiation errors, including strings for WSA and
  TLS and HTTP error conditions.

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.1.3...ironrdp-rdcleanpath-v0.2.0)] - 2025-08-29

### <!-- 1 -->Features

- [**breaking**] Extend helper API for handling negotiation errors (#930) ([ca11e338d7](https://github.com/Devolutions/IronRDP/commit/ca11e338d7231c86f60a110627a5d864377d8594)) 

  - Helper for proxies creating an RDCleanPath error with server response.
  - Helper for clients to handle these.

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.1.2...ironrdp-rdcleanpath-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.1.1...ironrdp-rdcleanpath-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


