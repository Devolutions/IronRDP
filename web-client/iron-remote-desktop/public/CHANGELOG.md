# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.11.0] - 2026-05-26

### <!-- 1 -->Features

- Expose granular RDCleanPath error details ([#1117](https://github.com/Devolutions/IronRDP/issues/1117)) ([2911124e8f](https://github.com/Devolutions/IronRDP/commit/2911124e8fe6160bc8ba03a574b67077e6d2cca9))

  Surface HTTP status, WSA, and TLS alert codes from RDCleanPath
  errors so consumers can distinguish specific network failures
  (e.g. `WSAEACCES`/10013) instead of a generic message.

- Clipboard file transfer API surface ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9))

  Backend-agnostic API for clipboard file upload and download,
  consumed by backends that implement CLIPRDR file transfer.

### <!-- 4 -->Bug Fixes

- Disable clipboard polling loop on Firefox v127+ ([#1162](https://github.com/Devolutions/IronRDP/issues/1162)) ([9a1ac3092e](https://github.com/Devolutions/IronRDP/commit/9a1ac3092ee3eac3e81823349d8e027065f5b8f8))

- Release mouse and keyboard state on focus loss to resolve Firefox stuck right-click ([#1297](https://github.com/Devolutions/IronRDP/issues/1297)) ([c56ea16d05](https://github.com/Devolutions/IronRDP/commit/c56ea16d05a88109815906b6d2501cfdae4c07c4))

  `mouseOut()` and a new `focusLost()` handler release pressed
  buttons and keys when the canvas loses focus (mouseleave, window
  `blur`, document `visibilitychange`). `mouseIn()` reconciles
  tracked server-side button state against `event.buttons` on
  re-entry.

- Include Meta keys in WebKit scancode dispatch ([#1304](https://github.com/Devolutions/IronRDP/issues/1304)) ([0bbffcd0ec](https://github.com/Devolutions/IronRDP/commit/0bbffcd0ec54eb9a14950db5f65f9a164dabc05d))
