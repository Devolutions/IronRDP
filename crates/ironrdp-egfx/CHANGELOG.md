# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.3.0...ironrdp-egfx-v0.4.0)] - 2026-08-29

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

- Decode RFX Progressive tiles ([#1673](https://github.com/Devolutions/IronRDP/issues/1673)) ([f21f1979f4](https://github.com/Devolutions/IronRDP/commit/f21f1979f4ef7ce8756a4022d866c7f7fc150b0b)) 

  Keep Progressive tile state scoped to its surface and codec context.
  
  Reject context-less updates that have no state for their surface, and
  expose targeted context and surface cleanup for consumers.

- [**breaking**] Populate decode/encode error offsets from cursor positions ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec)) 

  ## Summary
  
  The workspace sweep that follows #1266. Decode and encode error
  construction sites now pass the cursor, so the reported position is the
  byte the decoder or encoder actually stopped at.
  
  Stacked on #1266 and merges after it.
  
  ## What "no position" means here
  
  #1266 makes `offset` an `Option<usize>` where `None` means the error has
  no position in the input stream at all, rather than a position that
  happened to be unavailable. This PR is the other half of that: it walks
  the workspace and gives a real position to every site that has one, so
  the sites left reporting `None` are the ones that genuinely never had
  one.
  
  Those are constructors validating their arguments, integer conversions,
  cache lookups that missed, accessors on already-decoded structures, and
  the declared-size checks described below. They report nothing rather
  than byte zero, and that is now their permanent answer rather than a gap
  awaiting another sweep.
  
  There are no `at: 0` sites left anywhere in the workspace.
  
  ## The rule
  
  The position is attached where the cursor identifies the bytes being
  complained about. It is omitted where the complaint is about a size the
  peer declared, computed from data already consumed, because there the
  cursor points at a byte that is not the problem.

