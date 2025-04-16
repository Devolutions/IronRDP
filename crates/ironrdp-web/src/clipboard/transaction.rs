use iron_remote_desktop::{ClipboardContent, ClipboardTransaction};
use serde::Serialize;
use wasm_bindgen::JsValue;

/// Object which represents complete clipboard transaction with multiple MIME types.
#[derive(Debug, Default, Clone, Serialize)]
pub(crate) struct RdpClipboardTransaction {
    contents: Vec<RdpClipboardContent>,
}

impl RdpClipboardTransaction {
    pub(crate) fn contents(&self) -> &[RdpClipboardContent] {
        &self.contents
    }

    pub(crate) fn clear(&mut self) {
        self.contents.clear();
    }
}

impl ClipboardTransaction for RdpClipboardTransaction {
    type ClipboardContent = RdpClipboardContent;

    fn init() -> Self {
        Self { contents: Vec::new() }
    }

    fn add_content(&mut self, content: Self::ClipboardContent) {
        self.contents.push(content);
    }

    fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    fn js_contents(&self) -> js_sys::Array {
        js_sys::Array::from_iter(self.contents.iter().map(|content: &RdpClipboardContent| {
            serde_wasm_bindgen::to_value(&content).expect("Failed to convert clipboard transaction value into JsValue")
        }))
    }
}

impl FromIterator<RdpClipboardContent> for RdpClipboardTransaction {
    fn from_iter<T: IntoIterator<Item = RdpClipboardContent>>(iter: T) -> Self {
        Self {
            contents: iter.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) enum ClipboardContentValue {
    Text(String),
    Binary(Vec<u8>),
}

impl ClipboardContentValue {
    pub(crate) fn js_value(&self) -> JsValue {
        match self {
            ClipboardContentValue::Text(text) => JsValue::from_str(text),
            ClipboardContentValue::Binary(binary) => js_sys::Uint8Array::from(binary.as_slice()).into(),
        }
    }
}

/// Object which represents single clipboard format represented standard MIME type.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RdpClipboardContent {
    mime_type: String,
    value: ClipboardContentValue,
}

impl RdpClipboardContent {
    pub(crate) fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub(crate) fn value(&self) -> &ClipboardContentValue {
        &self.value
    }
}

impl ClipboardContent for RdpClipboardContent {
    fn new_text(mime_type: &str, text: &str) -> Self {
        Self {
            mime_type: mime_type.into(),
            value: ClipboardContentValue::Text(text.to_owned()),
        }
    }

    fn new_binary(mime_type: &str, binary: &[u8]) -> Self {
        Self {
            mime_type: mime_type.into(),
            value: ClipboardContentValue::Binary(binary.to_vec()),
        }
    }

    fn js_mime_type(&self) -> String {
        self.mime_type.clone()
    }

    fn js_value(&self) -> JsValue {
        self.value.js_value()
    }
}
