use std::sync::Arc;

use crate::buffer::PduBuffer;
use crate::process::{ReplayProcessor, ReplayProcessorConfig};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_session::image::DecodedImage;
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, ImageData, OffscreenCanvas, OffscreenCanvasRenderingContext2d, console,
};

/// Default desktop resolution used until the server sends ResolutionChanged.
const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

/// Result returned to JavaScript after `render_till()` completes.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct RenderResult {
    /// The number of PDUs skipped because processing failed.
    pub pdu_errors: u32,
    /// Whether a SessionEnded PDU was encountered.
    /// The caller should stop playback when this is true.
    pub session_ended: bool,
}

/// JS-facing configuration for replay initialization.
///
/// All fields are optional.
/// Unset fields use protocol-common defaults from [`ReplayProcessorConfig::default()`].
/// Channel IDs are narrowed to `u16` in [`Replay::init()`].
///
/// For example, construct a config in JavaScript with `const cfg = new ReplayConfig(); cfg.io_channel_id = 1005;`.
#[derive(Default)]
#[wasm_bindgen]
pub struct ReplayConfig {
    /// MCS I/O channel ID. Must fit in `u16`; `init()` returns an error if out of range.
    pub io_channel_id: Option<u32>,
    /// MCS user channel ID. Must fit in `u16`; `init()` returns an error if out of range.
    pub user_channel_id: Option<u32>,
    /// Share ID from Server Demand Active.
    pub share_id: Option<u32>,
}

#[wasm_bindgen]
impl ReplayConfig {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
}

#[wasm_bindgen]
pub struct Replay {
    pdu_buffer: PduBuffer,
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    /// INVARIANT: `None` before `init()` is called, `Some` after.
    processor: Option<ReplayProcessor>,
    config: ReplayProcessorConfig,
    image: DecodedImage,
    // Cursor state.
    pointer_hidden: bool,
    pointer_hotspot_x: u16,
    pointer_hotspot_y: u16,
    mouse_x: u16,
    mouse_y: u16,
    cursor_canvas: Option<OffscreenCanvas>,
    /// The pointer used to build `cursor_canvas`, compared via `Arc::ptr_eq` to skip redundant rebuilds.
    cached_pointer: Option<Arc<DecodedPointer>>,
}

#[wasm_bindgen]
impl Replay {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Replay, JsValue> {
        let ctx = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("failed to get 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

        Ok(Self {
            pdu_buffer: PduBuffer::new(),
            canvas,
            ctx,
            processor: None,
            config: ReplayProcessorConfig::default(),
            image: DecodedImage::new(PixelFormat::RgbA32, DEFAULT_WIDTH, DEFAULT_HEIGHT),
            pointer_hidden: false,
            pointer_hotspot_x: 0,
            pointer_hotspot_y: 0,
            mouse_x: 0,
            mouse_y: 0,
            cursor_canvas: None,
            cached_pointer: None,
        })
    }

    /// Initialize the replay processor, optionally with custom channel configuration.
    ///
    /// Call exactly once before `render_till()` or `set_update_canvas()`.
    /// The method returns an error if already initialized.
    /// To replay with the same configuration, call `reset()`.
    /// To use a different configuration, create a new `Replay` instance.
    pub fn init(&mut self, config: Option<ReplayConfig>) -> Result<(), JsValue> {
        if self.processor.is_some() {
            return Err(JsValue::from_str("replay already initialized"));
        }

        // Merge the JS configuration onto the stored defaults.
        // Channel IDs are narrowed from u32 to u16 here.
        // Reject out-of-range values rather than silently clamping them.
        if let Some(cfg) = config {
            if let Some(v) = cfg.io_channel_id {
                self.config.io_channel_id =
                    u16::try_from(v).map_err(|_| JsValue::from_str("io_channel_id exceeds u16 range"))?;
            }
            if let Some(v) = cfg.user_channel_id {
                self.config.user_channel_id =
                    u16::try_from(v).map_err(|_| JsValue::from_str("user_channel_id exceeds u16 range"))?;
            }
            if let Some(v) = cfg.share_id {
                self.config.share_id = v;
            }
        }

        self.processor = Some(ReplayProcessor::new(&self.config));
        Ok(())
    }

