# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.2.0...ironrdp-core-v0.2.1)] - 2026-07-02

### <!-- 4 -->Bug Fixes

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.

### <!-- 5 -->Performance

- Replace softbuffer with direct put_image_data canvas present ([#1374](https://github.com/Devolutions/IronRDP/issues/1374)) ([d3705af18c](https://github.com/Devolutions/IronRDP/commit/d3705af18cff1851f4d48017affcb85aaa678d57)) 

  ## Summary
  
  The web client presented frames through `softbuffer`, whose web backend
  repacks
  the **whole surface** (RGBA → u32 → RGBA into a fresh buffer) on every
  present.
  This replaces it with a direct `put_image_data` that uploads only the
  dirty
  region, and drops the `softbuffer` dependency.
  
  Same idea as the IronVNC change.
  
  ## What changed
  
  - Remove the `softbuffer` dependency; present each dirty region with
    `put_image_data` at its origin.
  - No full-surface buffer and no per-region scratch.
  `extract_partial_image` fills
  a single `WriteBuf` reused across frames, so steady-state draws don't
  allocate.
  - Force opaque alpha before upload (kept — see Correctness).
  - Add `WriteBuf::filled_mut` to `ironrdp-core` (mutable counterpart of
  `filled`).
  - `web-sys`: add `CanvasRenderingContext2d` + `ImageData`, drop the
  softbuffer-only
    features.
  
  ## Performance
  
  Draw-stage time on a 1080p replay (595 frames / 110 dirty regions),
  headless
  Chromium, 8 measured passes × 3 runs, median. Both rows are reproducible
  branches
  off the replay-bench harness; the only difference is the render path.
  
  | Render path | draw (ms) | vs softbuffer | branch |
  |---|--:|--:|---|
  | softbuffer `present_with_damage` | ~1031 | — | `bench/draw-softbuffer`
  |
  | this PR (direct upload, reused `WriteBuf`) | ~97 | **~10.6×** |
  `bench/draw-zerocopy` |
  
  - The win is structural: upload the dirty region instead of repacking
  the whole
    surface every present.
  - Reusing one `WriteBuf` (vs a per-frame allocation) keeps the
  steady-state draw
  allocation-free; the remaining cost is the unavoidable `ImageData` JS
  copy.
  - Output is **byte-identical**: framebuffer CRC32 `2d8e1b79` matches the
  recorded
    ground truth and the rendered-canvas FNV-1a is unchanged.
  - Absolute ms carry ~±15% noise from machine load (decode drifted
  1.5–1.9 s); the
    ratio held across runs.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.5...ironrdp-core-v0.2.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-error` public dependency to 0.2

## [[0.1.5](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.4...ironrdp-core-v0.1.5)] - 2025-05-28

### Features

- Adds `write_padding` and `read_padding` functions/macros extracted from `ironrdp-pdu` crate

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.3...ironrdp-core-v0.1.4)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.2...ironrdp-core-v0.1.3)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-core-v0.1.1...ironrdp-core-v0.1.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
