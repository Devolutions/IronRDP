# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.7.0...ironrdp-rdpdr-native-v0.7.1)] - 2026-08-13

### <!-- 0 -->Security

- Add advanced Windows filesystem semantics ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61)) 

  Extend the Windows-native RDPDR backend with confined directory,
  notification, lock, security, stream, control, and volume support.
  Decode only the portable IRPs and capability needed to dispatch these
  native operations.

### <!-- 1 -->Features

- Add Windows filesystem backend ([#1587](https://github.com/Devolutions/IronRDP/issues/1587)) ([7dce8a306f](https://github.com/Devolutions/IronRDP/commit/7dce8a306f43462677879905642967066c42337f)) 

  Add a handle-relative Windows RDPDR backend for one selected volume.
  It confines protocol paths below an opened root and supports bounded
  file I/O
  and basic metadata.
  
  Unsupported advanced filesystem operations return STATUS_NOT_SUPPORTED;
  later
  stack layers will add advanced Windows semantics and host integration.

- Wire RDPDR backends into client connections ([#1600](https://github.com/Devolutions/IronRDP/issues/1600)) ([1fbc9bab0b](https://github.com/Devolutions/IronRDP/commit/1fbc9bab0bc26d8fe0789d5215005d7ea22e2a54)) 

  Build a fresh RDPDR backend product for every connection attempt.
  
  Attach RDPDR only when its product has filesystem devices, advertise
  RDPSND for Windows interoperability, and deliver deferred responses.

- Add static drive redirection ([#1616](https://github.com/Devolutions/IronRDP/issues/1616)) ([a724f783d1](https://github.com/Devolutions/IronRDP/commit/a724f783d1638b4e215507849c1cf148887f30d5)) 

  Expose Windows logical volumes through the ActiveX drive collection and
  configure a static RDPDR backend from the selected pre-connect snapshot.
  
  DisableRdpdr remains a hard override.



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.6.0...ironrdp-rdpdr-native-v0.7.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-rdpdr` public dependency to 0.7

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.5.0...ironrdp-rdpdr-native-v0.6.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Model CreateDisposition as enum instead of bitflags ([#1145](https://github.com/Devolutions/IronRDP/issues/1145)) ([c4f87aa417](https://github.com/Devolutions/IronRDP/commit/c4f87aa417e83c9cf6d1550c877ea3facb2f9a59)) 

  CreateDisposition values (FILE_SUPERSEDE through FILE_OVERWRITE_IF) are
  mutually exclusive integers 0 through 5, not combinable bit flags.
  Modeling them with the bitflags macro causes subtle correctness issues.

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
