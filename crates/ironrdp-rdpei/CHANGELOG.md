# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpei-v0.3.0...ironrdp-rdpei-v0.3.1)] - 2026-11-28

### <!-- 1 -->Features

- Bound dynamic channel messages and report RDPSND backpressure ([#2441](https://github.com/Devolutions/IronRDP/issues/2441)) ([e05f890d0a](https://github.com/Devolutions/IronRDP/commit/e05f890d0a6db5121eda2767cb4ca812242461e8)) 

  Cap inbound dynamic virtual channel message reassembly to 64 KiB on
  `RdpeiServer` and 256 KiB on `GraphicsPipelineServer`, `GfxDvcBridge`,
  and `RdpeaiServer`, and forward RDPEI channel teardown to
  `RdpeiHandler::close`.
  
  Bound `TouchFrame`, `TouchEventPdu`, `PenFrame`, and `PenEventPdu`
  preallocation against the remaining payload bytes so a 2-byte count
  header cannot preallocate more items than the buffer can hold.
  
  Add `RdpsndServerHandler::unconfirmed_waves_changed` and invoke it on
  wave transmission and on the first timestamp-valid `WaveConfirm` so
  audio producers can track in-flight unconfirmed waves.



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpei-v0.2.0...ironrdp-rdpei-v0.3.0)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11

- [**breaking**] Update `ironrdp-svc` public dependency to 0.10



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpei-v0.1.0...ironrdp-rdpei-v0.2.0)] - 2026-10-23

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9

## [0.1.0] - 2026-10-23

### Added

- Initial MS-RDPEI implementation: variable-length integers, all input PDUs, client and server DVC processors.
