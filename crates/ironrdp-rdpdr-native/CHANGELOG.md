# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.7.0...ironrdp-rdpdr-native-v0.8.0)] - 2026-10-23

### <!-- 1 -->Features

- Add `WindowsRdpdrBackend` and `WindowsRdpdrBackendFactory`, a Windows filesystem backend that redirects one or more drives or directories, confines server paths below each root, and supports directory queries, change notifications, byte-range locks, security descriptors, alternate streams and volume information ([#1587](https://github.com/Devolutions/IronRDP/issues/1587)) ([7dce8a306f](https://github.com/Devolutions/IronRDP/commit/7dce8a306f43462677879905642967066c42337f)) ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61)) ([#1600](https://github.com/Devolutions/IronRDP/issues/1600)) ([1fbc9bab0b](https://github.com/Devolutions/IronRDP/commit/1fbc9bab0bc26d8fe0789d5215005d7ea22e2a54)) ([#1616](https://github.com/Devolutions/IronRDP/issues/1616)) ([a724f783d1](https://github.com/Devolutions/IronRDP/commit/a724f783d1638b4e215507849c1cf148887f30d5)) ([#2178](https://github.com/Devolutions/IronRDP/issues/2178)) ([1de38a2ee5](https://github.com/Devolutions/IronRDP/commit/1de38a2ee55346a577ec28f3706c81dd3f7e8316))

- Add Windows drive-letter redirection and hotplug with `WindowsRdpdrBackendFactory::with_logical_drives`, `from_drive_configuration`, `with_dynamic_drives`, the `LogicalDrive` types and `LogicalDriveWatcher` ([#1780](https://github.com/Devolutions/IronRDP/issues/1780)) ([5e05637f06](https://github.com/Devolutions/IronRDP/commit/5e05637f0659d460d6e02b5567bb728f1d04585a)) ([#2178](https://github.com/Devolutions/IronRDP/issues/2178)) ([1de38a2ee5](https://github.com/Devolutions/IronRDP/commit/1de38a2ee55346a577ec28f3706c81dd3f7e8316))

- Redirect smart cards to the local WinSCard resource manager on Windows, enabled with `WindowsRdpdrBackendFactory::with_smartcard` ([#1661](https://github.com/Devolutions/IronRDP/issues/1661)) ([1e68bf1a1f](https://github.com/Devolutions/IronRDP/commit/1e68bf1a1f74631c0a1e3dcd16ffb927cae2080c)) ([#1668](https://github.com/Devolutions/IronRDP/issues/1668)) ([992f7d8f5e](https://github.com/Devolutions/IronRDP/commit/992f7d8f5ec46ed1faea7ab57d876811df9f8c42)) ([#1672](https://github.com/Devolutions/IronRDP/issues/1672)) ([1561a4c327](https://github.com/Devolutions/IronRDP/commit/1561a4c327512a1a3a6c618d4b06dc388fab132f))

- Redirect Windows printers with `WindowsPrinterRedirection` and `WindowsRdpdrBackendFactory::with_printer_redirection` or `with_default_printer`: the default queue, every local queue or named queues, with per-queue driver overrides ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) ([eaadd650c9](https://github.com/Devolutions/IronRDP/commit/eaadd650c9e2be98b57274b72095bf41da6ac3dd)) ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995))

- Add `FileRdpdrPrinterCacheStore` to persist server printer settings across connections ([#2177](https://github.com/Devolutions/IronRDP/issues/2177)) ([8575afc308](https://github.com/Devolutions/IronRDP/commit/8575afc308ba7f67fced0b0f61ce07fa4aca5995))

- Add `RdpdrWorkerThreadHooks` and `WindowsRdpdrBackendFactory::with_worker_thread_hooks` to run setup code on backend worker threads ([#1876](https://github.com/Devolutions/IronRDP/issues/1876)) ([eaadd650c9](https://github.com/Devolutions/IronRDP/commit/eaadd650c9e2be98b57274b72095bf41da6ac3dd))

- Redirect serial and parallel ports on Windows with `WindowsPortRedirection`, `PortSelection` and `WindowsRdpdrBackendFactory::with_port_redirection` ([#2187](https://github.com/Devolutions/IronRDP/issues/2187)) ([850839f475](https://github.com/Devolutions/IronRDP/commit/850839f4756f4efb486a47abb89a93b3b4694a40))

- Redirect a printer on Linux and macOS with the `printer` module and `NixRdpdrBackend::with_printer`, and add `NixRdpdrBackendFactory` to build those backends with drives and a printer ([#2017](https://github.com/Devolutions/IronRDP/issues/2017)) ([d839ad3bd0](https://github.com/Devolutions/IronRDP/commit/d839ad3bd0e71dd44b68dd74bf122e108a5cbefb))

### <!-- 4 -->Bug Fixes

- Complete flush-buffers and security IRPs with `STATUS_NOT_SUPPORTED` in `NixRdpdrBackend` ([#1590](https://github.com/Devolutions/IronRDP/issues/1590)) ([d1c63ecd7b](https://github.com/Devolutions/IronRDP/commit/d1c63ecd7bd13c9ae3d88f3ae84176f214f41f61))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-rdpdr` public dependency to 0.8

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9



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
