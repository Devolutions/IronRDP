use ironrdp_input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use smallvec::SmallVec;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone)]
pub struct DeviceEvent(pub(crate) Operation);

#[wasm_bindgen]
impl DeviceEvent {
    pub fn new_mouse_button_pressed(button: u8) -> Self {
        Self(Operation::MouseButtonPressed(MouseButton::from(button)))
    }

    pub fn new_mouse_button_released(button: u8) -> Self {
        Self(Operation::MouseButtonReleased(MouseButton::from(button)))
    }

    pub fn new_mouse_move(x: u16, y: u16) -> Self {
        Self(Operation::MouseMove(MousePosition { x, y }))
    }

    pub fn new_wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
        Self(Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units,
        }))
    }

    pub fn new_key_pressed(scancode: u16) -> Self {
        Self(Operation::KeyPressed(Scancode::from(scancode)))
    }

    pub fn new_key_released(scancode: u16) -> Self {
        Self(Operation::KeyReleased(Scancode::from(scancode)))
    }
}

#[wasm_bindgen]
pub struct InputTransaction(pub(crate) SmallVec<[Operation; 3]>);

#[wasm_bindgen]
impl InputTransaction {
    pub fn new() -> Self {
        Self(SmallVec::new())
    }

    pub fn add_event(&mut self, event: DeviceEvent) {
        self.0.push(event.0);
    }
}

impl IntoIterator for InputTransaction {
    type IntoIter = smallvec::IntoIter<[Operation; 3]>;
    type Item = Operation;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
