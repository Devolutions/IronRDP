# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.5.0...ironrdp-rdpdr-native-v0.5.1)] - 2026-03-25

### <!-- 4 -->Bug Fixes

- Model CreateDisposition as enum instead of bitflags ([#1145](https://github.com/Devolutions/IronRDP/issues/1145)) ([c4f87aa417](https://github.com/Devolutions/IronRDP/commit/c4f87aa417e83c9cf6d1550c877ea3facb2f9a59)) 

  CreateDisposition values (FILE_SUPERSEDE through FILE_OVERWRITE_IF) are
  mutually exclusive integers 0 through 5, not combinable bit flags.
  Modeling them with the bitflags macro causes subtle correctness issues.

### <!-- 6 -->Documentation

- Establish the MSRV policy (current is 1.89) ([#1157](https://github.com/Devolutions/IronRDP/issues/1157)) ([c10e6ff16c](https://github.com/Devolutions/IronRDP/commit/c10e6ff16cc45f094b24e87ed1d46eb88b4a0419)) 

  The MSRV is the oldest stable Rust release that is at least 6 months
  old, bounded by the Rust version available in Debian stable-backports
  and Fedora stable.

### <!-- 7 -->Build

- Bump nix from 0.30.1 to 0.31.1 ([#1085](https://github.com/Devolutions/IronRDP/issues/1085)) ([e92135dc0d](https://github.com/Devolutions/IronRDP/commit/e92135dc0d46bb3217ad26fcb82651c29e9c43c4)) 



## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.4.0...ironrdp-rdpdr-native-v0.5.0)] - 2025-12-18


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.3.0...ironrdp-rdpdr-native-v0.4.0)] - 2025-08-29

### <!-- 7 -->Build

- Bump nix to 0.30 ([971ad922a5](https://github.com/Devolutions/IronRDP/commit/971ad922a51f78511243aaa885acdd8b1ed94b27)) 
- Bump ironrdp-pdu

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.1.2...ironrdp-rdpdr-native-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.1.1...ironrdp-rdpdr-native-v0.1.2)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 
