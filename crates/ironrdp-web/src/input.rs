use ironrdp::input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use smallvec::SmallVec;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone)]
pub struct DeviceEvent(pub(crate) Operation);

#[wasm_bindgen]
impl DeviceEvent {
    pub fn mouse_button_pressed(button: u8) -> Self {
        match MouseButton::from_web_button(button) {
            Some(button) => Self(Operation::MouseButtonPressed(button)),
            None => {
                warn!("Unknown mouse button ID: {button}");
                Self(Operation::MouseButtonPressed(MouseButton::Left))
            }
        }
    }

    pub fn mouse_button_released(button: u8) -> Self {
        match MouseButton::from_web_button(button) {
            Some(button) => Self(Operation::MouseButtonReleased(button)),
            None => {
                warn!("Unknown mouse button ID: {button}");
                Self(Operation::MouseButtonReleased(MouseButton::Left))
            }
        }
    }

    pub fn mouse_move(x: u16, y: u16) -> Self {
        Self(Operation::MouseMove(MousePosition { x, y }))
    }

    pub fn wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
        Self(Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units,
        }))
    }

    pub fn key_pressed(scancode: u16) -> Self {
        Self(Operation::KeyPressed(Scancode::from_u16(scancode)))
    }

    pub fn key_released(scancode: u16) -> Self {
        Self(Operation::KeyReleased(Scancode::from_u16(scancode)))
    }

    pub fn unicode_pressed(unicode: char) -> Self {
        Self(Operation::UnicodeKeyPressed(unicode))
    }

    pub fn unicode_released(unicode: char) -> Self {
        Self(Operation::UnicodeKeyReleased(unicode))
    }
}

#[wasm_bindgen]
pub struct InputTransaction(pub(crate) SmallVec<[Operation; 3]>);

#[wasm_bindgen]
impl InputTransaction {
    pub fn init() -> Self {
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