- Render the EGFX graphics pipeline output ([#1461](https://github.com/Devolutions/IronRDP/issues/1461)) ([d414622231](https://github.com/Devolutions/IronRDP/commit/d414622231ed3a944d8896118f409edead1d3df3)) 

  ## Summary
  
  - The `ironrdp-egfx` compositor exposes changed output regions via
  `drain_output()`, but nothing consumed them, so an EGFX session decoded
  frames and dropped them.
  - Drains the compositor from `ActiveStage::process`: composites each
  completed-frame `OutputUpdate` into the `DecodedImage` and emits
  `ActiveStageOutput::GraphicsUpdate`, so every session consumer renders
  EGFX with no new code. No-op when the graphics DVC is not registered.
  - Adds a by-type mutable DVC accessor `DrdynvcClient::get_dvc_mut`,
  mirrored on the session (the x224 processor and `ActiveStage`), matching
  the existing `get_dvc`.
  - All regions drained in one pass are composited first by
  `composite_graphics_updates`, then surfaced as a single `GraphicsUpdate`
  covering their union. A consumer may redraw whatever region an update
  names, and `ironrdp-client` rebuilds the whole framebuffer for each one,
  so emitting per region would copy the desktop once per rectangle. A
  single `RDPGFX_SOLIDFILL_PDU` or `RDPGFX_CACHE_TO_SURFACE_PDU` can name
  up to `u16::MAX` of them.
  
  ## Validation
  
  - `cargo xtask check fmt/lints/tests/typos/locks` all pass, including
  the dependency guard.
  - Four tests cover the coalescing: two disjoint deltas collapsing to the
  rectangle spanning both, 64 deltas yielding exactly one region, a single
  delta passing through unwidened, and an empty drain surfacing nothing.
  Checked against a reverted coalescing, where the first two fail.
  - The loop lives in `composite_graphics_updates` rather than inline in
  `ActiveStage::process` so it can be tested without standing up an x224
  processor. It takes `(ExclusiveRectangle, Vec<u8>)` rather than
  `OutputUpdate`, since that type is `#[non_exhaustive]` and cannot be
  constructed from `ironrdp-session`; this keeps the change inside the
  crate instead of widening an already-merged API for testability.
  
  ## Notes
  
  - `ironrdp-session` already depends on `ironrdp-displaycontrol` and
  `ironrdp-graphics`; `ironrdp-egfx` is consumed the same way. The guard
  keeping `ironrdp-connector` and `sspi` out of `ironrdp-session` still
  passes.
  - Reuses `apply_rgba32` (previously gated behind the `qoi` feature);
  converts the compositor's exclusive-rectangle regions to the session's
  inclusive-rectangle convention.
  - #1377 and #1460 have both merged, so this now sits directly on master
  and the diff is its own change alone: 6 files, +160/-6.
  - #1462 is stacked on this one, so the merge order is this then #1462.
  - Part of #1464. Motivated by #1446.

- Clip Progressive updates to REGION and decode Windows streams ([#1443](https://github.com/Devolutions/IronRDP/issues/1443)) ([11b10531de](https://github.com/Devolutions/IronRDP/commit/11b10531de491c67f5c8d6bb6972f6951bbeecc8)) 

  ## Problem
  
  #1673 landed the core `WireToSurface2` → `ProgressiveDecoder` dispatch
  (originally proposed here), and #1696/#1698 extended the decoder past
  this PR's scope. What remains splits in two.
  
  **REGION handling.** Master decodes REGION blocks wherever they appear,
  including outside `FRAME_BEGIN`/`FRAME_END` — [CBenoit's
  review](https://github.com/Devolutions/IronRDP/pull/1443#issuecomment-5164007525)
  asked for those to be ignored, and that part is still open. Decoded
  tiles are also blitted whole (64x64) with the REGION rectangles ignored,
  so pixels outside the damage region the server reported are written to
  the surface anyway.
  
  **Progressive from a Windows host does not decode at all.** Three
  separate defects, each hit in order while replaying a live session
  against Windows (details in the comment below).
  
  ## Changes
  
  REGION handling:
  
  - `decode_bitmap` processes REGION blocks only inside the first
  `FRAME_BEGIN`/`FRAME_END` pair and ignores blocks outside it.
  - Each `DecodedTile` carries `update_rectangles`: the tile clipped to
  the REGION rectangles and surface bounds. The client emits one
  `BitmapUpdate` per rectangle, keeping the whole-tile fast path.
  - `begin_frame`/`end_frame` (driven by StartFrame/EndFrame) let REGION
  blocks split across several `WireToSurface2` payloads of one frame
  reference shared tiles.
  - Clipping work is budgeted per payload so a hostile REGION cannot drive
  quadratic `union_rectangle`/`intersect_rectangle` scans.
  
  Windows compatibility:
  
  - `quality == 0xFF` selects full quality per [MS-RDPEGFX] 2.2.4.2.1.5.2
  instead of indexing `quantProgVals`; Windows sends it with an empty
  table, so indexing rejected every tile.
  - `ResetGraphics` no longer drops codec contexts. It only resizes the
  Graphics Output Buffer, and Windows never repeats SYNC + CONTEXT after
  one.
  - A new codec context id that arrives without SYNC + CONTEXT inherits
  the band layout retained for its surface, released when the surface is
  deleted. Deleting a context still drops its tiles.
  
  ## Testing
  
  `decoder_ignores_regions_outside_frame`,
  `decoder_bounds_region_clipping_work`,
  `full_quality_tiles_bypass_the_progressive_quant_table`,
  `progressive_context_survives_graphics_reset`,
  `wire_to_surface2_clips_compositor_output_to_region`, plus the reworked
  context-deletion assertions.
  
  `ironrdp-graphics` 235 and `ironrdp-egfx` 47 pass; `cargo clippy
  --all-targets -- -D warnings` and `cargo fmt` clean.
  
  ---------

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

- Compose scaled output surfaces ([#1699](https://github.com/Devolutions/IronRDP/issues/1699)) ([96e388eef2](https://github.com/Devolutions/IronRDP/commit/96e388eef27c121b5590da7a1a3fdfb1d47d11e9)) 

  Implement client-side composition for MapSurfaceToScaledOutput.
  
  Track scaled mappings in the EGFX compositor, materialize scaled dirty
  output at EndFrame, and preserve bounded output clipping and
  frame-atomic updates.
  
  Add client and compositor coverage for dispatch, nearest-neighbor
  pixels, dirty bounds, persistent output, and oversized wire-scale
  factors.

### <!-- 5 -->Performance

- Allocate the ClearCodec decoder lazily on first use ([#1738](https://github.com/Devolutions/IronRDP/issues/1738)) ([01075338e5](https://github.com/Devolutions/IronRDP/commit/01075338e5c23087b55e7673b84b18d7081b01ba)) 

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
