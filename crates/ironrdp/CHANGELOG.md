# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.14.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.14.0...ironrdp-v0.14.1)] - 2026-03-01

### <!-- 0 -->Security

- Add alternate_shell and work_dir configuration support ([#1095](https://github.com/Devolutions/IronRDP/issues/1095)) ([a33d27fe67](https://github.com/Devolutions/IronRDP/commit/a33d27fe6771a5a155161ef40a04de88803dd84c)) 

  Add support for configuring `alternate_shell` and `work_dir` fields in
  ClientInfoPdu, which are used by:
    - CyberArk PSM (Privileged Session Manager) for session tokens
    - Remote application scenarios (RemoteApp)
    - Custom shell configurations

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

- Implement ECHO virtual channel ([#1109](https://github.com/Devolutions/IronRDP/issues/1109)) ([6f6496ad29](https://github.com/Devolutions/IronRDP/commit/6f6496ad29395099563d50417d6dfff623914ee6)) 



## [[0.14.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.13.0...ironrdp-v0.14.0)] - 2025-12-18

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.13.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.12.0...ironrdp-v0.13.0)] - 2025-09-24

### Build

- Update dependencies

## [[0.12.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.11.0...ironrdp-v0.12.0)] - 2025-08-29

### Build

- Update dependencies

## [[0.11.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.10.0...ironrdp-v0.11.0)] - 2025-07-08

### Build

- Update dependencies

## [[0.9.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.9.0...ironrdp-v0.9.1)] - 2025-03-13

### <!-- 6 -->Documentation

- Fix documentation build (#700) ([0705840aa5](https://github.com/Devolutions/IronRDP/commit/0705840aa51bc920e76f0cf1fce06b29733c6e2d)) 

## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.8.0...ironrdp-v0.9.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.4...ironrdp-v0.8.0)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.7.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.3...ironrdp-v0.7.4)] - 2025-01-28

### Build

- Update dependencies

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

- Extend server example to demonstrate Opus audio codec support (#643) ([fa353765af](https://github.com/Devolutions/IronRDP/commit/fa353765af016734c07e31fff44d19dabfdd4199)) 


## [[0.7.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.2...ironrdp-v0.7.3)] - 2024-12-16

### <!-- 6 -->Documentation

- Inline documentation for re-exported items (#619) ([cff5c1a59c](https://github.com/Devolutions/IronRDP/commit/cff5c1a59cdc2da73cabcb675fcf2d85dc81fd68)) 



## [[0.7.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.1...ironrdp-v0.7.2)] - 2024-12-15

### <!-- 6 -->Documentation

- Fix server example ([#616](https://github.com/Devolutions/IronRDP/pull/616)) ([02c6fd5dfe](https://github.com/Devolutions/IronRDP/commit/02c6fd5dfe142b7cc6f15cb17292504657818498)) 

  The rt-multi-thread feature of tokio is not enabled when compiling the
  example alone (without feature unification from other crates of the
  workspace).



## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.0...ironrdp-v0.7.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

