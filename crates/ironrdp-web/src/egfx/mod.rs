//! EGFX (MS-RDPEGFX) graphics pipeline for the web client.
//!
//! The server can deliver graphics over the `Microsoft::Windows::RDS::Graphics` dynamic virtual
//! channel as H.264/AVC instead of progressive RemoteFX tiles — a coded picture per frame, with
//! real `StartFrame`/`EndFrame` boundaries. This module decodes that H.264 with the browser's
//! WebCodecs `VideoDecoder` (hardware, async) and composites the decoded `VideoFrame`s straight into
//! softblit's GPU texture, so full-screen video updates atomically instead of filling top-to-bottom.
//!
//! ## Data flow & the Send / `!Send` boundary
//!
//! ```text
//! DVC processing (Send, inside ActiveStage::process)        render loop (!Send)
//! ┌───────────────────────────────────────────┐            ┌──────────────────────────────────┐
//! │ GraphicsPipelineClient                     │  EgfxUpdate│ EgfxCompositor                    │
//! │  → WebGfxHandler ──────────────────────────┼───mpsc────▶│  apply_update():                 │
//! │     (on_surface_*, on_bitmap_updated,      │            │   • CPU bitmaps → DecodedImage    │
//! │      on_avc420_bitstream, on_frame_complete│            │   • AVC420 → WebCodecsH264Decoder │
//! └───────────────────────────────────────────┘            │       │ output (async)            │
//!                                                           │       ▼ DecodedVideo (mpsc)       │
//!                                                           │   import_video_frame → softblit   │
//!                                                           └──────────────────────────────────┘
//! ```
//!
//! The handler must be `Send` (it lives in the DVC processor); WebCodecs and the GPU surface are
//! `!Send` and live in the render loop. The handler therefore only forwards [`EgfxUpdate`] messages
//! over an mpsc channel; all decoding and presentation happens on the render-loop side.

mod compositor;
mod decoder;
mod handler;
mod update;

pub(crate) use compositor::EgfxCompositor;
pub(crate) use decoder::{DecodedVideo, DecodedVideoQueue};
pub(crate) use handler::WebGfxHandler;
pub(crate) use update::EgfxUpdate;
