use anyhow::{bail, ensure};
use ironrdp_core::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp_input::*;
use proptest::collection::vec;
use proptest::prelude::*;

fn mouse_button() -> impl Strategy<Value = MouseButton> {
    prop_oneof![
        Just(MouseButton::Left),
        Just(MouseButton::Middle),
        Just(MouseButton::Right),
        Just(MouseButton::X1),
        Just(MouseButton::X2),
    ]
}

fn mouse_button_op() -> impl Strategy<Value = Operation> {
    prop_oneof![
        mouse_button().prop_map(Operation::MouseButtonPressed),
        mouse_button().prop_map(Operation::MouseButtonReleased),
    ]
}

fn scancode() -> impl Strategy<Value = Scancode> {
    (any::<bool>(), any::<u8>()).prop_map(Scancode::from)
}

fn key_op() -> impl Strategy<Value = Operation> {
    prop_oneof![
        scancode().prop_map(Operation::KeyPressed),
        scancode().prop_map(Operation::KeyReleased),
    ]
}

fn mouse_position() -> impl Strategy<Value = MousePosition> {
    (any::<u16>(), any::<u16>()).prop_map(|(x, y)| MousePosition { x, y })
}

#[test]
fn smoke_mouse_buttons() {
    let test_impl = |ops: Vec<Operation>| -> anyhow::Result<()> {
        let mut db = Database::default();

        for op in ops {
            db.apply(std::iter::once(op.clone()));

            match op {
                Operation::MouseButtonPressed(button) => {
                    ensure!(db.is_mouse_button_pressed(button))
                }
                Operation::MouseButtonReleased(button) => ensure!(!db.is_mouse_button_pressed(button)),
                _ => bail!("unexpected case"),
            }
        }

        Ok(())
    };

    proptest!(|(ops in vec(mouse_button_op(), 1..5))| {
        test_impl(ops).map_err(|e| TestCaseError::fail(format!("{e:#}")))?;
    });
}

#[test]
fn smoke_mouse_position() {
    let test_impl = |ops: Vec<MousePosition>| -> anyhow::Result<()> {
        let mut db = Database::default();

        db.apply(ops.iter().cloned().map(Operation::MouseMove));

        let last_position = ops.last().unwrap();
        ensure!(db.mouse_position().eq(last_position));

        Ok(())
    };

    proptest!(|(ops in vec(mouse_position(), 1..3))| {
        test_impl(ops).map_err(|e| TestCaseError::fail(format!("{e:#}")))?;
    });
}

#[test]
fn smoke_keyboard() {
    let test_impl = |ops: Vec<Operation>| -> anyhow::Result<()> {
        let mut db = Database::default();

        for op in ops {
            let packets = db.apply(std::iter::once(op.clone()));

            match op {
                Operation::KeyPressed(scancode) => {
                    ensure!(packets.len() <= 2);
                    ensure!(db.is_key_pressed(scancode));

                    let mut packets = packets.into_iter();

                    match (packets.next(), packets.next()) {
                        (None, None) => {}
                        (None, Some(_)) => unreachable!(),
                        (Some(pressed_packet), None) => {
                            if let FastPathInputEvent::KeyboardEvent(flags, scancode) = pressed_packet {
                                ensure!(!flags.contains(KeyboardFlags::RELEASE));
                                ensure!(scancode == u8::from(scancode))
                            } else {
                                bail!("unexpected packet emitted");
                            }
                        }
                        (Some(released_packet), Some(pressed_packet)) => {
                            if let FastPathInputEvent::KeyboardEvent(flags, scancode) = released_packet {
                                ensure!(flags.contains(KeyboardFlags::RELEASE));
                                ensure!(scancode == u8::from(scancode))
                            } else {
                                bail!("unexpected packet emitted");
                            }

                            if let FastPathInputEvent::KeyboardEvent(flags, scancode) = pressed_packet {
                                ensure!(!flags.contains(KeyboardFlags::RELEASE));
                                ensure!(scancode == u8::from(scancode))
                            } else {
                                bail!("unexpected packet emitted");
                            }
                        }
                    }
                }
                Operation::KeyReleased(scancode) => {
                    ensure!(packets.len() <= 1);
                    ensure!(!db.is_key_pressed(scancode));

                    let packet = packets.into_iter().next();

                    if let Some(packet) = packet {
                        if let FastPathInputEvent::KeyboardEvent(flags, scancode) = packet {
                            ensure!(flags.contains(KeyboardFlags::RELEASE));
                            ensure!(scancode == u8::from(scancode))
                        } else {
                            bail!("unexpected packet emitted");
                        }
                    }
                }
                _ => bail!("unexpected case"),
            }
        }

        Ok(())
    };

    proptest!(|(ops in vec(key_op(), 1..5))| {
        test_impl(ops).map_err(|e| TestCaseError::fail(format!("{e:#}")))?;
    });
}
