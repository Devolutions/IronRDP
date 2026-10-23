# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.3...ironrdp-dvc-com-plugin-v0.2.0)] - 2026-10-23

### <!-- 1 -->Features

- Add `load_dvc_plugin_listeners` and `DvcComChannelListener` so a plugin channel can be reopened when the server closes and recreates it ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb))

- Add `process_plugin_request` to run a single request through a plugin channel on a dedicated COM thread ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb))

- Add `webauthn_dll_path` and `process_webauthn_dll_request` to forward a request to the system `webauthn.dll` over `WebAuthN_Channel` ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb))

### <!-- 4 -->Bug Fixes

- Keep a plugin loaded until its last channel is dropped, instead of shutting it down when any one of its channels is dropped ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9



## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.2...ironrdp-dvc-com-plugin-v0.1.3)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.



## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.1...ironrdp-dvc-com-plugin-v0.1.2)] - 2026-06-05



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.0...ironrdp-dvc-com-plugin-v0.1.1)] - 2026-05-27

### <!-- 7 -->Build

- Update dependencies.
