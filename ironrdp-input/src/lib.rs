use bitvec::array::BitArray;
use bitvec::BitArr;
use ironrdp_core::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp_core::input::mouse::{ButtonEvents, MovementEvents, WheelEvents};
use ironrdp_core::input::mouse_x::PointerFlags;
use ironrdp_core::input::{MousePdu, MouseXPdu};
use smallvec::SmallVec;

// TODO: sync event
// TODO: mouse wheel

/// Number associated to a mouse button.
///
/// Based on the MouseEvent.button property found in browsers APIs:
/// https://developer.mozilla.org/en-US/docs/Web/API/MouseEvent/button#value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MouseButton(u8);

impl MouseButton {
    const LEFT_VAL: u8 = 0;
    const MIDDLE_VAL: u8 = 1;
    const RIGHT_VAL: u8 = 2;
    const X1_VAL: u8 = 3;
    const X2_VAL: u8 = 4;

    pub const LEFT: Self = Self(Self::LEFT_VAL);
    pub const MIDDLE: Self = Self(Self::MIDDLE_VAL);
    pub const RIGHT: Self = Self(Self::RIGHT_VAL);
    pub const X1: Self = Self(Self::X1_VAL);
    pub const X2: Self = Self(Self::X2_VAL);

    pub fn is_unknown(&self) -> bool {
        self.0 > Self::X2.0
    }
}

impl From<u8> for MouseButton {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<MouseButton> for u8 {
    fn from(value: MouseButton) -> Self {
        value.0
    }
}

impl From<MouseButton> for usize {
    fn from(value: MouseButton) -> Self {
        usize::from(value.0)
    }
}

/// Keyboard scan code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scancode(u8);

impl From<u8> for Scancode {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<Scancode> for u8 {
    fn from(value: Scancode) -> Self {
        value.0
    }
}

impl From<Scancode> for usize {
    fn from(value: Scancode) -> Self {
        usize::from(value.0)
    }
}

/// Cursor position for a mouse device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MousePosition {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone)]
pub enum Operation {
    MouseButtonPressed(MouseButton),
    MouseButtonReleased(MouseButton),
    MouseMove(MousePosition),
    KeyPressed(Scancode),
    KeyReleased(Scancode),
}

pub type KeyboardState = BitArr!(for 256);
pub type MouseButtonsState = BitArr!(for 5);

/// In-memory database for maintaining the current keyboard and mouse state.
pub struct Database {
    keyboard: KeyboardState,
    mouse_buttons: MouseButtonsState,
    mouse_position: MousePosition,
}

impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}

impl Database {
    pub fn new() -> Self {
        Self {
            keyboard: BitArray::ZERO,
            mouse_buttons: BitArray::ZERO,
            mouse_position: MousePosition { x: 0, y: 0 },
        }
    }

    pub fn is_key_pressed(&self, code: Scancode) -> bool {
        self.keyboard
            .get(usize::from(code))
            .as_deref()
            .copied()
            .unwrap_or(false)
    }

    pub fn is_mouse_button_pressed(&self, code: MouseButton) -> bool {
        self.mouse_buttons
            .get(usize::from(code))
            .as_deref()
            .copied()
            .unwrap_or(false)
    }

    pub fn mouse_position(&self) -> MousePosition {
        self.mouse_position
    }

    pub fn keyboard_state(&self) -> &KeyboardState {
        &self.keyboard
    }

    pub fn mouse_buttons_state(&self) -> &MouseButtonsState {
        &self.mouse_buttons
    }

    /// Apply a transaction (list of operations) and returns a list of RDP input events to send.
    ///
    /// Operations that would cause no state change are ignored.
    pub fn apply(&mut self, transaction: impl IntoIterator<Item = Operation>) -> SmallVec<[FastPathInputEvent; 2]> {
        let mut events = SmallVec::new();

        for operation in transaction {
            match operation {
                Operation::MouseButtonPressed(button) => {
                    if button.is_unknown() {
                        continue;
                    }

                    let was_pressed = self.mouse_buttons.replace(usize::from(button), true);

                    if !was_pressed {
                        let event = match MouseButtonFlags::from(button) {
                            MouseButtonFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                                wheel_events: WheelEvents::empty(),
                                movement_events: MovementEvents::empty(),
                                button_events: ButtonEvents::DOWN | flags,
                                number_of_wheel_rotations: 0,
                                x_position: self.mouse_position.x,
                                y_position: self.mouse_position.y,
                            }),
                            MouseButtonFlags::Pointer(flags) => FastPathInputEvent::MouseEventEx(MouseXPdu {
                                flags: PointerFlags::DOWN | flags,
                                x_position: self.mouse_position.x,
                                y_position: self.mouse_position.y,
                            }),
                        };

                        events.push(event)
                    }
                }
                Operation::MouseButtonReleased(button) => {
                    if button.is_unknown() {
                        continue;
                    }

                    let was_pressed = self.mouse_buttons.replace(usize::from(button), false);

                    if was_pressed {
                        let event = match MouseButtonFlags::from(button) {
                            MouseButtonFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                                wheel_events: WheelEvents::empty(),
                                movement_events: MovementEvents::empty(),
                                button_events: flags,
                                number_of_wheel_rotations: 0,
                                x_position: self.mouse_position.x,
                                y_position: self.mouse_position.y,
                            }),
                            MouseButtonFlags::Pointer(flags) => FastPathInputEvent::MouseEventEx(MouseXPdu {
                                flags,
                                x_position: self.mouse_position.x,
                                y_position: self.mouse_position.y,
                            }),
                        };

                        events.push(event)
                    }
                }
                Operation::MouseMove(position) => {
                    if position != self.mouse_position {
                        self.mouse_position = position;
                        events.push(FastPathInputEvent::MouseEvent(MousePdu {
                            wheel_events: WheelEvents::empty(),
                            movement_events: MovementEvents::MOVE,
                            button_events: ButtonEvents::empty(),
                            number_of_wheel_rotations: 0,
                            x_position: position.x,
                            y_position: position.y,
                        }))
                    }
                }
                Operation::KeyPressed(scancode) => {
                    let was_pressed = self.keyboard.replace(usize::from(scancode), true);

                    if !was_pressed {
                        events.push(FastPathInputEvent::KeyboardEvent(
                            KeyboardFlags::empty(),
                            u8::from(scancode),
                        ));
                    }
                }
                Operation::KeyReleased(scancode) => {
                    let was_pressed = self.keyboard.replace(usize::from(scancode), false);

                    if was_pressed {
                        events.push(FastPathInputEvent::KeyboardEvent(
                            KeyboardFlags::FASTPATH_INPUT_KBDFLAGS_RELEASE,
                            u8::from(scancode),
                        ));
                    }
                }
            }
        }

        events
    }
}

enum MouseButtonFlags {
    Button(ButtonEvents),
    Pointer(PointerFlags),
}

impl From<MouseButton> for MouseButtonFlags {
    fn from(value: MouseButton) -> Self {
        match value.0 {
            MouseButton::LEFT_VAL => Self::Button(ButtonEvents::LEFT_BUTTON),
            MouseButton::MIDDLE_VAL => Self::Button(ButtonEvents::MIDDLE_BUTTON_OR_WHEEL),
            MouseButton::RIGHT_VAL => Self::Button(ButtonEvents::RIGHT_BUTTON),
            MouseButton::X1_VAL => Self::Pointer(PointerFlags::BUTTON1),
            MouseButton::X2_VAL => Self::Pointer(PointerFlags::BUTTON2),
            _ => Self::Button(ButtonEvents::empty()),
        }
    }
}
