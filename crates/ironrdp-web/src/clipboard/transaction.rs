use crate::error::IronError;
use anyhow::anyhow;
use js_sys::{Object, Reflect};
use wasm_bindgen::JsValue;

/// Object which represents complete clipboard transaction with multiple MIME types.
#[derive(Debug, Default, Clone)]
pub(crate) struct ClipboardTransaction {
    contents: Vec<ClipboardContent>,
}

impl ClipboardTransaction {
    pub(crate) fn contents(&self) -> &[ClipboardContent] {
        &self.contents
    }

    pub(crate) fn clear(&mut self) {
        self.contents.clear();
    }

    pub(crate) fn to_js_value(&self) -> Result<JsValue, IronError> {
        let js_object = Object::new();

        Reflect::set(
            &js_object,
            &JsValue::from("contents"),
            &iron_remote_desktop::ClipboardTransaction::contents(self)
                .map_err(|e| anyhow!("{:?}", e))?
                .into(),
        )
        .map_err(|e| anyhow!("JS error: {:?}", e))?;

        Ok(js_object.into())
    }
}

impl iron_remote_desktop::ClipboardTransaction for ClipboardTransaction {
    type ClipboardContent = ClipboardContent;
    type Error = IronError;

    fn init() -> Self {
        Self { contents: Vec::new() }
    }

    fn add_content(&mut self, content: Self::ClipboardContent) {
        self.contents.push(content);
    }

    fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    fn contents(&self) -> Result<js_sys::Array, Self::Error> {
        Ok(js_sys::Array::from_iter(
            self.contents
                .iter()
                .map(|content| content.to_js_value())
                .collect::<Result<Vec<_>, Self::Error>>()?,
        ))
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
pub(crate) enum ClipboardContentValue {
    Text(String),
    Binary(Vec<u8>),
}

impl ClipboardContentValue {
    pub(crate) fn value(&self) -> JsValue {
        match self {
            ClipboardContentValue::Text(text) => JsValue::from_str(text),
            ClipboardContentValue::Binary(binary) => js_sys::Uint8Array::from(binary.as_slice()).into(),
        }
    }
}

/// Object which represents single clipboard format represented standard MIME type.
#[derive(Debug, Clone)]
pub(crate) struct ClipboardContent {
    mime_type: String,
    value: ClipboardContentValue,
}

impl ClipboardContent {
    pub(crate) fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub(crate) fn value(&self) -> &ClipboardContentValue {
        &self.value
    }

    fn to_js_value(&self) -> Result<JsValue, IronError> {
        let js_object = Object::new();

        Reflect::set(&js_object, &JsValue::from("mime_type"), &JsValue::from(&self.mime_type))
            .map_err(|e| anyhow!("JS error: {:?}", e))?;
        Reflect::set(&js_object, &JsValue::from("value"), &self.value.value())
            .map_err(|e| anyhow!("JS error: {:?}", e))?;

        Ok(js_object.into())
    }
}

impl iron_remote_desktop::ClipboardContent for ClipboardContent {
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

    fn mime_type(&self) -> &str {
        self.mime_type.as_str()
    }

    fn value(&self) -> JsValue {
        self.value.value()
    }
}
