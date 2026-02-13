# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-egfx-v0.1.0)] - 2026-02-13

### <!-- 1 -->Features

- Add MS-RDPEGFX Graphics Pipeline Extension ([#1057](https://github.com/Devolutions/IronRDP/issues/1057)) ([300f9a3ea5](https://github.com/Devolutions/IronRDP/commit/300f9a3ea55d0bcaf5ce5b0a8ebf4a06897109e0)) 



### Added

- Initial release
- MS-RDPEGFX PDU types (all 23 PDUs)
- Client-side DVC processor
- Server-side implementation with:
  - Multi-surface management (Offscreen Surfaces ADM element)
  - Frame tracking with flow control (Unacknowledged Frames ADM element)
  - V8/V8.1/V10/V10.1-V10.7 capability negotiation
  - AVC420 and AVC444 frame sending
  - QoE metrics processing
  - Cache import handling
  - Resize coordination
