# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-dvc-com-plugin-v0.1.0)] - 2026-03-01

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


