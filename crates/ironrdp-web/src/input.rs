use iron_remote_desktop::{DeviceEvent, InputTransaction};
use ironrdp::input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use smallvec::SmallVec;

#[derive(Clone)]
pub(crate) struct RdpDeviceEvent(pub(crate) Operation);

impl DeviceEvent for RdpDeviceEvent {
    fn mouse_button_pressed(button: u8) -> Self {
        match MouseButton::from_web_button(button) {
            Some(button) => Self(Operation::MouseButtonPressed(button)),
            None => {
                warn!("Unknown mouse button ID: {button}");
                Self(Operation::MouseButtonPressed(MouseButton::Left))
            }
        }
    }

    fn mouse_button_released(button: u8) -> Self {
        match MouseButton::from_web_button(button) {
            Some(button) => Self(Operation::MouseButtonReleased(button)),
            None => {
                warn!("Unknown mouse button ID: {button}");
                Self(Operation::MouseButtonReleased(MouseButton::Left))
            }
        }
    }

    fn mouse_move(x: u16, y: u16) -> Self {
        Self(Operation::MouseMove(MousePosition { x, y }))
    }

    fn wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
        Self(Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units,
        }))
    }

    fn key_pressed(scancode: u16) -> Self {
        Self(Operation::KeyPressed(Scancode::from_u16(scancode)))
    }

    fn key_released(scancode: u16) -> Self {
        Self(Operation::KeyReleased(Scancode::from_u16(scancode)))
    }

    fn unicode_pressed(unicode: char) -> Self {
        Self(Operation::UnicodeKeyPressed(unicode))
    }

    fn unicode_released(unicode: char) -> Self {
        Self(Operation::UnicodeKeyReleased(unicode))
    }
}

pub(crate) struct RdpInputTransaction(pub(crate) SmallVec<[Operation; 3]>);

impl InputTransaction for RdpInputTransaction {
    type DeviceEvent = RdpDeviceEvent;

    fn init() -> Self {
        Self(SmallVec::new())
    }

    fn add_event(&mut self, event: Self::DeviceEvent) {
        self.0.push(event.0);
    }
}

impl IntoIterator for RdpInputTransaction {
    type IntoIter = smallvec::IntoIter<[Operation; 3]>;
    type Item = Operation;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
