# EGFX + H.264 — WebCodecs (web) / openh264 (native CPU fallback)

Branch: `experiment/egfx-webcodecs` (off `experiment/softblit-worker`).

Goal: full-screen video updates as **atomic, hardware-decoded frames** instead of progressive
RemoteFX tile-fill. Server sends EGFX (MS-RDPEGFX) with AVC420/H.264; the client decodes per
coded-picture (real frame boundaries). Web uses **WebCodecs `VideoDecoder`** (GPU, async, in the
worker); native uses **openh264** (CPU, sync) — both behind the existing `ironrdp-egfx` pipeline.

## What exists already
- `ironrdp-egfx`: full PDU parsing (incl. `Avc420BitmapStream`), surface management, codec dispatch,
  frame markers, capability negotiation, and a pluggable **sync `H264Decoder` trait** with an
  **openh264** impl (feature-gated) — this is the native CPU path.
- The **worker + softblit (WebGPU) + OffscreenCanvas** render foundation (`experiment/softblit-worker`).

## Done on this branch
1. **`feat(egfx)` — external AVC420 decode seam** (committed): `GraphicsPipelineClient::
   with_external_avc_decode()`. When set (and no in-process `H264Decoder`), AVC420 frames are handed
   to `GraphicsPipelineHandler::on_avc420_bitstream(surface_id, dest_rect, h264_data)` instead of
   skipped, and AVC caps are advertised. This is the seam an **async** decoder (WebCodecs) needs,
   since the `H264Decoder` trait is synchronous and can't wrap WebCodecs.
2. **WebCodecs H.264 decoder** (`iron-svelte-client/src/lib/worker/h264-webcodecs.ts`, typechecks):
   AVC-format (length-prefixed) → `VideoFrame`. Parses SPS/PPS, builds the `avcC` description,
   configures `VideoDecoder` with `optimizeForLatency`, decodes in order, delivers frames via
   callback. `webcodecs.d.ts` provides the ambient WebCodecs types.

## The hard part — integration (remaining)

**The `Send` / `!Send` split.** The EGFX `GraphicsPipelineHandler` runs inside DVC processing (must
be `Send`); WebCodecs and the canvas are `!Send` and live in the render loop. So:

- Add an `EgfxUpdate` channel (futures mpsc), mirroring the existing `RdpInputEvent` pattern.
- A `Send` handler in `ironrdp-web` forwards events: `Avc420Bitstream { surface_id, rect, data }`,
  `Bitmap { … }` (uncompressed), `SurfaceMapped { … }`, `FrameComplete`.
- Register the channel in the connector at `session.rs:~1618`:
  `DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(…)).with_dynamic_channel(
  GraphicsPipelineClient::new(Box::new(WebGfxHandler{tx}), None).with_external_avc_decode())`.
- The render loop (in the worker) gains a `select!` branch on `egfx_rx`:
  - `Avc420Bitstream` → feed `H264WebCodecsDecoder.decode(data)`; on `onFrame(VideoFrame)`, upload to
    the surface at `rect` and present.
  - map EGFX surfaces → the presented output (`MapSurfaceToOutput`).

**`VideoFrame` → GPU present.** softblit currently uploads dirty rects from a **CPU** framebuffer
(`write_texture`). A WebCodecs `VideoFrame` is already a GPU/texture resource — uploading it wants
`copyExternalImageToTexture` / importing the frame as a texture, **not** a CPU readback. This needs a
softblit API addition (`present_external_image(VideoFrame, rect)` or similar). This is the main new
softblit work and overlaps the softblit author's domain. (Fallback: `VideoFrame.copyTo()` to CPU then
the existing path — correct but defeats the zero-copy goal.)

**AVC444.** `ironrdp-egfx` currently leaves AVC444/v2 as `on_unhandled_pdu`. AVC444 = two H.264
streams (main YUV420 + auxiliary) recombined to YUV444 (a shader). Add `Avc444BitmapStream` parsing +
an `on_avc444_bitstream` seam + dual-`VideoDecoder` + recombine. Defer until AVC420 works end-to-end.

**Native CPU fallback.** `ironrdp-client` must register the EGFX channel with an `OpenH264Decoder`
(`ironrdp-egfx` feature `openh264` / `openh264-bundled`). Confirm/add the wiring in the native client;
the decoder itself exists.

## Codec negotiation reality
Windows often defaults to **RFX Progressive** inside EGFX unless the AVC444 GPO is set, and RFX
Progressive is *also* progressive (would still band). To exercise the H.264 path, pin the test server
to H.264/AVC444 via GPO ("Prioritize H.264/AVC 444 Graphics mode"). Keep a non-AVC EGFX fallback or
the existing RemoteFX path for servers that don't send H.264.

## Suggested PR split
1. `feat(egfx)`: external AVC420 decode seam (done).
2. `feat(softblit)`: `VideoFrame` texture-import present path.
3. `feat(web)`: EGFX channel + `Send` handler + `EgfxUpdate` channel + render-loop WebCodecs decode/present.
4. `feat(client)`: native EGFX wiring with openh264.
5. `feat(egfx)`: AVC444 (dual-stream + recombine) + WebCodecs/openh264 support.

## De-risking
Stage with a **sync wasm H.264 decoder** (openh264-to-wasm) implementing the existing `H264Decoder`
trait first — validates EGFX channel + surface model + present end-to-end with minimal new
architecture — then swap in WebCodecs for the async/hardware path.
