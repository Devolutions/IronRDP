use bitvec::array::BitArray;
use bitvec::BitArr;
use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::mouse::PointerFlags;
use ironrdp_pdu::input::mouse_x::PointerXFlags;
use ironrdp_pdu::input::{MousePdu, MouseXPdu};
use smallvec::SmallVec;

// TODO(#106): unicode keyboard event support

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    /// Typically Browser Back button
    X1 = 3,
    /// Typically Browser Forward button
    X2 = 4,
}

impl MouseButton {
    pub fn as_idx(self) -> usize {
        self as usize
    }

    pub fn from_idx(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::Left),
            1 => Some(Self::Middle),
            2 => Some(Self::Right),
            3 => Some(Self::X1),
            4 => Some(Self::X2),
            _ => None,
        }
    }

    pub fn from_web_button(value: u8) -> Option<Self> {
        // https://developer.mozilla.org/en-US/docs/Web/API/MouseEvent/button#value
        match value {
            0 => Some(Self::Left),
            1 => Some(Self::Middle),
            2 => Some(Self::Right),
            3 => Some(Self::X1),
            4 => Some(Self::X2),
            _ => None,
        }
    }

    pub fn from_native_button(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Left),
            2 => Some(Self::Middle),
            3 => Some(Self::Right),
            8 => Some(Self::X1),
            9 => Some(Self::X2),
            _ => None,
        }
    }
}

/// Keyboard scan code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scancode {
    code: u8,
    extended: bool,
}

impl Scancode {
    pub const fn from_u8(extended: bool, code: u8) -> Self {
        Self { code, extended }
    }

    pub const fn from_u16(scancode: u16) -> Self {
        let extended = scancode & 0xE000 == 0xE000;

        #[allow(clippy::cast_possible_truncation)] // truncating on purpose
        let code = scancode as u8;

        Self { code, extended }
    }

    pub fn as_idx(self) -> usize {
        if self.extended {
            usize::from(self.code).checked_add(256).expect("never overflow")
        } else {
            usize::from(self.code)
        }
    }

    pub fn as_u8(self) -> (bool, u8) {
        (self.extended, self.code)
    }

    pub fn as_u16(self) -> u16 {
        if self.extended {
            u16::from(self.code) | 0xE000
        } else {
            u16::from(self.code)
        }
    }
}

impl From<(bool, u8)> for Scancode {
    fn from((extended, code): (bool, u8)) -> Self {
        Self::from_u8(extended, code)
    }
}

impl From<u16> for Scancode {
    fn from(code: u16) -> Self {
        Self::from_u16(code)
    }
}

/// Cursor position for a mouse device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MousePosition {
    pub x: u16,
    pub y: u16,
}

/// Mouse wheel rotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WheelRotations {
    pub is_vertical: bool,
    pub rotation_units: i16,
}

#[derive(Debug, Clone)]
pub enum Operation {
    MouseButtonPressed(MouseButton),
    MouseButtonReleased(MouseButton),
    MouseMove(MousePosition),
    WheelRotations(WheelRotations),
    KeyPressed(Scancode),
    KeyReleased(Scancode),
}

