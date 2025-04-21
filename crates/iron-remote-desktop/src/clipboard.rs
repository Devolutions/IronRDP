use wasm_bindgen::JsValue;

pub trait ClipboardData {
    type Item: ClipboardItem;

    fn init() -> Self;
    fn add_text(&mut self, mime_type: &str, text: &str);
    fn add_binary(&mut self, mime_type: &str, binary: &[u8]);
    fn items(&self) -> &[Self::Item];
}

pub trait ClipboardItem {
    fn mime_type(&self) -> &str;
    fn value(&self) -> impl Into<JsValue>;
}
