use wasm_bindgen::prelude::*;

/// Error type for the replay player
pub struct ReplayError {
    message: String,
}

impl ReplayError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl core::fmt::Debug for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ReplayError: {}", self.message)
    }
}

// Convert to JsValue for WASM boundary
impl From<ReplayError> for JsValue {
    fn from(error: ReplayError) -> Self {
        JsValue::from_str(&error.message)
    }
}

// Convert from idb errors
impl From<idb::Error> for ReplayError {
    fn from(error: idb::Error) -> Self {
        Self::new(format!("IndexedDB error: {:?}", error))
    }
}
