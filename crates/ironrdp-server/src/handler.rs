use ironrdp_pdu::input::{fast_path::KeyboardFlags, mouse::PointerFlags, mouse_x::PointerXFlags, MousePdu, MouseXPdu};

#[derive(Debug)]
pub enum KeyboardEvent {
    Pressed(u16),
    Released(u16),
}

#[derive(Debug)]
pub enum MouseEvent {
    Move { x: u16, y: u16 },
    RightPressed,
    RightReleased,
    LeftPressed,
    LeftReleased,
    Scroll,
}

#[async_trait::async_trait]
pub trait RdpServerInputHandler {
    async fn keyboard(&mut self, event: KeyboardEvent);
    async fn mouse(&mut self, event: MouseEvent);
}

impl From<(u16, KeyboardFlags)> for KeyboardEvent {
    fn from((key, flags): (u16, KeyboardFlags)) -> Self {
        if flags.contains(KeyboardFlags::RELEASE) {
            KeyboardEvent::Released(key)
        } else {
            KeyboardEvent::Pressed(key)
        }
    }
}

impl From<MousePdu> for MouseEvent {
    fn from(value: MousePdu) -> Self {
        if value.flags.contains(PointerFlags::LEFT_BUTTON) {
            if value.flags.contains(PointerFlags::DOWN) {
                MouseEvent::LeftPressed
            } else {
                MouseEvent::LeftReleased
            }
        } else if value.flags.contains(PointerFlags::RIGHT_BUTTON) {
            if value.flags.contains(PointerFlags::DOWN) {
                MouseEvent::RightPressed
            } else {
                MouseEvent::RightReleased
            }
        } else {
            MouseEvent::Move {
                x: value.x_position,
                y: value.y_position,
            }
        }
    }
}

impl From<MouseXPdu> for MouseEvent {
    fn from(value: MouseXPdu) -> Self {
        if value.flags.contains(PointerXFlags::BUTTON1) {
            if value.flags.contains(PointerXFlags::DOWN) {
                MouseEvent::LeftPressed
            } else {
                MouseEvent::LeftReleased
            }
        } else if value.flags.contains(PointerXFlags::BUTTON2) {
            if value.flags.contains(PointerXFlags::DOWN) {
                MouseEvent::RightPressed
            } else {
                MouseEvent::RightReleased
            }
        } else {
            MouseEvent::Move {
                x: value.x_position,
                y: value.y_position,
            }
        }
    }
}
