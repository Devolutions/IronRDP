mod utils;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
async fn update_mouse(
    session_id: usize,
    mouse_x: u16,
    mouse_y: u16,
    left_click: bool,
) -> Result<(), String> {
    let _ = session_id; // TODO
    let _ = mouse_x; // TODO
    let _ = mouse_y; // TODO
    let _ = left_click; // TODO
    let _ = session_manager; // TODO

    Err("Unimplemented")
}

#[wasm_bindgen]
pub fn connect(username: String, password: String, address: String) -> Result<(), String> {
    let _ = username; // TODO
    let _ = address; // TODO

    if password == "abc" {
        Ok(())
    } else {
        Err("Something wrong happened".to_string())
    }
}

#[wasm_bindgen]
pub fn init() {
    utils::set_panic_hook();
}