    /// Processes all PDUs up to `target_ms` and blits the resulting framebuffer to the canvas.
    ///
    /// Returns a JS error if `init()` has not been called.
    ///
    /// # Panics
    ///
    /// Panics if called after `init()` succeeds but the processor is somehow `None`.
    /// This state is unreachable during normal use.
    #[wasm_bindgen(js_name = renderTill)]
    pub fn render_till(&mut self, target_ms: f64) -> Result<RenderResult, JsValue> {
        if self.processor.is_none() {
            return Err(JsValue::from_str("replay not initialized -- call init() first"));
        }

        // Narrow the processor borrow so `self.pdu_buffer` and `self.image` remain accessible.
        let result = {
            #[expect(clippy::unwrap_used, reason = "processor is Some per is_none() early return")]
            let processor = self.processor.as_mut().unwrap();
            processor.process_till(&mut self.pdu_buffer, &mut self.image, target_ms)
        };

        // Log any processing errors because the offending PDUs were skipped without aborting playback.
        // Cap individual messages to avoid flooding the browser console on corrupted recordings.
        const MAX_LOGGED_ERRORS: usize = 20;

        let logged_error_count = result.errors.len().min(MAX_LOGGED_ERRORS);
        for error in result.errors.iter().take(logged_error_count) {
            console::error_1(&format!("PDU processing error: {error}").into());
        }
        let suppressed_errors = &result.errors[logged_error_count..];
        if !suppressed_errors.is_empty() {
            console::error_1(
                &format!(
                    "Suppressed {} additional PDU processing errors",
                    suppressed_errors.len()
                )
                .into(),
            );
        }

        // Apply canvas-specific side effects.
        if let Some((width, height)) = result.new_resolution {
            self.canvas.set_width(u32::from(width));
            self.canvas.set_height(u32::from(height));
        }

        // Synchronize cursor state from the processor and release the borrow.
        let pointer_state = {
            #[expect(clippy::unwrap_used, reason = "processor is Some per is_none() early return")]
            self.processor.as_ref().unwrap().current_pointer_state().clone()
        };
        match &pointer_state {
            crate::process::PointerState::Bitmap(pointer) => {
                let pointer_changed = self
                    .cached_pointer
                    .as_ref()
                    .is_none_or(|prev| !Arc::ptr_eq(prev, pointer));
                if pointer_changed {
                    self.pointer_hotspot_x = pointer.hotspot_x;
                    self.pointer_hotspot_y = pointer.hotspot_y;
                    self.cursor_canvas = Self::build_cursor_canvas(pointer);
                    self.cached_pointer = Some(Arc::clone(pointer));
                }
                self.pointer_hidden = false;
            }
            crate::process::PointerState::Default => {
                self.pointer_hidden = false;
                self.cursor_canvas = None;
                self.cached_pointer = None;
            }
            crate::process::PointerState::Hidden => {
                self.pointer_hidden = true;
            }
        }

        if let Some((x, y)) = result.last_mouse_position {
            self.mouse_x = x;
            self.mouse_y = y;
        }

        if result.frame_dirty && self.processor.as_ref().is_some_and(|p| p.update_canvas()) {
            self.draw_to_canvas();
        }

        Ok(RenderResult {
            pdu_errors: u32::try_from(result.errors.len()).unwrap_or(u32::MAX),
            session_ended: result.session_ended,
        })
    }

