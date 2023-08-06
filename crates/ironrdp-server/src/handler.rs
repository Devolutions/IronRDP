use ironrdp_pdu::input::{
    fast_path, mouse::PointerFlags, mouse_x::PointerXFlags, scan_code, unicode, MousePdu, MouseXPdu,
};

/// Keyboard Event
///
/// Describes a keyboard event received from the client
///
#[derive(Debug)]
pub enum KeyboardEvent {
    Pressed(u16),
    Released(u16),
}

/// Mouse Event
///
/// Describes a mouse event received from the client
///
#[derive(Debug)]
pub enum MouseEvent {
    Move { x: u16, y: u16 },
    RightPressed,
    RightReleased,
    LeftPressed,
    LeftReleased,
    Scroll,
}

/// Input Event Handler for an RDP server
///
/// Whenever the RDP server will receive an input event from a client, the relevent callback from
/// this handler will be called
///
/// # Example
///
/// ```
/// pub struct InputHandler;
///
/// #[async_trait::async_trait]
/// impl RdpServerInputHandler for InputHandler {
///     async fn keyboard(&mut self, event: KeyboardEvent) {
///         match event {
///             KeyboardEvent::Pressed(code) => println!("Pressed {}", code)
///             KeyboardEvent::Released(code) => println!("Released {}", code)
///         };
///     }
///
///     async fn mouse(&mut self, event: MouseEvent) {
///         let result = match event {
///             MouseEvent::Move { x, y } => println!("Moved mouse to {} {}", x, y)
///             _ => unimplemented!()
///         };
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait RdpServerInputHandler {
    async fn keyboard(&mut self, event: KeyboardEvent);
    async fn mouse(&mut self, event: MouseEvent);
}

impl From<(u16, fast_path::KeyboardFlags)> for KeyboardEvent {
    fn from((key, flags): (u16, fast_path::KeyboardFlags)) -> Self {
        if flags.contains(fast_path::KeyboardFlags::RELEASE) {
            KeyboardEvent::Released(key)
        } else {
            KeyboardEvent::Pressed(key)
        }
    }
}

impl From<(u16, scan_code::KeyboardFlags)> for KeyboardEvent {
    fn from((key, flags): (u16, scan_code::KeyboardFlags)) -> Self {
        if flags.contains(scan_code::KeyboardFlags::RELEASE) {
            KeyboardEvent::Released(key)
        } else {
            KeyboardEvent::Pressed(key)
        }
    }
}

impl From<(u16, unicode::KeyboardFlags)> for KeyboardEvent {
    fn from((key, flags): (u16, unicode::KeyboardFlags)) -> Self {
        if flags.contains(unicode::KeyboardFlags::RELEASE) {
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
