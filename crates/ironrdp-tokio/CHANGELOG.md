# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.8.0...ironrdp-tokio-v0.8.1)] - 2026-03-01

### <!-- 0 -->Security

- Add DVC COM plugin loader for native Windows DVC client plugins ([9c987bcb40](https://github.com/Devolutions/IronRDP/commit/9c987bcb40a12712fa649e1087f6cb922f9bb75c)) 

  Implements support for loading and using native Windows Dynamic Virtual
  Channel (DVC) client plugin DLLs through the COM-based IWTSPlugin API.
  This enables IronRDP to leverage existing Windows DVC plugins such as
  webauthn.dll for hardware security key support via RDP.
  
  New crate: ironrdp-dvc-com-plugin
  - Implements IWTSVirtualChannelManager and IWTSVirtualChannel COM interfaces
  - Manages plugin lifecycle on dedicated COM worker thread
  - Handles channel open/close/reopen cycles with per-instance write callbacks
  - Properly bridges between COM synchronous calls and IronRDP's async runtime
  
  Client integration:
  - Add --dvc-plugin CLI argument to ironrdp-client
  - Load plugins in both TCP and WebSocket connection paths
  - Windows-only conditional compilation for cross-platform builds
  
  Additional fixes:
  - Fix pre-existing crash in ironrdp-tokio KDC handler on 64-bit Windows
    (usize to u32 conversion in reqwest.rs)
  - Add proper error handling using try_from instead of unsafe as casts
  - All changes pass cargo fmt and cargo clippy with strict pedantic lints
  
  Tested with: C:\Windows\System32\webauthn.dll



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.7.0...ironrdp-tokio-v0.8.0)] - 2025-12-18

### <!-- 1 -->Features

- Add MovableTokioFramed for Send+!Sync context ([#1033](https://github.com/Devolutions/IronRDP/issues/1033)) ([966ba8a53e](https://github.com/Devolutions/IronRDP/commit/966ba8a53e43a193271f40b9db80e45e495e2f24)) 

  The `ironrdp-tokio` crate currently provides the following two
  `Framed<S>` implementations using the standard `tokio::io` traits:
  - `type TokioFramed<S> = Framed<TokioStream<S>>` where `S: Send + Sync +
  Unpin`
  - `type LocalTokioFramed<S> = Framed<LocalTokioStream<S>>` where `S:
  Unpin`
  
  The former is meant for multi-threaded runtimes and the latter is meant
  for single-threaded runtimes.
  
  This PR adds a third `Framed<S>` implementation:
  
  `pub type MovableTokioFramed<S> = Framed<MovableTokioStream<S>>` where
  `S: Send + Unpin`
  
  This is a valid usecase as some implementations of the `tokio::io`
  traits are `Send` but `!Sync`. Without this new third type, consumers of
  `Framed<S>` who have a `S: Send + !Sync` trait for their streams are
  forced to downgrade to `LocalTokioFramed` and do some hacky workaround
  with `tokio::task::spawn_blocking` since the defined associated futures,
  `ReadFut` and `WriteAllFut`, are neither `Send` nor `Sync`.

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

### <!-- 7 -->Build

- Bump picky and sspi ([#1028](https://github.com/Devolutions/IronRDP/issues/1028)) ([5bd319126d](https://github.com/Devolutions/IronRDP/commit/5bd319126d32fbd8e505508e27ab2b1a18a83d04)) 

  This fixes build issues with some dependencies.

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.5.1...ironrdp-tokio-v0.6.0)] - 2025-07-08

### Build

- Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.5.0...ironrdp-tokio-v0.5.1)] - 2025-07-08

### <!-- 1 -->Features

- Add async `ReqwestNetworkClient::send` method (#859) ([7e23a8bb97](https://github.com/Devolutions/IronRDP/commit/7e23a8bb97991d0e24e65d77a11d9854492ee024)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.4.0...ironrdp-tokio-v0.5.0)] - 2025-06-06

### <!-- 4 -->Bug Fixes

- [**breaking**] Adjust reqwest-related features (#812) ([9408789491](https://github.com/Devolutions/IronRDP/commit/9408789491b3e09b69e0aaa03fd215326b624ec0)) 

  - Remove `reqwest` from the default feature set.
  - Disable default TLS backend.
  - Add `reqwest-rustls-ring` to enable rustls + ring backend.
  - Add `reqwest-native-tls` to enable native-tls backend.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.3.0...ironrdp-tokio-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- Add reqwest feature (#734) ([032c38be92](https://github.com/Devolutions/IronRDP/commit/032c38be9229cfd35f0f6fc8eac5cccc960480d3)) 

## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.2...ironrdp-tokio-v0.2.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.1...ironrdp-tokio-v0.2.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tokio-v0.2.0...ironrdp-tokio-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