    /// Pushes a single PDU into the internal buffer.
    /// JavaScript calls this method to feed PDU data before calling `renderTill()`.
    ///
    /// Returns a JavaScript error and does not enqueue the PDU when the timestamp is not finite.
    #[wasm_bindgen(js_name = pushPdu)]
    pub fn push_pdu(&mut self, timestamp_ms: f64, source: crate::PduSource, data: &[u8]) -> Result<(), JsValue> {
        self.pdu_buffer
            .push_pdu(timestamp_ms, source, data)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Resets playback state to the beginning and rebuilds the processor from the stored configuration.
    ///
    /// # Caller contract
    /// This method does not clear the canvas.
    /// Do not display the canvas between `reset()` and the first `render_till()` call.
    ///
    /// Returns a JS error if `init()` has not been called.
    pub fn reset(&mut self) -> Result<(), JsValue> {
        if self.processor.is_none() {
            return Err(JsValue::from_str("replay not initialized -- call init() first"));
        }

        self.pdu_buffer.clear();
        self.image = DecodedImage::new(PixelFormat::RgbA32, DEFAULT_WIDTH, DEFAULT_HEIGHT);
        self.processor = Some(ReplayProcessor::new(&self.config));
        self.pointer_hidden = false;
        self.pointer_hotspot_x = 0;
        self.pointer_hotspot_y = 0;
        self.mouse_x = 0;
        self.mouse_y = 0;
        // Drop the cached OffscreenCanvas to free the JavaScript object reference.
        self.cursor_canvas = None;
        self.cached_pointer = None;
        Ok(())
    }

    /// Enables or disables canvas updates during rendering.
    /// Set this to false during seek fast-forward to suppress intermediate frame blits.
    ///
    /// Returns a JS error if `init()` has not been called.
    #[wasm_bindgen(js_name = setUpdateCanvas)]
    pub fn set_update_canvas(&mut self, update: bool) -> Result<(), JsValue> {
        self.processor_mut()?.set_update_canvas(update);
        Ok(())
    }

    /// Blits the current in-memory framebuffer to the canvas without processing any PDUs.
    ///
    /// Use this after a seek when PDUs have been processed but the final frame has not been drawn.
    #[wasm_bindgen(js_name = forceRedraw)]
    pub fn force_redraw(&self) {
        self.draw_to_canvas();
    }

    /// Blits the framebuffer to the canvas with `putImageData()` and composites the cursor on top.
    fn draw_to_canvas(&self) {
        let width = u32::from(self.image.width());
        let height = u32::from(self.image.height());
        let clamped = wasm_bindgen::Clamped(self.image.data());

        let Ok(image_data) = ImageData::new_with_u8_clamped_array_and_sh(clamped, width, height) else {
            return;
        };

        // Skip cursor compositing if the frame blit fails.
        if self.ctx.put_image_data(&image_data, 0.0, 0.0).is_ok() {
            self.draw_cursor();
        }
    }

    /// Builds a cached OffscreenCanvas from a cursor bitmap.
    /// This method runs once per `PointerBitmap` change and returns `None` on failure.
    fn build_cursor_canvas(pointer: &DecodedPointer) -> Option<OffscreenCanvas> {
        if pointer.width == 0 || pointer.height == 0 {
            return None;
        }

        let Ok(offscreen) = OffscreenCanvas::new(u32::from(pointer.width), u32::from(pointer.height)) else {
            console::warn_1(&"Failed to create OffscreenCanvas for cursor".into());
            return None;
        };

        let Ok(Some(obj)) = offscreen.get_context("2d") else {
            console::warn_1(&"Failed to get 2d context from cursor OffscreenCanvas".into());
            return None;
        };

        let Ok(offscreen_ctx) = obj.dyn_into::<OffscreenCanvasRenderingContext2d>() else {
            console::warn_1(&"Failed to cast cursor OffscreenCanvas context".into());
            return None;
        };

        let clamped = wasm_bindgen::Clamped(pointer.bitmap_data.as_slice());
        let Ok(image_data) =
            ImageData::new_with_u8_clamped_array_and_sh(clamped, u32::from(pointer.width), u32::from(pointer.height))
        else {
            console::warn_1(&"Failed to create ImageData for cursor bitmap".into());
            return None;
        };

        let Ok(()) = offscreen_ctx.put_image_data(&image_data, 0.0, 0.0) else {
            console::warn_1(&"Failed to write cursor bitmap to OffscreenCanvas".into());
            return None;
        };

        Some(offscreen)
    }

    /// Composite the cached cursor canvas onto the main canvas at the current mouse position.
    fn draw_cursor(&self) {
        if self.pointer_hidden {
            return;
        }

        let Some(cursor_canvas) = &self.cursor_canvas else {
            return;
        };

        let dest_x = f64::from(self.mouse_x.saturating_sub(self.pointer_hotspot_x));
        let dest_y = f64::from(self.mouse_y.saturating_sub(self.pointer_hotspot_y));

        // Non-fatal if compositing fails; the frame is already drawn correctly.
        let _ = self.ctx.draw_image_with_offscreen_canvas(cursor_canvas, dest_x, dest_y);
    }

    /// Returns a mutable reference to the processor, or a JS error if not initialized.
    fn processor_mut(&mut self) -> Result<&mut ReplayProcessor, JsValue> {
        self.processor
            .as_mut()
            .ok_or_else(|| JsValue::from_str("replay not initialized -- call init() first"))
    }
}
