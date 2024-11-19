#[diplomat::bridge]
pub mod ffi {
    use crate::{error::ffi::IronRdpError, pdu::ffi::FastPathInputEventIterator};

    #[diplomat::opaque]
    pub struct InputDatabase(pub ironrdp::input::Database);

    impl InputDatabase {
        pub fn new() -> Box<InputDatabase> {
            Box::new(InputDatabase(ironrdp::input::Database::new()))
        }

        pub fn apply(&mut self, operation: &Operation) -> Box<FastPathInputEventIterator> {
            let res = self.0.apply(core::iter::once(operation.0.clone()));
            Box::new(res.to_vec().into())
        }
    }

    #[diplomat::opaque]
    pub struct Operation(pub ironrdp::input::Operation);

    pub enum OperationType {
        MouseButtonPressed,
        MouseButtonReleased,
        MouseMove,
        WheelRotations,
        KeyPressed,
        KeyReleased,
        UnicodeKeyPressed,
        UnicodeKeyReleased,
    }

    #[diplomat::opaque]
    pub struct MousePosition(pub ironrdp::input::MousePosition);

    impl MousePosition {
        pub fn new(x: u16, y: u16) -> Box<MousePosition> {
            Box::new(MousePosition(ironrdp::input::MousePosition { x, y }))
        }

        pub fn as_move_operation(&self) -> Box<Operation> {
            Box::new(Operation(ironrdp::input::Operation::MouseMove(self.0)))
        }
    }

    #[diplomat::opaque]
    pub struct MouseButton(pub ironrdp::input::MouseButton);

    #[diplomat::enum_convert(ironrdp::input::MouseButton)]
    pub enum MouseButtonType {
        Left = 0,
        Middle = 1,
        Right = 2,
        X1 = 3,
        X2 = 4,
    }

    impl MouseButton {
        pub fn new(button: MouseButtonType) -> Box<MouseButton> {
            Box::new(MouseButton(button.into()))
        }

        pub fn as_operation_mouse_button_pressed(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::MouseButtonPressed(self.0);
            Box::new(Operation(operation))
        }

        pub fn as_operation_mouse_button_released(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::MouseButtonReleased(self.0);
            Box::new(Operation(operation))
        }
    }

    #[diplomat::opaque]
    pub struct WheelRotations(pub ironrdp::input::WheelRotations);

    impl WheelRotations {
        pub fn new(is_vertical: bool, rotation_units: i16) -> Box<WheelRotations> {
            Box::new(WheelRotations(ironrdp::input::WheelRotations {
                is_vertical,
                rotation_units,
            }))
        }

        pub fn as_operation(&self) -> Box<Operation> {
            Box::new(Operation(ironrdp::input::Operation::WheelRotations(self.0)))
        }
    }

    #[diplomat::opaque]
    pub struct Scancode(pub ironrdp::input::Scancode);

    impl Scancode {
        pub fn from_u8(extended: bool, code: u8) -> Box<Scancode> {
            Box::new(Scancode(ironrdp::input::Scancode::from_u8(extended, code)))
        }

        pub fn from_u16(code: u16) -> Box<Scancode> {
            Box::new(Scancode(ironrdp::input::Scancode::from_u16(code)))
        }

        pub fn as_operation_key_pressed(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::KeyPressed(self.0);
            Box::new(Operation(operation))
        }

        pub fn as_operation_key_released(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::KeyReleased(self.0);
            Box::new(Operation(operation))
        }
    }

    #[diplomat::opaque]
    pub struct Char(pub char);

    impl Char {
        pub fn new(c: u32) -> Result<Box<Char>, Box<IronRdpError>> {
            char::from_u32(c)
                .map(|c| Box::new(Char(c)))
                .ok_or_else(|| "Invalid unicode character".into())
        }

        pub fn as_operation_unicode_key_pressed(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::UnicodeKeyPressed(self.0);
            Box::new(Operation(operation))
        }

        pub fn as_operation_unicode_key_released(&self) -> Box<Operation> {
            let operation = ironrdp::input::Operation::UnicodeKeyReleased(self.0);
            Box::new(Operation(operation))
        }
    }
}
