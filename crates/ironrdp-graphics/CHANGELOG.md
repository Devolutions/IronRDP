# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.9.0...ironrdp-graphics-v0.10.0)] - 2026-08-07

### <!-- 1 -->Features

- Add ClearCodec client-side decode dispatch ([#1175](https://github.com/Devolutions/IronRDP/issues/1175)) ([714dce4662](https://github.com/Devolutions/IronRDP/commit/714dce46627e299c57d82f4f6a5c18067a95bffa)) 

  Follow-up to #1174. Supersedes #1195 (the standalone server-helper PR;
  its 46-line `send_clearcodec_frame()` is included here).
  
  Wires ClearCodec into the EGFX client's WireToSurface1 codec dispatch,
  matching the existing AVC420 and Uncompressed decode patterns.

### <!-- 4 -->Bug Fixes

- Correct progressive base quantization scale ([#1499](https://github.com/Devolutions/IronRDP/issues/1499)) ([ccfe5bb8b3](https://github.com/Devolutions/IronRDP/commit/ccfe5bb8b3b1ddf447776e056d27cd14e2399ab7)) 

  ## Summary

- Decode indexed pointers and foreground RLE runs ([#1519](https://github.com/Devolutions/IronRDP/issues/1519)) ([ad19280762](https://github.com/Devolutions/IronRDP/commit/ad192807620bcc3a0467eaeb07173a79cb1da257)) 

  ## Summary
  
  4bpp and 8bpp New/Large pointer shapes previously could not use the
  active session palette, and malformed or unsupported pointer data could
  terminate the session. This decodes indexed XOR masks with the current
  palette and falls back to the default cursor while evicting stale cached
  data when decoding fails.
  
  It also corrects RLE foreground runs so only set-foreground variants
  consume a foreground pixel.
  
  Palette updates now follow the RDP wire format (type, padding, 256
  packed RGB triplets) and are applied from both fast- and slow-path
  updates, ensuring indexed pointer decoding uses the negotiated palette.
  
  ## Tests
  
  - `cargo test -p ironrdp-graphics -p ironrdp-session`
  - `cargo test -p ironrdp-session palette`
  - `cargo clippy -p ironrdp-graphics -p ironrdp-session --all-targets --
  -D warnings`
  
  ---------

- Rename {Read,Write}Cursor::rewinded into rewound ([#1529](https://github.com/Devolutions/IronRDP/issues/1529)) ([c85b089b46](https://github.com/Devolutions/IronRDP/commit/c85b089b4617176240b41482be65a77c9ad76a07)) 

- Correct three RLGR encoder bugs per MS-RDPRFX spec ([#1179](https://github.com/Devolutions/IronRDP/issues/1179)) ([f26d06da6f](https://github.com/Devolutions/IronRDP/commit/f26d06da6f16bfb7fa57f8d4f67658b33c01bc01)) 

  ## Summary
  
  Fixes three bugs in the RLGR entropy encoder
  (`ironrdp-graphics::rlgr::encode`) identified by cross-referencing the
  MS-RDPRFX §3.1.8.1.7.3 pseudocode and the FreeRDP reference
  implementation (`rfx_rlgr.c`).
  
  - **RL mode trailing value**: the encoder skipped emitting sign bit + GR
  code when input was exhausted after a zero run. The decoder
  unconditionally reads these bits, so the encoder must always emit them.
  Restructured to match FreeRDP's `GetNextInput` pattern.
  - **RLGR3 exhausted second value**: used `unwrap_or(1)` as default
  `twoMs` when the second value in a GR-mode pair was unavailable.
  FreeRDP's `GetNextInput` returns 0 when exhausted, and `Get2MagSign(0) =
  0`, so the correct default is 0.
  - **RLGR1 GR-mode kp update**: used `UP_GR` (4, the RL-mode constant)
  instead of `UQ_GR` (3, the GR-mode constant) when updating `kp` for zero
  symbols. Both the spec pseudocode and FreeRDP use `UQ_GR` here.
  
  Each fix is in a separate commit with full rationale and spec/FreeRDP
  references.
  
  ### Spec errata note
  
  The MS-RDPRFX §4.2.4.1 reference hex dump appears to have been generated
  with `UQ_GR=4` and `twoMs2=1` defaults — inconsistent with the normative
  pseudocode in §3.1.8.1.7.3. Our encoder follows the pseudocode (which is
  authoritative over example data) and matches FreeRDP. The Y test vector
  is updated accordingly (2 bytes differ from the spec hex dump at indices
  939–940). Encoder→decoder roundtrip is verified correct.
  
  ## Test plan
  
  - [x] All 12 existing RLGR unit tests pass (encode + decode, small
  vectors + full 4096-coefficient Y/Cb/Cr datasets)
  - [x] Encoder→decoder roundtrip verified for Y, Cb, Cr components
  - [x] `cargo clippy -p ironrdp-graphics` clean
  - [x] `cargo xtask check lints -v` passes
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- [**breaking**] Do not panic when the RLGR output buffer is too small ([#1558](https://github.com/Devolutions/IronRDP/issues/1558)) ([71903c5509](https://github.com/Devolutions/IronRDP/commit/71903c550929dbbd1b1b881cc403a6a693b6e233)) 

  ## Summary
  
  - `BitStream::output_bit` and `output_bits` indexed the output
  `BitSlice` with no capacity check, so a tile whose entropy coding didn't
  fit the caller's buffer panicked inside a function that already returns
  `Result`.
  - The RLGR decoder a few hundred lines down has been bounds-checked all
  along: `try_split_bits!` breaks when bits run short, the loop guards on
  `!bits.is_empty() && !output.is_empty()`, and run-length fills clamp
  with `min(run, output.len())`. Only the encoder was unguarded.
  - The writers now reserve before writing and record an overflow rather
  than indexing past the end. `encode` turns that into
  `RlgrError::OutputTooSmall`.
  
  ## Why it's reachable
  
  RLGR gives no compression guarantee, but callers size the output as
  though it did. `ironrdp-server` splits a 12288-byte tile buffer into
  three 4096-byte components, which is 2:1 against 4096 `i16`
  coefficients. That holds comfortably at the default quantization table
  and stops holding as the table gets lighter, so this is reachable from
  configuration rather than from malformed input.
  
  The tile is still walked to completion after an overflow, so the error
  reports the size the tile would have needed. A caller sizing on a ratio
  needs that number to correct the ratio; it's the reason the variant
  carries data.
  
  ## Retrying at the size the encoder asks for
  
  Detection alone was not a fix, which @mamoreau-devolutions was right to
  push back on. `alloc_data` gave every component exactly 4096 bytes, and
  the loop in `encoder/mod.rs` that grows a buffer on `NotEnoughBytes`
  grows the whole-frame one, so it could never have helped here. A tile
  that overflowed went from panicking to failing the update.
  
  The per-component reserve is a parameter now, and the tile-set encode
  retries at exactly the size the encoder reports, growing monotonically
  and stopping at twice a component's raw `i16` size.
  
  I did not take the fixed upper bound offered as an alternative. RLGR is
  adaptive and unary-dominated in its worst case, so a bound loose enough
  to be provable is megabytes per component, and anything small enough to
  allocate is a guess. Since the encoder can say exactly what it needs,
  retrying at that seemed better than inventing a constant.
  
  The retry lives in `RfxEncoder::encode` because `rfx::Tile` borrows out
  of the buffer, so the buffer has to be sized before any borrow is taken.
  Widening `Tile` would reshape a wire PDU type, and a per-tile fallback
  needs the overflow buffers to outlive a `par_chunks_mut` borrow.
  
  `OutputTooSmall` is deliberately not mapped onto `NotEnoughBytes` to
  reuse that existing loop, since the loop grows a different buffer and
  would look like a fix without being one.
  
  ## Breaking change
  
  `RlgrError` gains an `OutputTooSmall` variant and isn't
  `#[non_exhaustive]`, so exhaustive matches need updating.
  
  `ironrdp-graphics` is marked `# public` in `ironrdp-server`,
  `ironrdp-client`, `ironrdp-egfx`, `ironrdp-session`, and
  `ironrdp-nscodec` (under its `encoder` feature), so the bump cascades to
  those.
  
  I considered reusing
  `RlgrError::Io(io::Error::from(ErrorKind::WriteZero))` to avoid the
  break. It discards both numbers the caller needs to act on, which
  defeats the point. Marking the enum `#[non_exhaustive]` is a separate
  breaking change and a wider policy question, so it isn't bundled here.
  
  ## Validation
  
  - `cargo xtask check fmt/lints/tests/typos/locks` all pass on the pinned
  toolchain, `fuzz/` built before the lock check.
  - Two tests added in
  `crates/ironrdp-testsuite-core/tests/graphics/rlgr.rs`: an undersized
  buffer reports `OutputTooSmall` with `needed > available` for both RLGR1
  and RLGR3, and a buffer of exactly the required size still succeeds. The
  twelve existing `graphics::rlgr` tests are unchanged and still pass.
  - Two more in `crates/ironrdp-testsuite-core/tests/server/rfx.rs` for
  the retry: the reported size must be sufficient rather than merely
  indicative, so encoding at a 64-byte reserve and retrying at exactly the
  size reported has to succeed; and the default reserve still handles a
  full-entropy tile, so the retry stays a fallback. Under-reporting the
  size by one byte fails the first of those.
  - Those two started life inline in `encoder/rfx.rs`, where they never
  ran: `ironrdp-server` sets `[lib] test = false`, so `cargo test
  --workspace` builds no unittest binary for it. They reach the encoder
  through the crate's existing `__bench` feature, which now also exports
  `rfx_enc_at` next to the two helpers already there, so no type had to be
  widened.
  - Driving the server pipeline (BGRA, `to_64x64_ycbcr_tile`,
  `dwt::encode`, `quantization::encode`, `rlgr::encode` into 4096 bytes)
  on a random-noise tile, a quantization table of all ones panics on
  master and now returns `encoded tile needs 6018 bytes, output buffer is
  4096`.
  - Spec-legal tables are unaffected, and that is measured rather than
  assumed. Binary-searching the minimum per-component reserve for a set of
  adversarial 64x64 tiles at `Quant::default()` gives a worst case of 2857
  bytes of the 4096 available (per-channel independent noise at full
  swing, RLGR3); full-range noise needs 2379 under RLGR1. Taking that
  worst pattern across the whole legal range from [MS-RDPRFX] 2.2.2.1.5,
  RLGR1 needs 3768 bytes at quant 6, 2740 at 8, and 917 at 15. Nothing in
  spec overflows, which is why this PR fixes the panic and stops there.
  
  ## Notes
  
  Found while looking at #1557, where an all-ones quantization table
  panicked the encoder. That table is out of spec ([MS-RDPRFX] 2.2.2.1.5
  requires 6 to 15), and the configurability that issue asks for is
  separate work. This change is only about not panicking.
  
  Touches `BitStream` and the tail of `encode`, deliberately staying clear
  of the body of `encode` where #1370 and #1179 both have hunks. It should
  rebase cleanly whichever of those lands first.

- Don't emit a value after a run that ends the input ([#1569](https://github.com/Devolutions/IronRDP/issues/1569)) ([d9d2896c8c](https://github.com/Devolutions/IronRDP/commit/d9d2896c8cbc06cf1c7b82f45b19482d0633dcb2)) 

  RL mode codes the following value's magnitude minus one, so it cannot
  express
  zero. #1179 began coding a zero there when the run consumed the rest of
  the
  input, and the decoder reconstructs magnitude as the coded value plus
  one, so
  every input ending in a zero run gained a trailing 1.
  
  That is what fails
  `progressive_fractional_base_quantization_reconstructs_rgb`
  from #1499 on master: quantized coefficients end in a zero run, so the
  phantom
  value lands in HH1 of each component.
  
  The trailing zeros are the run and nothing follows them. #1179's other
  two
  fixes are untouched.
  
  2000 randomized round trips per entropy mode, 0 failures. Workspace
  suite green.

### <!-- 5 -->Performance

- Portable SIMD inverse DWT (wide + SWAR) ([#1383](https://github.com/Devolutions/IronRDP/issues/1383)) ([629154026d](https://github.com/Devolutions/IronRDP/commit/629154026de0eaaf16b93352b4cecbae49a87511)) 

  ## Summary
  
  On the WASM web client, frame **decode** dominates (~93% of frame time
  on a 1080p RemoteFX replay), and within decode the **RFX inverse DWT was
  ~48%** (the YCbCr→RGBA convert is already SIMD via `yuv`; the
  entropy/RLE stages are inherently sequential). This vectorizes the
  inverse DWT with the portable [`wide`](https://crates.io/crates/wide)
  crate (`i16x8`), so the same code lowers to **wasm `simd128`, x86
  SSE/AVX, and ARM NEON** — desktop and browser both benefit.
  
  The encode path is unchanged.
  
  ## How it stays bit-exact (no `unsafe`, no `cfg` split)
  
  The lifting steps need i32 intermediates only for the averages.
  Overflow-free SWAR identities let the whole kernel stay in `i16` lanes
  (no widen/narrow):
  
  - `ceil_avg(a,b)  = (a|b) - ((a^b)>>1)`  ≡ `(a + b + 1) >> 1`
  - `floor_avg(a,b) = (a&b) + ((a^b)>>1)`  ≡ `(a + b) >> 1`
  
  and `(2x+1)>>1 == x` / `(x+x)>>1 == x` simplify the first/last rows.
  Every other op is wrapping `i16` arithmetic, identical to the old
  `i32`-intermediate-then-`as i16` truncation.
  
  ## Performance
  
  1080p RemoteFX replay, headless Chromium, wasm release `+simd128`,
  8-pass median:
  
  | inverse DWT | decode (ms) |
  |---|--:|
  | scalar (baseline) | ~1598 |
  | **portable `wide` SIMD** | **~985** |
  
  → inverse DWT ~2×, **~39% off the decode stage**. (Absolute ms carry
  ~±15% machine-load noise; the ratio is stable. Per-frame this is a
  throughput win — decode was already within real-time budget.)
  
  ## Correctness
  
  Verified bit-exact three ways:
  - the replay-bench **framebuffer CRC32** is unchanged,
  - the existing **native DWT tests** pass (so it's exact on x86 too, not
  just wasm),
  - an **exhaustive** check of the SWAR identities over all `i16 × i16`
  pairs (0 mismatches).
  
  ## Notes
  
  - `wide` is a single-user dep in `ironrdp-graphics`; chosen over
  `std::simd` (still nightly-only) and over per-arch intrinsics (one
  portable kernel vs three).
  - Reproducible bench branches: `bench/draw-*` (renderer) and the DWT
  measurements were taken on the replay-bench harness branch (the capture
  corpus is gitignored).



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.8.1...ironrdp-graphics-v0.9.0)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Don't require CONTEXT block on every progressive frame ([#1395](https://github.com/Devolutions/IronRDP/issues/1395)) ([368fe8e68b](https://github.com/Devolutions/IronRDP/commit/368fe8e68b2d5d72da2e15dcf99469b98e965a2b)) 

  Fixes progressive RemoteFX (MS-RDPEGFX) decoding by no longer requiring a CONTEXT block on every WireToSurface2 progressive frame once a codec context has already been established (keyed by codec_context_id). This aligns the decoder with real-world server behavior and the spec’s “establish once, then reference” model for progressive contexts.

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.8.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.8.0...ironrdp-graphics-v0.8.1)] - 2026-06-05

### <!-- 4 -->Bug Fixes

- Bound ZGFX compressor hash table size ([#1344](https://github.com/Devolutions/IronRDP/issues/1344)) ([4e11a17617](https://github.com/Devolutions/IronRDP/commit/4e11a1761750bb706f5c3cef370589d0eb63fc45)) 

  Bounds the ZGFX compressor's hash table to prevent O(n·table_size) per-frame compaction on incompressible payloads (e.g., already-encoded H.264). Previously, `compact_hash_table` only halved per-prefix position lists without reducing prefix count, so high-entropy input kept the table above the cap and triggered compaction on every literal byte. The fix evicts whole least-recently-seen prefixes down to a low watermark (half the cap), amortizing compaction to O(1) per byte while preserving reachable matches (distance is already capped at `MAX_MATCH_DISTANCE`).



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.7.0...ironrdp-graphics-v0.8.0)] - 2026-05-27

### <!-- 1 -->Features

- Add segment wrapping utilities ([#1076](https://github.com/Devolutions/IronRDP/issues/1076)) ([5fa4964807](https://github.com/Devolutions/IronRDP/commit/5fa4964807fa15bbf1a5e3c23b365344758961aa)) 

  Adds ZGFX segment wrapping utilities for encoding data in RDP8 format.

- Add LZ77 compression support ([#1097](https://github.com/Devolutions/IronRDP/issues/1097)) ([48715483a3](https://github.com/Devolutions/IronRDP/commit/48715483a36c824af034a51f4db0580c34825d63)) 

  Adds ZGFX (RDP8) LZ77 compression to complement the existing
  decompressor, plus a high-level API for EGFX PDU preparation with
  auto/always/never mode selection.
  
  The compressor uses a hash table mapping 3-byte prefixes to history
  positions for O(1) match candidate lookup against the 2.5 MB sliding
  window.

- Complete pixel format support for bitmap updates ([#1134](https://github.com/Devolutions/IronRDP/issues/1134)) ([a6b41093ce](https://github.com/Devolutions/IronRDP/commit/a6b41093ce4ece081d2538c157f6bc547c3b2607)) 

  Wires missing bitmap pixel formats (8/15/24bpp) into the session rendering
  pipeline so bitmap updates at those depths are rendered instead of being
  dropped, and adds fast-path palette update parsing to support 8bpp indexed
  color sessions.

- Add RemoteFX Progressive codec primitives ([#1196](https://github.com/Devolutions/IronRDP/issues/1196)) ([49099f0c31](https://github.com/Devolutions/IronRDP/commit/49099f0c3136c25b67801fb1b07f78542dc796de)) 

  Add wire-format types for RemoteFX Progressive Codec (MS-RDPRFX
  Progressive Extension) and the computational primitives required for progressive refinement.

- Add progressive RFX decode and EGFX integration ([#1197](https://github.com/Devolutions/IronRDP/issues/1197)) ([a142799d1d](https://github.com/Devolutions/IronRDP/commit/a142799d1dcbdcd6546ec6e75173fbfe66f0ea67)) 

- Add progressive RFX server encode and mixed-codec frames ([#1198](https://github.com/Devolutions/IronRDP/issues/1198)) ([6d43d2692d](https://github.com/Devolutions/IronRDP/commit/6d43d2692d206b7557f722f294d3e51d7eac8ab1)) 

- Add ClearCodec bitmap compression codec ([#1174](https://github.com/Devolutions/IronRDP/issues/1174)) ([059ca902a5](https://github.com/Devolutions/IronRDP/commit/059ca902a5518113163042225bc5d2088869933a)) 

### <!-- 4 -->Bug Fixes

- Fix pixel format handling in bitmap decoders ([#1101](https://github.com/Devolutions/IronRDP/issues/1101)) ([75863245ab](https://github.com/Devolutions/IronRDP/commit/75863245ab376f15e35c00df434860c93b123633)) 

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.6.0...ironrdp-graphics-v0.7.0)] - 2025-12-18

### Added

- [**breaking**] `InvalidIntegralConversion` variant in `RlgrError` and `ZgfxError`

### <!-- 7 -->Build

- Bump bytemuck from 1.23.2 to 1.24.0 ([#1008](https://github.com/Devolutions/IronRDP/issues/1008)) ([a24a1fa9e8](https://github.com/Devolutions/IronRDP/commit/a24a1fa9e8f1898b2fcdd41d87660ab9e38f89ed)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.5.0...ironrdp-graphics-v0.6.0)] - 2025-06-27

### <!-- 4 -->Bug Fixes

- `to_64x64_ycbcr_tile` now returns a `Result`

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.4.0...ironrdp-graphics-v0.4.1)] - 2025-06-27

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 3 updates (#816) ([5c5f441bdd](https://github.com/Devolutions/IronRDP/commit/5c5f441bdd514d3fe6a29b4df872709167a9916d)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.3.0...ironrdp-graphics-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- Add helper to find diff between images ([20581bb6f1](https://github.com/Devolutions/IronRDP/commit/20581bb6f12561e22031ce0e233daeada836ea67)) 

  Add some helper to find "damaged" regions, as 64x64 tiles.

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.2.0...ironrdp-graphics-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.2...ironrdp-graphics-v0.2.0)] - 2025-03-07

### Performance

- Replace hand-coded yuv/rgb with yuvutils ([5f1c44027a](https://github.com/Devolutions/IronRDP/commit/5f1c44027a7f6da5271565461764dd3f61729ee4)) 

  cargo bench:
  to_ycbcr                time:   [2.2988 µs 2.3251 µs 2.3517 µs]
                          change: [-83.643% -83.534% -83.421%] (p = 0.00 < 0.05)
                          Performance has improved.

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.1...ironrdp-graphics-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.0...ironrdp-graphics-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
