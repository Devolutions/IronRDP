# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.5.0...ironrdp-rdpdr-v0.6.0)] - 2026-04-08

### <!-- 4 -->Bug Fixes

- Model CreateDisposition as enum instead of bitflags ([#1145](https://github.com/Devolutions/IronRDP/issues/1145)) ([c4f87aa417](https://github.com/Devolutions/IronRDP/commit/c4f87aa417e83c9cf6d1550c877ea3facb2f9a59)) 

  CreateDisposition values (FILE_SUPERSEDE through FILE_OVERWRITE_IF) are
  mutually exclusive integers 0 through 5, not combinable bit flags.
  Modeling them with the bitflags macro causes subtle correctness issues.

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



## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.4.1...ironrdp-rdpdr-v0.5.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- Fix incorrect padding when parsing NDR strings ([#1015](https://github.com/Devolutions/IronRDP/issues/1015)) ([a0a3e750c9](https://github.com/Devolutions/IronRDP/commit/a0a3e750c9e4ee9c73b957fbcb26dbc59e57d07d)) 

  When parsing Network Data Representation (NDR) messages, we're supposed
  to account for padding at the end of strings to remain aligned on a
  4-byte boundary. The existing code doesn't seem to cover all cases, and
  the resulting misalignment causes misleading errors when processing the
  rest of the message.

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.4.0...ironrdp-rdpdr-v0.4.1)] - 2025-09-04

### <!-- 1 -->Features

- Support device removal (#947) ([50574c570f](https://github.com/Devolutions/IronRDP/commit/50574c570f6e44d264153337e5f87a5313f190e6)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.2.0...ironrdp-rdpdr-v0.3.0)] - 2025-05-27

### <!-- 1 -->Features

- Add USER_LOGGEDON flag support ([5e78f91713](https://github.com/Devolutions/IronRDP/commit/5e78f917132a174bdd5d8711beb1744de1bd265a)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.3...ironrdp-rdpdr-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.2...ironrdp-rdpdr-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.1...ironrdp-rdpdr-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.1.0...ironrdp-rdpdr-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
