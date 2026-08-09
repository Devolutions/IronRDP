# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.3.0...ironrdp-egfx-v0.3.1)] - 2026-08-09

### <!-- 1 -->Features

- Add ClearCodec client-side decode dispatch ([#1175](https://github.com/Devolutions/IronRDP/issues/1175)) ([714dce4662](https://github.com/Devolutions/IronRDP/commit/714dce46627e299c57d82f4f6a5c18067a95bffa)) 

  Follow-up to #1174. Supersedes #1195 (the standalone server-helper PR;
  its 46-line `send_clearcodec_frame()` is included here).
  
  Wires ClearCodec into the EGFX client's WireToSurface1 codec dispatch,
  matching the existing AVC420 and Uncompressed decode patterns.

- Composite client surface commands into pixel buffers ([#1460](https://github.com/Devolutions/IronRDP/issues/1460)) ([9cd36952ca](https://github.com/Devolutions/IronRDP/commit/9cd36952ca18196cf72c85dc42a79d5d1f5620d5)) 

- Decode Planar bitmaps in the client ([#1507](https://github.com/Devolutions/IronRDP/issues/1507)) ([66c8a81be0](https://github.com/Devolutions/IronRDP/commit/66c8a81be0a9f966e3cf4935ca2a0274d10b063f)) 

  Wires `RDPGFX_CODECID_PLANAR` (0x000A) into the EGFX client's
  `WireToSurface1` dispatch, alongside the existing ClearCodec and
  Uncompressed paths.

- Add egfx_avc420_decode oracle and target ([#1326](https://github.com/Devolutions/IronRDP/issues/1326)) ([cafbef1c9f](https://github.com/Devolutions/IronRDP/commit/cafbef1c9faba78acb3e33a39556eb4c4c78c6d4)) 

  This change adds an assertion-or-panic fuzz oracle for the AVC
  length-prefix to Annex-B conversion that runs inside
  `OpenH264Decoder::decode` before any OpenH264 entry point. The same
  change refactors the conversion from a private method on
  `OpenH264Decoder` into two public free functions in
  `ironrdp_egfx::pdu::avc`. The first function, `avc_to_annex_b`,
  returns a fresh `Vec<u8>` and is symmetric with the existing
  `annex_b_to_avc`. The second function, `avc_to_annex_b_into`, writes
  into a caller-provided buffer and preserves the per-frame buffer-reuse
  optimization that `OpenH264Decoder` relied on.
  
  The conversion is now available unconditionally to any consumer of
  `ironrdp-egfx`. The previous `#[cfg(feature = "openh264")]` gating
  went with the location, not the bytes-to-bytes logic, so lifting the
  function out of `decode.rs` removed the gate too.
  
  The new oracle exercises two input distributions on every fuzz call:
  
  - Direct path: the oracle calls `avc_to_annex_b(data)` on the raw fuzz
    input. This exercises the wrapper on arbitrary byte distributions,
    including inputs that do not parse as `Avc420BitmapStream`.
  - Decode-chain path: the oracle tries
  `Avc420BitmapStream::decode(data)`;
    on success it calls `avc_to_annex_b(stream.data)`. This exercises the
    wrapper on the realistic post-decode payload distribution.
  
  The oracle catches panics in the wrapper, OOM allocation from
  attacker-controlled NAL length encoding, and contract violations on the
  produced Annex-B byte stream. The oracle does NOT catch OpenH264
  internal bugs (OSS-Fuzz coverage), the YUV-to-RGBA conversion path
  downstream of OpenH264 (separate workstream), or AVC444 luma plus
  chroma split (sibling target).
  
  Smoke fuzz ran 10,922,695 iterations in 31 seconds at ~352K exec/s
  sustained with zero crashes. Coverage settled at 158 lines, 456
  features, 69 corpus entries.
  
  The new target auto-discovers into CI via the `cargo xtask fuzz list`
  dynamic fan-out mechanism. The new `check_egfx_avc420_decode`
  regression-replay test in
  `crates/ironrdp-testsuite-core/tests/fuzz_regression.rs` runs against
  the seed corpus and passes.

### <!-- 4 -->Bug Fixes

- Preserve AVC_DISABLED during negotiation ([#1490](https://github.com/Devolutions/IronRDP/issues/1490)) ([c5bd574c8a](https://github.com/Devolutions/IronRDP/commit/c5bd574c8ac7e4ba92c31348187a196d3e84ae70)) 

- Add spec-compliant Planar frame sender ([#1498](https://github.com/Devolutions/IronRDP/issues/1498)) ([409b256b41](https://github.com/Devolutions/IronRDP/commit/409b256b4168b3a63d97998da311fa9e761bef3e)) 

  ## Summary

- Bound the compositor's dirty-region metadata ([#1510](https://github.com/Devolutions/IronRDP/issues/1510)) ([81f7392422](https://github.com/Devolutions/IronRDP/commit/81f73924221ea994d7f5f32bc9113caa13551ae6)) 

  ## Summary
  
  - The compositor charges materialized pixel buffers against
  `MAX_COMPOSITOR_BYTES` but not the `DirtyRegion` entries that produce
  them, so `frame` grows outside the budget.
  - The repeat filter is O(1) and compares only against the previous
  entry, so it collapses a rectangle repeated 65,535 times but not two
  rectangles alternating. The frame stays open until the peer sends
  `EndFrame`, and the peer chooses when that happens.
  - `record_dirty` now charges one entry before pushing, and `EndFrame`
  releases the whole set before materializing so the pixel copies can
  spend what the metadata was holding.
  
  ## Cost to the peer
  
  `RDPGFX_POINT16` (2.2.1.1) is four bytes on the wire; `RDPGFX_RECT16`
  (2.2.1.2) is eight. A `DirtyRegion` is ten bytes resident. Three
  commands loop over these arrays and record one dirty region each:
  
  | PDU | Array element | Wire | Resident | Ratio |
  |---|---|---|---|---|
  | `RDPGFX_SOLIDFILL_PDU` (2.2.2.4) | `fillRects`, RECT16 | 8 | 10 |
  1.25x |
  | `RDPGFX_SURFACE_TO_SURFACE_PDU` (2.2.2.5) | `destPts`, POINT16 | 4 |
  10 | 2.5x |
  | `RDPGFX_CACHE_TO_SURFACE_PDU` (2.2.2.7) | `destPts`, POINT16 | 4 | 10
  | 2.5x |
  
  The ratios are modest. The point is that there was no ceiling at all, so
  a sustained stream grows the queue until the client is out of memory
  regardless of ratio.
  
  ## Validation
  
  - New test alternates two rectangles past the budget and asserts the
  frame stops growing. Verified against a reverted fix: without the charge
  it reaches 128 entries, with it 64.
  - `cargo xtask check fmt/lints/tests/typos/locks` all pass.
  
  ## Notes
  
  - The charge counts logical entries, not `Vec` capacity. Since `Vec`
  grows by doubling, resident bytes for `frame` can reach roughly twice
  the charged figure. This matches the existing accounting, which charges
  `data.len()` for pixel buffers rather than capacity; flagging it rather
  than diverging from the established model.
  - Refusing the charge drops the dirty region, so a starved client can
  show stale pixels. That is the contract `materialize` already follows
  for refused allocations, and `charge` logs the allocated and budget
  figures.
  - Reported by Copilot on #1462. Follows #1460, which introduced the
  budget and the deferred materialization.

- Advertise only capability sets the client can decode ([#1564](https://github.com/Devolutions/IronRDP/issues/1564)) ([b7657bcb67](https://github.com/Devolutions/IronRDP/commit/b7657bcb670b080c8cd19a9dff0078387cb8c478)) 

  ## Summary



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.2.0...ironrdp-egfx-v0.3.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.8

- [**breaking**] Update `ironrdp-graphics` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.1.0...ironrdp-egfx-v0.2.0)] - 2026-06-05

### <!-- 1 -->Features

- [**breaking**] Surface total_frames_decoded on the frame-ack callback ([#1345](https://github.com/Devolutions/IronRDP/issues/1345)) ([cf51bdd1d5](https://github.com/Devolutions/IronRDP/commit/cf51bdd1d5ba062132039f5ed6d7871e00af6412)) 

- Cascade Arbitrary derives across ironrdp-egfx public PDU types ([#1334](https://github.com/Devolutions/IronRDP/issues/1334)) ([479a13aa49](https://github.com/Devolutions/IronRDP/commit/479a13aa49478e333ccdc4c8fdf03aa4f36d2cac)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Make DecodedFrame fields private with getters to enforce size invariant ([#1331](https://github.com/Devolutions/IronRDP/issues/1331)) ([1534d1b40e](https://github.com/Devolutions/IronRDP/commit/1534d1b40e902a404b020fbae8e970a65ca74458)) 



## [0.1.0] - 2026-06-01

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
