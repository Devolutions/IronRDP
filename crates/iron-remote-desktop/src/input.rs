use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub enum RotationUnit {
    Pixel,
    Line,
    Page,
}

pub trait DeviceEvent {
    fn mouse_button_pressed(button: u8) -> Self;

    fn mouse_button_released(button: u8) -> Self;

    fn mouse_move(x: u16, y: u16) -> Self;

    fn wheel_rotations(vertical: bool, rotation_amount: i16, rotation_unit: RotationUnit) -> Self;

    fn key_pressed(scancode: u16) -> Self;

    fn key_released(scancode: u16) -> Self;

    fn unicode_pressed(unicode: char) -> Self;

    fn unicode_released(unicode: char) -> Self;
}

pub trait InputTransaction {
    type DeviceEvent: DeviceEvent;

    fn create() -> Self;

    fn add_event(&mut self, event: Self::DeviceEvent);
}
