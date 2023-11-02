use wasm_bindgen::prelude::*;

/// Object which represents complete clipboard transaction with multiple MIME types.
#[wasm_bindgen]
#[derive(Debug, Default, Clone)]
pub struct ClipboardTransaction {
    contents: Vec<ClipboardContent>,
}

impl ClipboardTransaction {
    pub fn contents(&self) -> &[ClipboardContent] {
        &self.contents
    }

    pub fn clear(&mut self) {
        self.contents.clear();
    }
}

#[wasm_bindgen]
impl ClipboardTransaction {
    pub fn new() -> Self {
        Self { contents: Vec::new() }
    }

    pub fn add_content(&mut self, content: ClipboardContent) {
        self.contents.push(content);
    }

    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    #[wasm_bindgen(js_name = content)]
    pub fn js_contents(&self) -> js_sys::Array {
        js_sys::Array::from_iter(
            self.contents
                .iter()
                .map(|content: &ClipboardContent| JsValue::from(content.clone())),
        )
    }
}

impl FromIterator<ClipboardContent> for ClipboardTransaction {
    fn from_iter<T: IntoIterator<Item = ClipboardContent>>(iter: T) -> Self {
        Self {
            contents: iter.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ClipboardContentValue {
    Text(String),
    Binary(Vec<u8>),
}

impl ClipboardContentValue {
    pub fn js_value(&self) -> JsValue {
        match self {
            ClipboardContentValue::Text(text) => JsValue::from_str(text),
            ClipboardContentValue::Binary(binary) => js_sys::Uint8Array::from(binary.as_slice()).into(),
        }
    }
}

/// Object which represents single clipboard format represented standard MIME type.
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct ClipboardContent {
    mime_type: String,
    value: ClipboardContentValue,
}

#[wasm_bindgen]
impl ClipboardContent {
    pub fn new_text(mime_type: &str, text: &str) -> Self {
        Self {
            mime_type: mime_type.into(),
            value: ClipboardContentValue::Text(text.to_string()),
        }
    }

    pub fn new_binary(mime_type: &str, binary: &[u8]) -> Self {
        Self {
            mime_type: mime_type.into(),
            value: ClipboardContentValue::Binary(binary.to_vec()),
        }
    }

    #[wasm_bindgen(js_name = mime_type)]
    pub fn js_mime_type(&self) -> String {
        self.mime_type.clone()
    }

    #[wasm_bindgen(js_name = value)]
    pub fn js_value(&self) -> JsValue {
        self.value.js_value()
    }
}

impl ClipboardContent {
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub fn value(&self) -> &ClipboardContentValue {
        &self.value
    }
}
