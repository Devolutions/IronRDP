#![allow(clippy::new_without_default)] // Default trait can't be used by wasm consumer anyway.

// Silence the unused_crate_dependencies lint - tracing will be used for logging later.
extern crate tracing as _;

mod error;
mod reader;

use wasm_bindgen::prelude::*;

use crate::reader::ReplayReader;

/// A replay session for playing back recorded RDP sessions.
#[wasm_bindgen]
pub struct Replay {
    reader: ReplayReader,
    width: u16,
    height: u16,
}

#[wasm_bindgen]
impl Replay {
    /// Creates a new Replay instance.
    ///
    /// # Arguments
    /// * `db_name` - Name of the IndexedDB database containing frames
    /// * `width` - Desktop width in pixels
    /// * `height` - Desktop height in pixels
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const replay = await Replay.create("rdp_replay", 1920, 1080);
    /// ```
    #[wasm_bindgen]
    pub async fn create(db_name: &str, width: u16, height: u16) -> Result<Replay, JsValue> {
        let reader = ReplayReader::open(db_name).await.map_err(JsValue::from)?;

        Ok(Self {
            reader,
            width,
            height,
        })
    }

    /// Process the next frame.
    ///
    /// Returns `true` if there are more frames, `false` if replay is complete.
    pub async fn step(&mut self) -> Result<bool, JsValue> {
        // Get next frame from IndexedDB
        let bytes = match self.reader.next().await {
            Some(Ok(bytes)) => bytes,
            Some(Err(e)) => return Err(JsValue::from(e)),
            None => return Ok(false), // No more frames
        };

        // Parse the PDU to get the Action type
        let pdu_info = ironrdp_pdu::find_size(&bytes)
            .map_err(|e| JsValue::from_str(&format!("PDU parse error: {:?}", e)))?
            .ok_or_else(|| JsValue::from_str("Incomplete PDU"))?;

        // Log the frame info (for debugging)
        let action = pdu_info.action;
        web_sys::console::log_1(
            &format!(
                "Frame {}: action={:?}, size={}",
                self.reader.current_index() - 1, // Already incremented
                action,
                bytes.len()
            )
            .into(),
        );

        // TODO: Later, process with ActiveStage
        // let outputs = self.active_stage.process(&mut self.image, action, &bytes)?;

        Ok(true)
    }

    /// Reset replay to the beginning
    pub fn reset(&mut self) {
        self.reader.reset();
    }

    /// Get the current frame index
    #[wasm_bindgen(getter, js_name = "currentFrame")]
    pub fn current_frame(&self) -> u32 {
        self.reader.current_index()
    }

    /// Get the desktop width
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Get the desktop height
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u16 {
        self.height
    }
}
