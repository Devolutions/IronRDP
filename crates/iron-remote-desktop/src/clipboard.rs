use crate::IronError;
use wasm_bindgen::JsValue;
use web_sys::js_sys;

pub trait ClipboardTransaction {
    type ClipboardContent: ClipboardContent;
    type Error: IronError;

    fn init() -> Self;
    fn add_content(&mut self, content: Self::ClipboardContent);
    fn is_empty(&self) -> bool;
    fn contents(&self) -> Result<js_sys::Array, Self::Error>;
}

pub trait ClipboardContent {
    fn new_text(mime_type: &str, text: &str) -> Self;
    fn new_binary(mime_type: &str, binary: &[u8]) -> Self;
    fn mime_type(&self) -> &str;
    fn value(&self) -> JsValue;
}
