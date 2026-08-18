# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.7.0...ironrdp-rdpdr-v0.8.0)] - 2026-08-18

### <!-- 0 -->Security

- Add advanced Windows filesystem semantics ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61)) 

  Extend the Windows-native RDPDR backend with confined directory,
  notification, lock, security, stream, control, and volume support.
  Decode only the portable IRPs and capability needed to dispatch these
  native operations.

### <!-- 1 -->Features

- Add filesystem PDU foundation ([#1566](https://github.com/Devolutions/IronRDP/issues/1566)) ([161409e18d](https://github.com/Devolutions/IronRDP/commit/161409e18dd185de9bb30730303e56f5e8d28941)) 

  ## Summary
  - Add portable MS-RDPEFS/MS-FSCC filesystem request and completion
  codecs with malformed-input validation and wire tests.
  - Keep RDPDR runtime dispatch, Windows-native backend implementation,
  and client/session integration out of this foundation.
  
  ## Follow-up
  Later stacked PRs provide the backend implementation and runtime
  integration.
  
  ---------

- Add filesystem backend dispatch ([#1578](https://github.com/Devolutions/IronRDP/issues/1578)) ([8f5f3e2515](https://github.com/Devolutions/IronRDP/commit/8f5f3e25154a5edd324a8e89e30ef509a682dbaa)) 

  Route confirmed filesystem requests through portable backend contracts
  and make lifecycle, completion, and announcement state explicit.
  Validate filesystem close padding, release dynamically activated drives
  after rejected announcements, and prevent raw device removal from
  bypassing backend cleanup. The noop backend now rejects unsupported
  filesystem I/O.
  
  Later stacked PRs supply the concrete Windows backend and host
  integration.

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

- Complete MS-RDPESC PDUs and native handles ([#1654](https://github.com/Devolutions/IronRDP/issues/1654)) ([9f3ab27f88](https://github.com/Devolutions/IronRDP/commit/9f3ab27f887a2ee291afaffc97ace455b02e1ac7)) 

  Fill out the remaining MS-RDPESC call/return PDUs and encode
  SCARDCONTEXT/SCARDHANDLE as variable-length native values so x86
  (4-byte) and x64 (8-byte) WinSCard handles round-trip on the wire. Keep
  this change PDU-only; the WinSCard backend lands in stacked follow-ups.

- Plumb smartcard device into RDPDR backends ([#1656](https://github.com/Devolutions/IronRDP/issues/1656)) ([66831bbbba](https://github.com/Devolutions/IronRDP/commit/66831bbbbabe3bf36bedff769c3e62819f60d46b)) 

  Return immediate SvcMessage completions from handle_scard_call so
  backends can finish MS-RDPESC IRPs without blocking the channel path.
  
  Wire WindowsRdpdrBackendFactory::with_smartcard and a minimal
  ScardSession stub that answers decoded calls with
  SCARD_E_UNSUPPORTED_FEATURE. Allow smartcard-only RDPDR products (no
  drives). Full WinSCard work and product CLI/ActiveX enablement remain
  follow-ups.
  
  Depends on #1654.

- Windows WinSCard core smartcard backend ([#1661](https://github.com/Devolutions/IronRDP/issues/1661)) ([1e68bf1a1f](https://github.com/Devolutions/IronRDP/commit/1e68bf1a1f74631c0a1e3dcd16ffb927cae2080c)) 

  Replace the PR2 Windows smartcard stub with a trimmed WinSCard core so
  logon-critical MS-RDPESC IOCTLs complete against the host resource
  manager.

- Extend Windows WinSCard IOCTL coverage ([#1668](https://github.com/Devolutions/IronRDP/issues/1668)) ([992f7d8f5e](https://github.com/Devolutions/IronRDP/commit/992f7d8f5ec46ed1faea7ab57d876811df9f8c42)) 

  Implement remaining MS-RDPESC WinSCard paths on the PR3 core backend:
  LocateCards/ByATR, Control, Get/SetAttrib, GetTransmitCount,
  Read/WriteCache, GetReaderIcon, GetDeviceTypeId, and ContextAndString*
  reader-group admin. Keep A→W execution, buffer probes, and deferred
  workers. Product wiring stays out of scope.
  
  Size gate (counted .rs only): +467/-12 = 479 lines, 2 files (size/L).
  PR5 still owns daemon/agent/viewer/ActiveX product wiring.



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.6.0...ironrdp-rdpdr-v0.7.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.5.0...ironrdp-rdpdr-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- Notify RdpdrBackend of 'User Logged On' Messages ([#1211](https://github.com/Devolutions/IronRDP/issues/1211)) ([1a09dbaca9](https://github.com/Devolutions/IronRDP/commit/1a09dbaca9dd5d35025ee50aaa645100222be189)) 

- Add Web RDPDR virtual printer support ([#1230](https://github.com/Devolutions/IronRDP/issues/1230)) ([14b1cef9cb](https://github.com/Devolutions/IronRDP/commit/14b1cef9cbbd0d8ef5e1fc8c73a3003a5e9f9bc2)) 

  Adds RDPDR virtual printer redirection for web sessions, enabling the web client to announce a redirected printer, receive server print jobs over RDPDR, and deliver completed PostScript jobs to a browser callback.

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

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


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
