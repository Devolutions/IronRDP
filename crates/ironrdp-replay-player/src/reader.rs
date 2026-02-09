use idb::{Database, Factory, TransactionMode};
use js_sys::Uint8Array;
use wasm_bindgen::JsValue;

use crate::error::ReplayError;

/// Reads RDP frames sequentially from IndexedDB
pub(crate) struct ReplayReader {
    db: Database,
    current_index: u32,
}

impl ReplayReader {
    const STORE_NAME: &'static str = "frames";

    /// Open the IndexedDB database
    pub(crate) async fn open(db_name: &str) -> Result<Self, ReplayError> {
        // Get the IndexedDB factory from the browser
        let factory = Factory::new()?;

        // Open database with version 1
        // Note: on_upgrade_needed would be called if DB doesn't exist,
        // but we expect the DB to already exist with recorded frames
        let db = factory.open(db_name, Some(1))?.await?;

        Ok(Self {
            db,
            current_index: 0,
        })
    }

    /// Get the next frame, or None if no more frames
    pub(crate) async fn next(&mut self) -> Option<Result<Vec<u8>, ReplayError>> {
        let result = self.read_frame(self.current_index).await;

        match result {
            Ok(Some(bytes)) => {
                self.current_index += 1;
                Some(Ok(bytes))
            }
            Ok(None) => None, // No more frames (key doesn't exist)
            Err(e) => Some(Err(e)),
        }
    }

    /// Reset to the beginning
    pub(crate) fn reset(&mut self) {
        self.current_index = 0;
    }

    /// Get the current frame index
    pub(crate) fn current_index(&self) -> u32 {
        self.current_index
    }

    /// Read a specific frame by index
    async fn read_frame(&self, index: u32) -> Result<Option<Vec<u8>>, ReplayError> {
        let tx = self
            .db
            .transaction(&[Self::STORE_NAME], TransactionMode::ReadOnly)?;
        let store = tx.object_store(Self::STORE_NAME)?;

        // Convert index to JsValue - idb expects JsValue directly for Query
        let key: JsValue = index.into();
        let value = store.get(key)?.await?;

        match value {
            Some(js_value) => {
                // Convert Uint8Array to Vec<u8>
                let uint8_array = Uint8Array::new(&js_value);
                let bytes = uint8_array.to_vec();
                Ok(Some(bytes))
            }
            None => Ok(None), // Key doesn't exist = end of frames
        }
    }
}
