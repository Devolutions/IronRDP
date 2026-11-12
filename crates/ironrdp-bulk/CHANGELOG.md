# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-bulk-v0.1.1...ironrdp-bulk-v0.2.0)] - 2026-10-23

### <!-- 1 -->Features

- Add the `rdp8` module with `Rdp8Decompressor`, an RDP 8.0 bulk decompressor with RDP 8.0 and RDP 8.0 Lite profiles ([#2173](https://github.com/Devolutions/IronRDP/issues/2173)) ([c3dd17e607](https://github.com/Devolutions/IronRDP/commit/c3dd17e6071342aa5f92b743a89a38382665eb64))

### <!-- 4 -->Bug Fixes

- [**breaking**] Make `BulkCompressor::new` infallible; it now returns `Self` ([#1255](https://github.com/Devolutions/IronRDP/issues/1255)) ([0dc0194418](https://github.com/Devolutions/IronRDP/commit/0dc0194418375d504a8041b75ba250dc8eeb21ad))

- Reset the RDP 6.1 (XCRUSH) history on a flushed packet that is not compressed, and return its data unchanged instead of parsing it as compressed ([#1518](https://github.com/Devolutions/IronRDP/issues/1518)) ([6151e21bf5](https://github.com/Devolutions/IronRDP/commit/6151e21bf58b7297e9b4abc2167aa36fc2ba77e4))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-bulk-v0.1.0...ironrdp-bulk-v0.1.1)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Gate alloc-dependent modules behind the alloc feature ([#1279](https://github.com/Devolutions/IronRDP/issues/1279)) ([18a430a51a](https://github.com/Devolutions/IronRDP/commit/18a430a51aca07aa45db4642df5c932ef65d2016)) 

- Off-by-one in forward match loop causes panic during compression ([#1293](https://github.com/Devolutions/IronRDP/issues/1293)) ([0dd7c94ba2](https://github.com/Devolutions/IronRDP/commit/0dd7c94ba22e9bd11b4ea36fd03af3bfcccecab8)) 

### <!-- 7 -->Build

- Bump criterion from 0.5.1 to 0.8.1 ([#1184](https://github.com/Devolutions/IronRDP/issues/1184)) ([d92dd382b3](https://github.com/Devolutions/IronRDP/commit/d92dd382b3fbaa163f355f6489db45ca8a3e7498)) 


