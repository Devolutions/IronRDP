use wasm_bindgen::JsValue;
use web_sys::js_sys;

pub trait ClipboardTransaction {
    type ClipboardContent: ClipboardContent;

    fn init() -> Self;
    fn add_content(&mut self, content: Self::ClipboardContent);
    fn is_empty(&self) -> bool;
    fn js_contents(&self) -> js_sys::Array;
}

pub trait ClipboardContent {
    fn new_text(mime_type: &str, text: &str) -> Self;
    fn new_binary(mime_type: &str, binary: &[u8]) -> Self;
    fn js_mime_type(&self) -> String;
    fn js_value(&self) -> JsValue;
}
