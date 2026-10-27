# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-str-v0.2.0...ironrdp-str-v0.2.1)] - 2026-10-27

### <!-- 7 -->Build

- Mark exposed dependencies as public ([#2213](https://github.com/Devolutions/IronRDP/issues/2213)) ([405e6576ba](https://github.com/Devolutions/IronRDP/commit/405e6576bac7c64b04efc8a0d00beeecdc648356)) 

  These crates expose types or traits from these dependencies in their
  public API, so a breaking release of the dependency requires a breaking
  release of the crate:
  
  - `ironrdp-str`: `ironrdp-core`, whose `Encode` and `DecodeOwned` traits
  the string types implement.
  - `ironrdp-dvc-pipe-proxy`: `ironrdp-dvc`, through `DvcNamedPipeProxy`'s
  `DvcProcessor` implementation.
  - `ironrdp-dvc-com-plugin`: `ironrdp-dvc`, plus `ironrdp-pdu` and
  `ironrdp-svc` through the `PduResult` return type and the `SvcMessage`
  callback bound of `load_dvc_plugin`.
  - `ironrdp-mstsgu`: `ironrdp-error`, whose `Error<GwErrorKind>` the
  `GwClient` methods return.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-str-v0.1.1...ironrdp-str-v0.2.0)] - 2026-10-23

### <!-- 1 -->Features

- Report the byte offset at which string decoding or encoding failed in the returned error ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

### <!-- 7 -->Build

- [**breaking**] Update the `ironrdp-core` dependency to 0.3, whose `Encode` and `DecodeOwned` traits the string types implement

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-str-v0.1.0...ironrdp-str-v0.1.1)] - 2026-05-27

### <!-- 7 -->Build

- Update dependencies.
