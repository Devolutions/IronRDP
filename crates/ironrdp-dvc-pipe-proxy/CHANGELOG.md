# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.4.1...ironrdp-dvc-pipe-proxy-v0.5.0)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.

- Remove trailing punctuation from log messages ([#1380](https://github.com/Devolutions/IronRDP/issues/1380)) ([f38554277a](https://github.com/Devolutions/IronRDP/commit/f38554277a1af3085c1fa5739cda515939d09abf)) 

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.4.0...ironrdp-dvc-pipe-proxy-v0.4.1)] - 2026-06-05



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.3.0...ironrdp-dvc-pipe-proxy-v0.4.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` and `ironrdp-svc` public dependencies

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.2.0...ironrdp-dvc-pipe-proxy-v0.2.1)] - 2025-09-24

### <!-- 4 -->Bug Fixes

- Change dvc proxy pipe mode from Message to Byte on Windows (#986) ([5f52a44b84](https://github.com/Devolutions/IronRDP/commit/5f52a44b840dd71eae6a355be00f1c4c671b3b58)) 

- Add blocking logic for sending dvc pipe messages ([3182a018e2](https://github.com/Devolutions/IronRDP/commit/3182a018e2972eb77c52ea248387c96a9eb6a6a6)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.1.0...ironrdp-dvc-pipe-proxy-v0.2.0)] - 2025-08-29

### <!-- 1 -->Features

- Make dvc named pipe proxy cross-platform (#896) ([166b76010c](https://github.com/Devolutions/IronRDP/commit/166b76010cbd8f8674e6e8d4801fee5cda1ad9e5)) 

  - Make dvc named pipe proxy cross-platform (Unix implementation via
  `tokio::net::unix::UnixStream`)
  - Removed unsafe code for Windows implementation, switched to
  `tokio::net::windows::named_pipe`
