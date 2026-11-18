# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpeai-v0.2.0...ironrdp-rdpeai-v0.3.0)] - 2026-11-18

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

- Add validity-guarded clipboard and audio server events ([#2443](https://github.com/Devolutions/IronRDP/issues/2443)) ([865b2aa3b3](https://github.com/Devolutions/IronRDP/commit/865b2aa3b37ec9ec48abc38b2cbed69b68b9e38d)) 

  Allow clipboard, audio output, and audio input producers to attach a
  nonblocking validity predicate to queued server events so events that go
  stale behind earlier transport writes do not mutate channel state,
  consume `WAVE_KEEP` slots, or write to the connection.
  
  Add channel-targeted `RdpeaiServerMessage::OpenOnChannel` and
  `ChangeFormatOnChannel` variants so runtime-created `AUDIO_INPUT`
  dynamic channels can be driven by channel ID.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpeai-v0.1.0...ironrdp-rdpeai-v0.2.0)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11

- [**breaking**] Update `ironrdp-rdpsnd` public dependency to 0.11

- [**breaking**] Update `ironrdp-svc` public dependency to 0.10
