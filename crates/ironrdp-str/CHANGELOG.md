# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-str-v0.1.1...ironrdp-str-v0.2.0)] - 2026-10-23

### <!-- 1 -->Features

- Report the byte offset at which string decoding or encoding failed in the returned error ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

### <!-- 7 -->Build

- [**breaking**] Update the `ironrdp-core` dependency to 0.3, whose `Encode` and `DecodeOwned` traits the string types implement

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-str-v0.1.0...ironrdp-str-v0.1.1)] - 2026-05-27

### <!-- 7 -->Build

- Update dependencies.
