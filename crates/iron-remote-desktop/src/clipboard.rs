use wasm_bindgen::JsValue;

pub trait ClipboardData {
    type Item: ClipboardItem;

    fn create() -> Self;

    fn add_text(&mut self, mime_type: &str, text: &str);

    fn add_binary(&mut self, mime_type: &str, binary: &[u8]);

    fn items(&self) -> &[Self::Item];

    fn is_empty(&self) -> bool {
        self.items().is_empty()
    }
}

pub trait ClipboardItem {
    fn mime_type(&self) -> &str;

    fn value(&self) -> impl Into<JsValue>;
}
