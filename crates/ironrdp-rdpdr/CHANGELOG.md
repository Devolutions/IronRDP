# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-v0.7.0...ironrdp-rdpdr-v0.8.0)] - 2026-10-23

### <!-- 1 -->Features

- Add MS-RDPEFS filesystem PDUs for flush buffers, change notifications, byte-range locks, security queries and updates, volume set-information and stream information, along with more `NtStatus` codes ([#1566](https://github.com/Devolutions/IronRDP/issues/1566)) ([161409e18d](https://github.com/Devolutions/IronRDP/commit/161409e18dd185de9bb30730303e56f5e8d28941))

- [**breaking**] Add `FileInformationClass::Stream`, the `FlushBuffers`, `QuerySecurity` and `SetSecurity` variants of `MajorFunction`, three `ServerDriveIoRequest` variants and five `RdpdrPdu` variants ([#1566](https://github.com/Devolutions/IronRDP/issues/1566)) ([161409e18d](https://github.com/Devolutions/IronRDP/commit/161409e18dd185de9bb30730303e56f5e8d28941)) ([#1587](https://github.com/Devolutions/IronRDP/issues/1587)) ([7dce8a306f](https://github.com/Devolutions/IronRDP/commit/7dce8a306f43462677879905642967066c42337f)) ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61))

- [**breaking**] Add the `operation`, `wait` and `locks` fields to `ServerDriveLockControlRequest` ([#1566](https://github.com/Devolutions/IronRDP/issues/1566)) ([161409e18d](https://github.com/Devolutions/IronRDP/commit/161409e18dd185de9bb30730303e56f5e8d28941))

- Add `RdpdrBackend` methods with default implementations for drive lifecycle (`reset`, `restore_drive`, `add_drive`, `remove_drive`), filesystem device control, deferred completions (`poll_deferred_messages`) and security support (`supports_drive_security`) ([#1578](https://github.com/Devolutions/IronRDP/issues/1578)) ([8f5f3e2515](https://github.com/Devolutions/IronRDP/commit/8f5f3e25154a5edd324a8e89e30ef509a682dbaa)) ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61))

- Add drive hotplug with `Rdpdr::add_dynamic_drive`, `Rdpdr::remove_drive`, `Rdpdr::drive_hotplug_available` and `RdpdrBackendProduct::with_drive_hotplug`, and `Rdpdr::poll_deferred_messages` to send deferred IRP completions ([#1578](https://github.com/Devolutions/IronRDP/issues/1578)) ([8f5f3e2515](https://github.com/Devolutions/IronRDP/commit/8f5f3e25154a5edd324a8e89e30ef509a682dbaa)) ([#1780](https://github.com/Devolutions/IronRDP/issues/1780)) ([5e05637f06](https://github.com/Devolutions/IronRDP/commit/5e05637f0659d460d6e02b5567bb728f1d04585a))

- Add `RdpdrBackendFactory`, `RdpdrBackendProduct` and `RdpdrDrive` to build a fresh backend and device set for each connection ([#1600](https://github.com/Devolutions/IronRDP/issues/1600)) ([1fbc9bab0b](https://github.com/Devolutions/IronRDP/commit/1fbc9bab0bc26d8fe0789d5215005d7ea22e2a54))

- Decode the PDUs a client sends and encode the PDUs a server sends, so the PDU layer works in both directions ([#1779](https://github.com/Devolutions/IronRDP/issues/1779)) ([a993f91408](https://github.com/Devolutions/IronRDP/commit/a993f914080bcca69341031e1ac54bc5c4c60213))

- [**breaking**] Require `Into<u32> + Copy` on `IoCtlCode` implementations ([#1779](https://github.com/Devolutions/IronRDP/issues/1779)) ([a993f91408](https://github.com/Devolutions/IronRDP/commit/a993f914080bcca69341031e1ac54bc5c4c60213))

- Add `server::RdpdrServer`, the `RdpdrServerBackend` trait and `RdpdrServerMessage` for server-side device redirection ([#1783](https://github.com/Devolutions/IronRDP/issues/1783)) ([fb5f662af9](https://github.com/Devolutions/IronRDP/commit/fb5f662af9d8f3ee39429b9da00f84c5cad8582a)) ([#1784](https://github.com/Devolutions/IronRDP/issues/1784)) ([695af5a6e8](https://github.com/Devolutions/IronRDP/commit/695af5a6e814badb3daa60184a8d2048d0e66716))

- Report byte offsets in RDPDR PDU decode and encode errors ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

- [**breaking**] Encode smart card contexts and handles as variable-length native values, making the `ScardContext` and `ScardHandle` fields private ([#1654](https://github.com/Devolutions/IronRDP/issues/1654)) ([9f3ab27f88](https://github.com/Devolutions/IronRDP/commit/9f3ab27f887a2ee291afaffc97ace455b02e1ac7))

- [**breaking**] Add the remaining MS-RDPESC calls and returns, adding eleven `ScardCall` variants ([#1654](https://github.com/Devolutions/IronRDP/issues/1654)) ([9f3ab27f88](https://github.com/Devolutions/IronRDP/commit/9f3ab27f887a2ee291afaffc97ace455b02e1ac7))

- [**breaking**] Return the completions to send from `RdpdrBackend::handle_scard_call` ([#1656](https://github.com/Devolutions/IronRDP/issues/1656)) ([66831bbbba](https://github.com/Devolutions/IronRDP/commit/66831bbbbabe3bf36bedff769c3e62819f60d46b))

- [**breaking**] Support smart card length probes: `ListReadersReturn::new` takes a `CharacterSet`; `ListReadersReturn` gains `encoding` and `c_bytes`, `StatusReturn` gains `reader_c_bytes`, `TransmitReturn` gains `recv_len` and `ReadCacheReturn` gains `data_len`; and the buffers of these returns, `ControlReturn` and `GetAttribReturn` are optional ([#1661](https://github.com/Devolutions/IronRDP/issues/1661)) ([1e68bf1a1f](https://github.com/Devolutions/IronRDP/commit/1e68bf1a1f74631c0a1e3dcd16ffb927cae2080c)) ([#1668](https://github.com/Devolutions/IronRDP/issues/1668)) ([992f7d8f5e](https://github.com/Devolutions/IronRDP/commit/992f7d8f5ec46ed1faea7ab57d876811df9f8c42))

- Add `ScardCall::error_response`, `pdu::esc::complete_call`, `ScardContext::empty` and `ScardHandle::empty` to complete failed smart card calls as MS-RDPESC requires ([#2175](https://github.com/Devolutions/IronRDP/issues/2175)) ([b735d190dd](https://github.com/Devolutions/IronRDP/commit/b735d190ddfd40fe2ecdf818f96235f185f3420d))

- Add `RdpdrPrinter`, `Rdpdr::with_redirected_printer`, `Rdpdr::with_printer_driver_and_network` and `RdpdrBackendProduct::with_printers` to announce several printers with driver, network, default and cached-configuration metadata, each under its own `PRN<n>` port name ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) ([eaadd650c9](https://github.com/Devolutions/IronRDP/commit/eaadd650c9e2be98b57274b72095bf41da6ac3dd)) ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995))

- Add `RdpdrPrinterCacheStore`, `InMemoryRdpdrPrinterCacheStore` and the `pdu::epc` module to save server printer cache data and replay it in later announcements ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995))

- Add `RdpdrBackend::reject_printer_write` to reject oversized printer writes before their payload is allocated ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) ([eaadd650c9](https://github.com/Devolutions/IronRDP/commit/eaadd650c9e2be98b57274b72095bf41da6ac3dd))

- [**breaking**] Return whether the printer was added from `Devices::add_printer` and `Devices::add_printer_with_driver` ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995))

- Add serial and parallel port redirection with `RdpdrPort`, `RdpdrBackendProduct::with_ports`, `Rdpdr::with_port`, `RdpdrBackend::handle_port_io_request` and the `pdu::esp` module ([#2187](https://github.com/Devolutions/IronRDP/issues/2187)) ([850839f475](https://github.com/Devolutions/IronRDP/commit/850839f4756f4efb486a47abb89a93b3b4694a40))

### <!-- 4 -->Bug Fixes

- Fail filesystem I/O in `NoopRdpdrBackend` instead of leaving the request unanswered ([#1578](https://github.com/Devolutions/IronRDP/issues/1578)) ([8f5f3e2515](https://github.com/Devolutions/IronRDP/commit/8f5f3e25154a5edd324a8e89e30ef509a682dbaa))

- Answer every smart card call in `NoopRdpdrBackend` with `SCARD_E_NO_SERVICE`, so server-side WinSCard calls no longer hang ([#2175](https://github.com/Devolutions/IronRDP/issues/2175)) ([b735d190dd](https://github.com/Devolutions/IronRDP/commit/b735d190ddfd40fe2ecdf818f96235f185f3420d))

- Consume the whole Server Drive Query Information Request, so a non-empty `QueryBuffer` no longer desynchronizes parsing ([#1810](https://github.com/Devolutions/IronRDP/issues/1810)) ([476e8a174d](https://github.com/Devolutions/IronRDP/commit/476e8a174d6a61b7dbadbf4fe0e3e2290fbfc491))

- Complete well-formed drive IRPs that are not implemented with `STATUS_NOT_SUPPORTED` instead of ending the session ([#2174](https://github.com/Devolutions/IronRDP/issues/2174)) ([0f9154d212](https://github.com/Devolutions/IronRDP/commit/0f9154d2128f8c65f4d51f6b7eb7cb1d66a12329))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9



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
