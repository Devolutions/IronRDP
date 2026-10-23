# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.3.0...ironrdp-egfx-v0.4.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

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

- Add diagnostic logging for EGFX flow control and dispatch timing ([#1834](https://github.com/Devolutions/IronRDP/issues/1834)) ([8d1e91eb32](https://github.com/Devolutions/IronRDP/commit/8d1e91eb3256e0ba007e909512bbaf84cb76d67b)) 

  I ran into a case where I needed to debug slow frame acknowledgement and
  dispatch stalls in ironrdp-server and ironrdp-egfx, and found the
  relevant signals were either missing or buried at TRACE level where
  nobody enables them in normal operation. This PR adds targeted
  diagnostic logging without changing any behavior.
  
  - FrameTracker now edge-triggers a debug log when backpressure or
  ack_suspended change state, instead of logging every call (which would
  flood at 30+/sec) or nothing at all.
  - FrameAcknowledge is now logged at DEBUG with latency, queue_depth, and
  in-flight count. An ack for an unknown frame_id (protocol violation or
  stale ack per MS-RDPEGFX 2.2.4.3) is now a warning instead of silent.
  - drain_output (ZGFX compression) now tracks per-batch compress time and
  ratio, logging at INFO when a batch exceeds a 10ms budget and DEBUG
  otherwise, since it runs under both the state lock and the writer lock
  and can block inbound PDU processing.
  - Incoming EGFX DVC PDUs are logged at DEBUG with their kind, so the
  client-to-server side of the channel is visible without enabling TRACE.
  - dispatch_pdu and dispatch_events now separately time lock-acquisition
  wait and handler dispatch, warning when either exceeds 50ms so the two
  causes (lock contention vs. handler/runtime stall) can be told apart
  instead of both surfacing as one generic slow-dispatch symptom.
  
  No public API changes, no behavioral changes, logging only.
  
  Note on CI: the workspace-wide test compile is currently broken on
  master independent of this PR, ironrdp-daemon's consume_output() call
  site passes a raw Receiver where OutputEventReceiver is expected. I
  confirmed this reproduces on a clean, unmodified checkout of master.
  This PR only touches ironrdp-server and ironrdp-egfx.

- Handle mid-session CapsAdvertise as decoder-recovery ([#1833](https://github.com/Devolutions/IronRDP/issues/1833)) ([b4ba2c2237](https://github.com/Devolutions/IronRDP/commit/b4ba2c2237a2269681256e82fc1797d22d6fe0e7)) 

  Real-world clients (mstsc on Windows 11 under load, macOS Microsoft
  Remote Desktop) re-emit RDPGFX_CAPSADVERTISE mid-session as a
  decoder-recovery sequence when their decoder loses sync. The previous
  handler treated every CapsAdvertise as initial setup, leaving
  reset_graphics_sent=true so the application's follow-up create_surface
  wouldn't auto-emit ResetGraphics, breaking recovery.
  
  I detect the re-advertise via state == Ready at entry, then silently
  clear surfaces + frames and re-arm reset_graphics_sent. I don't emit a
  DeleteSurface PDU for this: the client has already cleared its surface
  state on its end, and treats a stray DeleteSurface as a protocol
  violation, closing the connection within milliseconds.
  
  Surfaces gains reset_for_reinit(), distinct from clear(): it also resets
  next_surface_id to 0 so the client's subsequent CreateSurface yields ID
  0, matching its fresh-state expectation.
  
  MS-RDPEGFX does not document this recovery flow explicitly, but the
  empirical pattern is CapsAdvertise -> CacheImportOffer -> the server is
  expected to emit CapsConfirm + ResetGraphics + CreateSurface(id=0) +
  MapSurfaceToOutput + IDR.
  
  I validated this against mstsc on Windows 11 and openSUSE Tumbleweed
  (KDE Plasma 6.6.4).

- Add H264Encoder trait and an openh264 reference implementation ([#1852](https://github.com/Devolutions/IronRDP/issues/1852)) ([9609c9e93b](https://github.com/Devolutions/IronRDP/commit/9609c9e93b7e33d64e9daab3326dd4caa0407532)) 

  ironrdp-egfx has had the decode half of an H.264 abstraction since the
  openh264 decoder landed: an H264Decoder trait plus a feature-gated
  reference implementation. The encode half never existed, so every server
  producing AVC420 frames brings its own encoder stack and its own answer
  to the format questions. This adds the symmetric twin.
  
  encode.rs mirrors decode.rs piece for piece: an EncodeFrame
  borrowed-RGBA input (the same pixel layout DecodedFrame produces on the
  other side), an H264Encoder trait with defaulted request_key_frame and
  reset hooks, an EncoderError shaped like DecoderError, and an
  OpenH264Encoder reference implementation behind the existing openh264 /
  openh264-bundled / openh264-libloading features with the same two
  construction paths and the same patent-posture notes as the decoder.
  
  The output contract is documented deliberately: an ITU-T H.264 Annex B
  bitstream with in-band SPS/PPS, which is what RFX_AVC420_BITMAP_STREAM
  carries per [MS-RDPEGFX] 2.2.4.4 and what send_avc420_frame forwards
  unmodified. OpenH264 emits Annex B natively, so the reference
  implementation does no conversion. The ecosystem precedent for the trait
  shape is FreeRDP's H264_CONTEXT_SUBSYSTEM vtable, which registers
  OpenH264, Media Foundation, FFmpeg and MediaCodec backends behind one
  seam; downstream hardware encoders (VA-API, NVENC, VideoToolbox) get the
  same injection point here.
  
  No server rewiring is included: send_avc420_frame keeps taking
  pre-encoded bytes, and nothing changes for existing callers. The trait
  is the integration seam only.

- Present EGFX dirty regions ([#1874](https://github.com/Devolutions/IronRDP/issues/1874)) ([2b44c62bdb](https://github.com/Devolutions/IronRDP/commit/2b44c62bdbd997f4aa33bdbaf4749ac4ecc85140)) 

  Propagate validated ResetGraphics extents into the session framebuffer
  and deliver exact packed desktop damage to ActiveX without rebuilding
  full image snapshots.
  
  Coalesce only fully covered regions, preserve sparse updates with a
  64-event and 256 MiB pixel-data budget, and backpressure without
  evicting accepted frame or static-channel payloads. Update retained GDI
  and RPC surfaces in place. Reuse framebuffer storage with fallible
  growth, and retain software cursor shape, position, visibility, and
  clipping across graphics resets. Keep V8 non-AVC negotiation and
  full-frame output fallback unchanged. Rejected oversized resets still
  destroy prior surfaces and are reported as unsupported without
  allocating a session framebuffer.

- Add AVC444 tiles to mixed-codec frames ([#2002](https://github.com/Devolutions/IronRDP/issues/2002)) ([44704e81a7](https://github.com/Devolutions/IronRDP/commit/44704e81a7a835ece00cfb5fd3c2f3609ddb2f67)) 

  Adds AVC444/AVC444v2 tiles to mixed-codec frames with shared Avc444Tile/avc444_wire_pdu helpers, a 10.4-floor capability gate for AVC-in-mixed-frames, per-tile region/QP/quality validation, and a 6-bit QP width check in QuantQuality::encode.

- Skip ZGFX compression for H.264 surface commands ([#2003](https://github.com/Devolutions/IronRDP/issues/2003)) ([2412bb1712](https://github.com/Devolutions/IronRDP/commit/2412bb17123632b717a8974d9627dbf31a810924)) 

  drain_output runs every PDU through the ZGFX compressor when the
  compression mode is Auto or Always, including WireToSurface1 PDUs that
  carry AVC420, AVC444 or AVC444v2. H.264 is already entropy coded, so I
  expect ZGFX to shrink it very little (Auto already falls back to
  uncompressed when compressing doesn't help, so what this saves is the
  CPU spent searching, not bandwidth), and that search runs on the
  caller's send path. The same session usually also carries ClearCodec or
  Planar tiles, which ZGFX shrinks a lot, so turning compression off to
  save the H.264 cost gives that up. The cost is that a byte-identical
  repeat of an earlier H.264 PDU, such as a periodic keyframe of an
  unchanged screen, is now sent in full where ZGFX would have matched it,
  and the with_compression doc says so.
  
  With this change those PDUs are wrapped uncompressed in Auto and Always
  modes and their bytes are appended to the compressor's history without
  searching for matches. The append is required: MS-RDPEGFX 3.1.9.1.2 says
  every output byte, including bytes from segments sent uncompressed, is
  recorded in the history, so skipping the append would make later
  back-references point at the wrong bytes. Never mode is unchanged.
  
  The append is a new public function, zgfx::wrap_uncompressed_recorded,
  which wraps the data in an uncompressed segment and records its bytes in
  the ring from #2001 in one step, so the two can't be done separately;
  Compressor::record_uncompressed stays crate-private. The bytes are not
  bulk indexed, so the search is skipped, though the last two recorded
  bytes can still start a match because the next append indexes the
  positions that straddle the boundary. The with_compression and
  CompressionMode doc comments now say which PDUs are sent uncompressed.
  
  A server test, run for Auto and Always, for AVC420, AVC444 and AVC444v2,
  and for an 8,000 and a 70,000 byte payload (the larger one is
  multipart), sends a ClearCodec frame, an H.264 frame and another
  ClearCodec frame and decompresses everything with one Decompressor. It
  checks that every PDU decodes, that the H.264 PDU went out uncompressed,
  and that both ClearCodec PDUs were compressed, the second one only
  decoding correctly if both sides recorded the H.264 bytes. A unit test
  in zgfx::api does the same for wrap_uncompressed_recorded. The xtask
  fmt, lints, tests, typos and locks checks pass.

- Send part of a frame as an uncompressed region ([#2125](https://github.com/Devolutions/IronRDP/issues/2125)) ([7997e6e051](https://github.com/Devolutions/IronRDP/commit/7997e6e051b6485ff9cf1d517044302a7cb2cbd1)) 

  A client that decodes no compressed codec can only be sent EGFX frames
  through send_uncompressed_frame, and that always places the bitmap at
  the surface origin. A server that knows only part of the screen changed
  still has to send the whole frame, which is 8,294,425 bytes at
  1920x1080. With a test client that advertises no AVC and no Progressive,
  my server logged 303 write stalls in one run, each a write of 4,160,856
  bytes for a 1280x800 frame that took 53 ms.
  
  ZGFX compression doesn't fix it. IronRDP only keeps a compressed result
  that fits one 65,535 byte segment, so on two real 1280x800 desktop
  frames the ratio was 1.00 and compressing took 65 to 190 ms a frame.
  
  This adds GraphicsPipelineServer::try_send_uncompressed_region, which
  takes the destination rectangle and the pixels of that rectangle only.
  It follows the try_send_* senders for AVC and returns a
  FrameSubmissionError: InvalidFrame for an empty region, a region outside
  the surface, or a payload that isn't width times height times four
  bytes, and NotReady, UnknownSurface and Backpressured as the other
  senders do. A rejected region queues nothing and doesn't count as a
  frame in flight. send_uncompressed_frame is unchanged.
  
  The readiness, backpressure and surface-lookup steps the AVC senders
  share now live in one private helper, so the region sender can use them
  without requiring AVC support. The AVC senders check in the same order
  as before.
  
  The tests check that the rectangle reaches the wire as given and that
  each rejection leaves the output queue empty.

- Play video optimized remoting with geometry tracking ([#2185](https://github.com/Devolutions/IronRDP/issues/2185)) ([ed65759d4a](https://github.com/Devolutions/IronRDP/commit/ed65759d4ac670f661fbacfb5e15d14ea98d9b38)) 

  ## Summary
  
  Adds MS-RDPEVOR video optimized remoting with MS-RDPEGT geometry
  tracking to the client.
  
  > Stacked on the camera redirection PR (it reuses
  `DrdynvcClient::with_dynamic_channel_factory` and touches the same
  client wiring). Merge that one first; this PR then retargets to
  `master`.
  
  - **`ironrdp-rdpegt`** (new, `no_std`): `MAPPED_GEOMETRY_PACKET` codec
  with RGNDATA regions, and `GeometryClient`, which tracks the mappings
  and notifies an observer.
  - **`ironrdp-rdpevor`** (new): TSMM PDU codecs, `VideoControlClient` /
  `VideoDataClient`, and a sans-I/O `VideoPresenter` that:
    - answers Start Presentation only once it has a decoder;
  - reassembles fragmented samples and decodes them through the existing
  `ironrdp_egfx::decode::H264Decoder` trait;
    - schedules pictures from their timestamps against an injected clock;
    - scales and clips them to the mapped geometry;
    - asks for a key frame after loss or a decode failure.
  - **`ironrdp-egfx-native`** (new): `MediaFoundationH264Decoder`, an
  `H264Decoder` backed by the H.264 decoder MFT shipped with Windows. It
  runs in low-latency mode, follows format changes, and converts NV12 with
  BT.601 limited or BT.709 full range.
  - **`ironrdp-session`**: `ActiveStage::composite_rgba32`, which
  composites externally rendered pixels and records their damage.
  - **`ironrdp-client`**
    - `video` feature (in `all`) and an `openh264` feature.
  - The default decoder is Media Foundation on Windows, or OpenH264 with
  `openh264`. With no decoder the channels are still declined.
    - A presentation timer and a compositing step in the session loop.
  - A new `ironrdp_egfx:i:<0|1>` property. Windows closes the video
  channels unless the graphics pipeline is on.
  - **`ironrdp-viewer`** / **`ironrdp`**: `openh264`, `client-video`,
  `client-openh264` features.
  - **`ironrdp-daemon`**: built with `video`, so `ironrdp-agent` can play
  RDPEVOR video.
  
  ## Verification
  
  - **Live, Windows Server 2025 through `ironrdp-agent`** (`--prop
  ironrdp_egfx:i:1`):
    - YouTube in Edge streamed as a 672x448 RDPEVOR presentation.
  - 3,035 pictures were presented (86 dropped catching up), placed at the
  RDPEGT mapping; the session screenshot shows the video in place.
  - Without the graphics pipeline the server closes the video channels.
  Geometry and video channels are reopened during the session, which the
  factory registration handles.
  - **Tests**: `ironrdp-testsuite-core` covers the MS-RDPEVOR and
  MS-RDPEGT specification examples, malformed input, reassembly, timing,
  loss and key-frame requests, and scaling and clipping.
  - The real H.264 sample from MS-RDPEVOR 4.3 decodes through OpenH264
  (`openh264-bundled`) and through Media Foundation (Windows), and lands
  at the MS-RDPEGT 4.1 rectangle.
    - The two decoders agree within a mean channel difference of 2.
  - **Checks**: `cargo xtask check fmt -v`, `cargo xtask check lints -v`
  and `cargo xtask check locks -v` pass.
  - **Known**: `e2e::egfx_moves_onto_the_udp_tunnel_with_soft_sync` fails
  locally on `master` as well; it is unrelated to this change.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------

- Support client-initiated protocol reset ([#2207](https://github.com/Devolutions/IronRDP/issues/2207)) ([e4a0d17b8e](https://github.com/Devolutions/IronRDP/commit/e4a0d17b8e5b1b81758202dc131d8fa2aed58a23)) 

  Expose `GraphicsPipelineClient::request_protocol_reset()` to rebuild
  graphics state without reconnecting RDP or interrupting other virtual
  channels.
  For negotiated V10.3–V10.7, it re-advertises the confirmed capability
  version and flags, clears surfaces and codec state, and ignores
  in-flight graphics until the server confirms that version again.
  Changed flags are accepted; a processing error while awaiting reset
  confirmation closes the client and subsequent input is rejected until
  the channel is recreated with a new processor.
  The alternate `V10_6Err` value is echoed only if previously confirmed.
  
  The ACK count restarts because Windows resets its sent-frame count and
  rejects cumulative post-reset ACKs.
  ZGFX history and the last output extent remain intact, while pending
  pixels and bitmap caches are discarded.
  The caller must send the returned PDU before processing more input and
  recreate the channel if sending or reset confirmation fails.
  Unsupported versions and an already pending reset return `None`.
  
  This implements the client-side reset described in [MS-RDPEGFX section
  3.3.5.19](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/a720af02-0a47-44e2-a934-dc8534062b44)
  and complements the server support in #1833.
  
  ---------

- [**breaking**] Use H.264 and AVC444 graphics by default ([#2199](https://github.com/Devolutions/IronRDP/issues/2199)) ([a0c8c0dc1e](https://github.com/Devolutions/IronRDP/commit/a0c8c0dc1e1296f428092d3158585cfc7d395efb)) 

  The graphics pipeline was off by default and got no H.264 decoder, and
  AVC444 and AVC444v2 updates were forwarded to the handler undecoded, so
  Windows servers sent no AVC to ironrdp-client or ActiveX.
  
  `H264Decoder` gains `decode_yuv420` and `supports_yuv420`, with
  defaults, so a decoder can hand over the planes of a picture; the Media
  Foundation and OpenH264 decoders implement them. The graphics pipeline
  then converts AVC420 pictures with the MS-RDPEGFX color transform
  itself, whatever matrix the decoder was created with. A new
  `ironrdp_egfx::avc444` module keeps the combined YUV 4:4:4 picture of
  each surface: main views update luma and subsampled chroma, auxiliary
  views the remaining chroma (MS-RDPEGFX 3.3.8.3.2 and 3.3.8.3.3), each
  within its own region rectangles, so luma-only and chroma-only updates
  combine with earlier ones; the reverse chroma filter runs at conversion.
  The same type splits a picture into the two views for servers.
  
  The default capability list now offers versions 10.7 down to 10 before
  8.1 and 8. Sets implying AVC444 are dropped when the decoder returns no
  YUV planes, as AVC sets are without a decoder. The confirmed
  `SMALL_CACHE`, version 8 `THINCLIENT` and version 10.3 now report and
  apply the 16 MB bitmap cache cap.
  
  In ironrdp-client, `H264DecoderFactory` moves to a new `h264` module and
  becomes the one decoder source for the graphics pipeline and video
  optimized remoting: Media Foundation's on Windows (ironrdp-egfx-native
  is now a regular Windows dependency), OpenH264 with the `openh264`
  feature, which no longer implies `video`. `ChannelConfig::h264_decoder`
  holds it, `ConfigBuilder::with_h264_decoder` sets it without the `video`
  feature, and `VideoConfig` keeps only an `enabled` flag for
  `videoplaybackmode`. The graphics pipeline is advertised by default when
  a decoder is configured; `with_graphics_pipeline(false)`,
  `ironrdp_egfx:i:0` or `--egfx=false` opt out, since mstsc has no `.rdp`
  key for it. ActiveX gets it through the same default. The graphics
  pipeline's decoder starts on another thread during the connection
  sequence (`H264DecoderFactory::create_in_background`) and is awaited
  through the new `H264Decoder::wait_until_ready` when the graphics
  channel opens, so a slow Media Foundation start (now allowed ten
  seconds) does not delay connecting; if it fails, no AVC set is
  advertised.

### <!-- 4 -->Bug Fixes

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

- Declare the complete capability ladder in the default server preferences ([#1854](https://github.com/Devolutions/IronRDP/issues/1854)) ([8cdd788ce7](https://github.com/Devolutions/IronRDP/commit/8cdd788ce7643488ef06df743038815789fd122b)) 

  The default preferred_capabilities ladder in GraphicsPipelineHandler
  declares four of the eleven capability versions this build can
  interpret: V10.7, V10, V8.1 and V8. Negotiation matches exact versions
  (discriminant equality, highest server priority first), so a client that
  advertises only a mid-tier V10 variant falls through every rung: it
  negotiates nothing at the V10 level and either drops to V8-class
  capabilities or fails the channel, despite both sides fully supporting,
  say, V10.6. Mid-tier-only advertisements exist in the wild; the Windows
  App family is the known case.
  
  This completes the default ladder to all eleven versions in priority
  order, with the flags each version actually defines: SMALL_CACHE where
  the version has it, V10.3's flag set left empty (it defines no
  SMALL_CACHE, and an empty set means AVC enabled), V10.1 as the unit
  variant, and the existing V8.1/V8 rungs unchanged.
  CodecCapabilities::from_capability_set already maps every one of these
  versions correctly, so a newly negotiable mid-tier version derives the
  right AVC availability with no other change.
  
  The handler documentation now explains why the complete ladder matters,
  and the negotiation function itself is untouched.

- [**breaking**] Use exclusive AVC region bounds ([#1788](https://github.com/Devolutions/IronRDP/issues/1788)) ([e0727394c0](https://github.com/Devolutions/IronRDP/commit/e0727394c0c5ef172a13617594e1b2db90e006f7)) 

  MS-RDPEGFX 2.2.1.2 defines `RDPGFX_RECT16` with exclusive `right` and
  `bottom`, and 2.2.4.4.1 gives `regionRects` in `RFX_AVC420_METABLOCK`
  that same type. `Avc420Region` documented its edges as inclusive,
  `full_frame()` built `width - 1` / `height - 1`, and `to_rectangle()`
  handed those to an `InclusiveRectangle` that encoded them unchanged, so
  every `regionRects` entry went out a pixel short on the right and bottom
  edge. `GraphicsPipelineServer::compute_dest_rect()`, used for the
  `WireToSurface1` destination, does add that pixel back: the two call
  sites disagreed about whether the conversion had already happened.
  
  `Avc420Region` is exclusive throughout now and `to_rectangle` produces
  an `ExclusiveRectangle`, so the asymmetric adjustment disappears.
  `fix(egfx): bound both AVC444 streams` then computes the destination
  rectangle from both streams rather than from the luma one, so a chroma
  region larger than luma is no longer cut short. `feat(egfx): add
  explicit AVC444v2 frame sender` separates the AVC444v2 path from AVC444;
  both fixes build on it.
  
  What the defect costs: with `ironrdp-server` on master, Windows App on
  macOS drops the connection right after the encoder starts, the server
  reporting `peer closed connection without sending TLS close_notify`.
  Pinning the session resolution, so that no deactivation-reactivation
  happens at all, changes nothing. With these three commits the same
  server build and the same client run normally. FreeRDP-based clients
  accept the frames either way, which is why this shows only against the
  Microsoft client. That is not proof this field is the trigger -
  2.2.4.4.1 also calls the metablock informational - only that the field
  is wrong on the wire and that these commits make that client work.
  
  `avc.rs` lost its inline test module to #1736 while these sat in a fork,
  so the `full_frame` expectation is updated in
  `testsuite-core/tests/egfx/avc.rs` instead.
  
  Please rebase-merge rather than squash. The three commits are
  @MuNeNiCK's with his authorship intact, and the breaking change is
  confined to the middle one; squashing would collapse both.

- Decode AVC420 with full-range BT.709 ([#1923](https://github.com/Devolutions/IronRDP/issues/1923)) ([9542e017da](https://github.com/Devolutions/IronRDP/commit/9542e017da4481ba4919b944f4e0742f23a83742)) 

- Do not track frames a suspended client cannot acknowledge ([#1912](https://github.com/Devolutions/IronRDP/issues/1912)) ([6ceb5387b5](https://github.com/Devolutions/IronRDP/commit/6ceb5387b546290b5d49c60e126ad07c69fe70ba)) 

- Draw AVC420 partial updates from regionRects ([#2042](https://github.com/Devolutions/IronRDP/issues/2042)) ([#2052](https://github.com/Devolutions/IronRDP/issues/2052)) ([9f07fd4b20](https://github.com/Devolutions/IronRDP/commit/9f07fd4b2047ef6147454bace54f97bc45d2e9fb)) 

  AVC420 partial frame updates paint the wrong pixels. On the client
  decode path, `decode_avc420` decoded the `Avc420BitmapStream` but only
  ever used `stream.data` — the `regionRects` in `stream.rectangles` (the
  `RFX_AVC420_METABLOCK` from [MS-RDPEGFX] 2.2.4.4) were never read.
  `crop_decoded_frame` then copied a single block from the decoded frame's
  origin `(0,0)` and blitted it across the whole destination rectangle.
  
  So whenever a frame's changed regions did not happen to sit at the
  top-left corner, each region was filled with pixels lifted from `(0,0)`,
  and the space between regions got overwritten. That is the "horizontal
  lines and unfilled rectangle outlines" people see in GFX / H.264
  (AVC420) mode once the server starts sending partial updates instead of
  full frames.
  
  The fix follows the spec: `regionRects` are the sub-regions that
  actually changed, each one takes its pixels from the *same* `(x, y)` in
  the decoded frame, and the PDU's destination rectangle is just their
  bounding box — not a copy source. `decode_avc420` now walks
  `stream.rectangles`, clips each rect to `surface ∩ frame`, copies `(x,
  y) → (x, y)` through a small new `copy_frame_region` helper, and emits
  one surface update per rectangle. A stream that carries no `regionRects`
  keeps the old single bounding-box path, so nothing changes for that
  case.
  
  The change is deliberately narrow — only the client decode path in
  `crates/ironrdp-egfx/src/client.rs`. AVC444, the server encode path, and
  the NAL-format work in #1986 are all left alone.
  
  For the test, a deterministic stand-in decoder stamps every pixel with
  its own coordinate (`R = x`, `G = y`), so a test can tell which source
  pixel actually landed where. It sends an AVC420 update with two disjoint
  regions, neither at the origin, whose bounding box is the destination
  rectangle, and checks that each region is drawn from its own
  coordinates. Before the fix it fails with a single origin-cropped update
  where two were expected; after the fix it gets one update per region,
  with region B at `(32, 32)` carrying its own pixels. `cargo test -p
  ironrdp-egfx` passes (54 tests) and `cargo clippy -p ironrdp-egfx
  --all-targets` is clean.
  
  This one is a co-fix. The root-cause analysis and the region-rects
  harness/recipe (`c06_avc420_region_rects.rs`) came from the issue
  author, @se-wo; this PR implements that recipe on the client decode path
  and turns the harness into an in-tree unit test. Glad to fold in
  @se-wo's original harness verbatim or adjust the attribution however
  maintainers prefer.

- Zero cbAvc420EncodedBitstream1 for chroma-only AVC444 streams ([#1999](https://github.com/Devolutions/IronRDP/issues/1999)) ([bdf01f6b3a](https://github.com/Devolutions/IronRDP/commit/bdf01f6b3a68feb69038069999340590a5126946)) 

  Makes Avc444BitmapStream::encode write zero into cbAvc420EncodedBitstream1 when LC=CHROMA, matching MS-RDPEGFX 2.2.4.5/2.2.4.6 (no YUV420p frame in the first sub-stream), while leaving LC=0/1 unchanged; the decoder still accepts the non-zero form some servers send.

- Decode AVC420 sent as an Annex B byte stream ([#1986](https://github.com/Devolutions/IronRDP/issues/1986)) ([dadacf54b9](https://github.com/Devolutions/IronRDP/commit/dadacf54b97368816ac0ffdf68907988ee5e83a8)) 

  ## Problem

- Respect client frame acknowledgement limits ([#2102](https://github.com/Devolutions/IronRDP/issues/2102)) ([e4143f2630](https://github.com/Devolutions/IronRDP/commit/e4143f26306a6639a889d80c1c484ca69c365a8f)) 

  A client advertising a one-frame ACK window can currently be sent frames
  using the server's larger configured window. An AVC producer also
  receives the same `None` for a full window as for a permanent submission
  error, which makes ordinary backpressure difficult to handle without
  replacing the encoder.
  
  This PR is based directly on `master` and is independent of #1413. It
  contains no credential, binder or acceptor changes.
  
  * Apply the nonzero core Frame Acknowledge capability as a ceiling on
  the operator-configured EGFX window, including legacy graphics
  factories. Configured zero remains unlimited only when the client has no
  ceiling; resize preserves the negotiated ceiling.
  * Reject unknown/stale ACKs before they can suspend or mutate tracking.
  Preserve valid suspension/resumption for frames issued in the current
  channel generation, and clear channel-owned state on close.
  * Report client queue depth as `Unavailable`: our client counts frames,
  while the protocol field measures bytes. This is distinct from
  `Suspend`.
  * Add typed AVC420, AVC444 and AVC444v2 submission errors, including
  `Backpressured`, while retaining the existing `Option` methods as
  compatibility wrappers. Mixed-codec send APIs are unchanged.
  
  WRDP exposed the configured-three/client-one case during desktop
  dragging. These changes let the producer retain its encoder and
  retry/recover after an ACK instead of treating backpressure as encoder
  failure.
  
  Focused checks passed locally (Rust 1.96; the repository pins 1.94.1):
  
  ```
  cargo test -p ironrdp-egfx --lib                               # 55 passed
  cargo test -p ironrdp-testsuite-core --test integration_tests_core -- egfx::server  # 92 passed
  cargo test -p ironrdp-server --lib --features egfx -- frame_ack # 2 passed
  cargo test -p ironrdp-server --lib --features egfx -- gfx::tests # 2 passed
  cargo fmt --all
  git diff --check
  ```
  
  Regressions cover window/zero semantics, activation clamps, resize,
  legacy factories, stale suspend/resume ACKs across close/reopen,
  restored tracking after valid resume, and typed rejection without queued
  output. A read-only review caught the legacy-factory and suspended-ACK
  resume paths; both are corrected and covered here.
  
  Happy to adjust the additive error API to the project's preferred shape
  during review. This PR can be reviewed and merged separately from the
  session-binding work.
  
  ---------

- Keep the bitmap cache across ResetGraphics ([#2011](https://github.com/Devolutions/IronRDP/issues/2011)) ([b60ffdffa2](https://github.com/Devolutions/IronRDP/commit/b60ffdffa2ab17b3cbe898aec6610d6bd8605493)) 

  The compositor emptied the bitmap cache on every ResetGraphics. Windows
  reuses cached toolbars, icons and text after a resize, so subsequent
  CacheToSurface commands could leave those regions black until a fresh
  upload.

- Drop undecodable surface updates instead of the session ([#1789](https://github.com/Devolutions/IronRDP/issues/1789)) ([3f977f89f8](https://github.com/Devolutions/IronRDP/commit/3f977f89f87e42aa7cdc27956d30662fd90eddf3)) 

  Codec decode errors currently propagate through EGFX and terminate the
  RDP session. Drop rejected AVC420, ClearCodec, Planar and Progressive
  updates, retain protocol-persistent decoder contexts and caches, and
  throttle diagnostics. Logs identify the actual codec and retain AVC420
  frame/destination dimensions.
  
  Only bitmap decoder failures become nonfatal. Unknown surfaces and
  invalid rectangle ordering retain upstream validation behavior; recovery
  from malformed headers or surface lifecycle errors needs a separate
  policy. A client without an H.264 decoder retains the existing
  capability-skip behavior. The cumulative `dropped_updates` accessor
  counts updates rejected after attempted codec decoding, and capture
  replay uses it alongside the unsupported-codec flag to report gaps
  without terminating playback.
  
  Regression tests cover Progressive upgrades on two surfaces after 64
  failures, a cached ClearCodec glyph across failures and `ResetGraphics`,
  actual CONTEXT-less Progressive rendering after `ResetGraphics`, replay
  gaps for malformed supported codecs and unsupported AVC420, and retained
  AVC420 dimension diagnostics. The context/cache regressions fail with
  the old failure-streak reset heuristic.
  
  Default EGFX/graphics/capture-replay tests (390), EGFX all-feature tests
  (63), targeted Clippy and formatting pass. All-feature Clippy reproduces
  existing `encode.rs` lint failures on unmodified `master`. A final local
  Codex review found no remaining actionable findings.

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