pub type KeyboardState = BitArr!(for 512);
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

    pub fn is_key_pressed(&self, scancode: Scancode) -> bool {
        self.keyboard
            .get(scancode.as_idx())
            .as_deref()
            .copied()
            .unwrap_or(false)
    }

    pub fn is_mouse_button_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons
            .get(button.as_idx())
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
                    let was_pressed = self.mouse_buttons.replace(button.as_idx(), true);

                    if !was_pressed {
                        let event = match MouseButtonFlags::from(button) {
                            MouseButtonFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                                flags: PointerFlags::DOWN | flags,
                                number_of_wheel_rotation_units: 0,
                                x_position: self.mouse_position.x,
                                y_position: self.mouse_position.y,
                            }),
                            MouseButtonFlags::Pointer(flags) => FastPathInputEvent::MouseEventEx(MouseXPdu {
                                flags: PointerXFlags::DOWN | flags,
                                x_position: self.mouse_position.x,
                                y_position: self.mouse_position.y,
                            }),
                        };

                        events.push(event)
                    }
                }
                Operation::MouseButtonReleased(button) => {
                    let was_pressed = self.mouse_buttons.replace(button.as_idx(), false);

                    if was_pressed {
                        let event = match MouseButtonFlags::from(button) {
                            MouseButtonFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                                flags,
                                number_of_wheel_rotation_units: 0,
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
                            flags: PointerFlags::MOVE,
                            number_of_wheel_rotation_units: 0,
                            x_position: position.x,
                            y_position: position.y,
                        }))
                    }
                }
                Operation::WheelRotations(rotations) => events.push(FastPathInputEvent::MouseEvent(MousePdu {
                    flags: if rotations.is_vertical {
                        PointerFlags::VERTICAL_WHEEL
                    } else {
                        PointerFlags::HORIZONTAL_WHEEL
                    },
                    number_of_wheel_rotation_units: rotations.rotation_units,
                    x_position: self.mouse_position.x,
                    y_position: self.mouse_position.y,
                })),
                Operation::KeyPressed(scancode) => {
                    let was_pressed = self.keyboard.replace(scancode.as_idx(), true);

                    let mut flags = KeyboardFlags::empty();

                    if scancode.extended {
                        flags |= KeyboardFlags::EXTENDED
                    };

                    if was_pressed {
                        events.push(FastPathInputEvent::KeyboardEvent(
                            flags | KeyboardFlags::RELEASE,
                            scancode.code,
                        ));
                    }

                    events.push(FastPathInputEvent::KeyboardEvent(flags, scancode.code));
                }
                Operation::KeyReleased(scancode) => {
                    let was_pressed = self.keyboard.replace(scancode.as_idx(), false);

                    let mut flags = KeyboardFlags::RELEASE;

                    if scancode.extended {
                        flags |= KeyboardFlags::EXTENDED
                    };

                    if was_pressed {
                        events.push(FastPathInputEvent::KeyboardEvent(flags, scancode.code));
                    }
                }
            }
        }

        events
    }

    /// Releases all keys and buttons. Returns a list of RDP input events to send.
    pub fn release_all(&mut self) -> SmallVec<[FastPathInputEvent; 2]> {
        let mut events = SmallVec::new();

        for idx in self.mouse_buttons.iter_ones() {
            let button = MouseButton::from_idx(idx).expect("in-range index");

            let event = match MouseButtonFlags::from(button) {
                MouseButtonFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                    flags,
                    number_of_wheel_rotation_units: 0,
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

        for idx in self.keyboard.iter_ones() {
            let (scancode, extended) = if idx >= 256 {
                let extended_code = idx.checked_sub(256).expect("never underflow");
                (u8::try_from(extended_code).unwrap(), true)
            } else {
                (u8::try_from(idx).unwrap(), false)
            };

            let mut flags = KeyboardFlags::RELEASE;

            if extended {
                flags |= KeyboardFlags::EXTENDED
            };

            events.push(FastPathInputEvent::KeyboardEvent(flags, scancode));
        }

        self.mouse_buttons = BitArray::ZERO;
        self.keyboard = BitArray::ZERO;

        events
    }
}

/// Returns the RDP input event to send in order to synchronize lock keys.
pub fn synchronize_event(scroll_lock: bool, num_lock: bool, caps_lock: bool, kana_lock: bool) -> FastPathInputEvent {
    use ironrdp_pdu::input::fast_path::SynchronizeFlags;

    let mut flags = SynchronizeFlags::empty();

    if scroll_lock {
        flags |= SynchronizeFlags::SCROLL_LOCK;
    }

    if num_lock {
        flags |= SynchronizeFlags::NUM_LOCK;
    }

    if caps_lock {
        flags |= SynchronizeFlags::CAPS_LOCK;
    }

    if kana_lock {
        flags |= SynchronizeFlags::KANA_LOCK;
    }

    FastPathInputEvent::SyncEvent(flags)
}

enum MouseButtonFlags {
    Button(PointerFlags),
    Pointer(PointerXFlags),
}

impl From<MouseButton> for MouseButtonFlags {
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => Self::Button(PointerFlags::LEFT_BUTTON),
            MouseButton::Middle => Self::Button(PointerFlags::MIDDLE_BUTTON_OR_WHEEL),
            MouseButton::Right => Self::Button(PointerFlags::RIGHT_BUTTON),
            MouseButton::X1 => Self::Pointer(PointerXFlags::BUTTON1),
            MouseButton::X2 => Self::Pointer(PointerXFlags::BUTTON2),
        }
    }
}
