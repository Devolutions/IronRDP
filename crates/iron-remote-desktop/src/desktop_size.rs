use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

#[wasm_bindgen]
impl DesktopSize {
    #[wasm_bindgen(constructor)]
    pub fn create(width: u16, height: u16) -> Self {
        DesktopSize { width, height }
    }
}
