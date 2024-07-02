#![allow(clippy::print_stderr, clippy::print_stdout)] // allowed in this module only

use std::num::NonZeroU32;

use anyhow::Context as _;
use tokio::sync::mpsc;
use winit::dpi::LogicalPosition;
use winit::event::{self, Event, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use winit::window::{Window, WindowBuilder};

use crate::rdp::{RdpInputEvent, RdpOutputEvent};

pub struct GuiContext {
    window: Window,
    event_loop: EventLoop<RdpOutputEvent>,
    context: softbuffer::Context,
}

impl GuiContext {
    pub fn init() -> anyhow::Result<Self> {
        let event_loop = EventLoopBuilder::<RdpOutputEvent>::with_user_event().build();

        let window = WindowBuilder::new()
            .with_title("IronRDP")
            .build(&event_loop)
            .context("unable to create winit Window")?;

        // SAFETY: both the context and the window are held by the GuiContext
        let context = unsafe { softbuffer::Context::new(&window) }
            .map_err(|e| anyhow::Error::msg(format!("unable to initialize softbuffer context: {e}")))?;

        Ok(Self {
            window,
            event_loop,
            context,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn create_event_proxy(&self) -> EventLoopProxy<RdpOutputEvent> {
        self.event_loop.create_proxy()
    }

    pub fn run(self, input_event_sender: mpsc::UnboundedSender<RdpInputEvent>) -> ! {
        let Self {
            window,
            event_loop,
            context,
        } = self;

        // SAFETY: both the context and the window are kept alive until the end of this function’s scope
        let mut surface = unsafe { softbuffer::Surface::new(&context, &window) }.expect("surface");

        let mut input_database = ironrdp::input::Database::new();

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;

            match event {
                Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                    WindowEvent::Resized(size) => {
                        let scale_factor = (window.scale_factor() * 100.0) as u32;

                        let _ = input_event_sender.send(RdpInputEvent::Resize {
                            width: u16::try_from(size.width).unwrap(),
                            height: u16::try_from(size.height).unwrap(),
                            scale_factor,
                            // TODO: it should be possible to get the physical size here, however winit doesn't make it straightforward.
                            // FreeRDP does it based on DPI reading grabbed via [`SDL_GetDisplayDPI`](https://wiki.libsdl.org/SDL2/SDL_GetDisplayDPI):
                            // https://github.com/FreeRDP/FreeRDP/blob/ba8cf8cf2158018fb7abbedb51ab245f369be813/client/SDL/sdl_monitor.cpp#L250-L262
                            physical_size: None,
                        });
                    }
                    WindowEvent::CloseRequested => {
                        if input_event_sender.send(RdpInputEvent::Close).is_err() {
                            error!("Failed to send graceful shutdown event, closing the window");
                            control_flow.set_exit();
                        }
                    }
                    WindowEvent::DroppedFile(_) => {
                        // TODO(#110): File upload
                    }
                    WindowEvent::ReceivedCharacter(_) => {
                        // Sadly, we can't use this winit event to send RDP unicode events because
                        // of the several reasons:
                        // 1. `ReceivedCharacter` event doesn't provide a way to distinguish between
                        //    key press and key release, therefore the only way to use it is to send
                        //    a key press + release events sequentially, which will not allow to
                        //    handle long press and key repeat events.
                        // 2. This event do not fire for non-printable keys (e.g. Control, Alt, etc.)
                        // 3. This event fies BEFORE `KeyboardInput` event, so we can't make a
                        //    reasonable workaround for `1` and `2` by collecting physical key press
                        //    information first via `KeyboardInput` before processing `ReceivedCharacter`.
                        //
                        // However, all of these issues can be solved by updating `winit` to the
                        // newer version.
                        //
                        // TODO(#376): Update winit
                        // TODO(#376): Implement unicode input in native client
                    }
                    WindowEvent::KeyboardInput { input, .. } => {
                        let scancode = if let Some(virtual_keycode) = input.virtual_keycode {
                            ironrdp::input::Scancode::from_u16(to_scancode(virtual_keycode))
                        } else {
                            ironrdp::input::Scancode::from_u16(u16::try_from(input.scancode).unwrap())
                        };
                        let operation = match input.state {
                            event::ElementState::Pressed => ironrdp::input::Operation::KeyPressed(scancode),
                            event::ElementState::Released => ironrdp::input::Operation::KeyReleased(scancode),
                        };

                        let input_events = input_database.apply(std::iter::once(operation));

                        send_fast_path_events(&input_event_sender, input_events);
                    }
                    WindowEvent::ModifiersChanged(state) => {
                        const SHIFT_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(false, 0x2A);
                        const CONTROL_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(false, 0x1D);
                        const ALT_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(false, 0x38);
                        const LOGO_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(true, 0x5B);

                        let mut operations = smallvec::SmallVec::<[ironrdp::input::Operation; 4]>::new();

                        let mut add_operation = |pressed: bool, scancode: ironrdp::input::Scancode| {
                            let operation = if pressed {
                                ironrdp::input::Operation::KeyPressed(scancode)
                            } else {
                                ironrdp::input::Operation::KeyReleased(scancode)
                            };
                            operations.push(operation);
                        };

                        add_operation(state.shift(), SHIFT_LEFT);
                        add_operation(state.ctrl(), CONTROL_LEFT);
                        add_operation(state.alt(), ALT_LEFT);
                        add_operation(state.logo(), LOGO_LEFT);

                        let input_events = input_database.apply(operations);

                        send_fast_path_events(&input_event_sender, input_events);
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        // FIXME: allow physical position for HiDPI remote
                        // + should take display scale into account
                        let sf = window.scale_factor();
                        let operation = ironrdp::input::Operation::MouseMove(ironrdp::input::MousePosition {
                            x: (position.x / sf) as u16,
                            y: (position.y / sf) as u16,
                        });

                        let input_events = input_database.apply(std::iter::once(operation));

                        send_fast_path_events(&input_event_sender, input_events);
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let mut operations = smallvec::SmallVec::<[ironrdp::input::Operation; 2]>::new();

                        match delta {
                            event::MouseScrollDelta::LineDelta(delta_x, delta_y) => {
                                if delta_x.abs() > 0.001 {
                                    operations.push(ironrdp::input::Operation::WheelRotations(
                                        ironrdp::input::WheelRotations {
                                            is_vertical: false,
                                            rotation_units: (delta_x * 100.) as i16,
                                        },
                                    ));
                                }

                                if delta_y.abs() > 0.001 {
                                    operations.push(ironrdp::input::Operation::WheelRotations(
                                        ironrdp::input::WheelRotations {
                                            is_vertical: true,
                                            rotation_units: (delta_y * 100.) as i16,
                                        },
                                    ));
                                }
                            }
                            event::MouseScrollDelta::PixelDelta(delta) => {
                                if delta.x.abs() > 0.001 {
                                    operations.push(ironrdp::input::Operation::WheelRotations(
                                        ironrdp::input::WheelRotations {
                                            is_vertical: false,
                                            rotation_units: delta.x as i16,
                                        },
                                    ));
                                }

                                if delta.y.abs() > 0.001 {
                                    operations.push(ironrdp::input::Operation::WheelRotations(
                                        ironrdp::input::WheelRotations {
                                            is_vertical: true,
                                            rotation_units: delta.y as i16,
                                        },
                                    ));
                                }
                            }
                        };

                        let input_events = input_database.apply(operations);

                        send_fast_path_events(&input_event_sender, input_events);
                    }
                    WindowEvent::MouseInput { state, button, .. } => {
                        let mouse_button = match button {
                            event::MouseButton::Left => ironrdp::input::MouseButton::Left,
                            event::MouseButton::Right => ironrdp::input::MouseButton::Right,
                            event::MouseButton::Middle => ironrdp::input::MouseButton::Middle,
                            event::MouseButton::Other(native_button) => {
                                if let Some(button) = ironrdp::input::MouseButton::from_native_button(native_button) {
                                    button
                                } else {
                                    return;
                                }
                            }
                        };

                        let operation = match state {
                            event::ElementState::Pressed => ironrdp::input::Operation::MouseButtonPressed(mouse_button),
                            event::ElementState::Released => {
                                ironrdp::input::Operation::MouseButtonReleased(mouse_button)
                            }
                        };

                        let input_events = input_database.apply(std::iter::once(operation));

                        send_fast_path_events(&input_event_sender, input_events);
                    }
                    _ => {}
                },
                Event::RedrawRequested(window_id) if window_id == window.id() => {
                    // TODO: is there something we should handle here?
                }
                Event::UserEvent(RdpOutputEvent::Image { buffer, width, height }) => {
                    trace!(width = ?width, height = ?height, "Received image with size");
                    trace!(window_physical_size = ?window.inner_size(), "Drawing image to the window with size");
                    surface
                        .resize(
                            NonZeroU32::new(u32::from(width)).unwrap(),
                            NonZeroU32::new(u32::from(height)).unwrap(),
                        )
                        .expect("surface resize");

                    let mut sb_buffer = surface.buffer_mut().expect("surface buffer");
                    sb_buffer.copy_from_slice(buffer.as_slice());
                    sb_buffer.present().expect("buffer present");
                }
                Event::UserEvent(RdpOutputEvent::ConnectionFailure(error)) => {
                    error!(?error);
                    eprintln!("Connection error: {}", error.report());
                    control_flow.set_exit_with_code(proc_exit::sysexits::PROTOCOL_ERR.as_raw());
                }
                Event::UserEvent(RdpOutputEvent::Terminated(result)) => {
                    let exit_code = match result {
                        Ok(reason) => {
                            println!("Terminated gracefully: {reason}");
                            proc_exit::sysexits::OK
                        }
                        Err(error) => {
                            error!(?error);
                            eprintln!("Active session error: {}", error.report());
                            proc_exit::sysexits::PROTOCOL_ERR
                        }
                    };

                    control_flow.set_exit_with_code(exit_code.as_raw());
                }
                Event::UserEvent(RdpOutputEvent::PointerHidden) => {
                    window.set_cursor_visible(false);
                }
                Event::UserEvent(RdpOutputEvent::PointerDefault) => {
                    window.set_cursor_visible(true);
                }
                Event::UserEvent(RdpOutputEvent::PointerPosition { x, y }) => {
                    if let Err(error) = window.set_cursor_position(LogicalPosition::new(x, y)) {
                        error!(?error, "Failed to set cursor position");
                    }
                }
                Event::LoopDestroyed => {
                    let _ = input_event_sender.send(RdpInputEvent::Close);
                }
                _ => {}
            }

            if input_event_sender.is_closed() {
                control_flow.set_exit();
            }
        })
    }
}

fn send_fast_path_events(
    input_event_sender: &mpsc::UnboundedSender<RdpInputEvent>,
    input_events: smallvec::SmallVec<[ironrdp::pdu::input::fast_path::FastPathInputEvent; 2]>,
) {
    if !input_events.is_empty() {
        let _ = input_event_sender.send(RdpInputEvent::FastPath(input_events));
    }
}

fn to_scancode(key: VirtualKeyCode) -> u16 {
    match key {
        VirtualKeyCode::Escape => 0x0001,
        VirtualKeyCode::Key1 => 0x0002,
        VirtualKeyCode::Key2 => 0x0003,
        VirtualKeyCode::Key3 => 0x0004,
        VirtualKeyCode::Key4 => 0x0005,
        VirtualKeyCode::Key5 => 0x0006,
        VirtualKeyCode::Key6 => 0x0007,
        VirtualKeyCode::Key7 => 0x0008,
        VirtualKeyCode::Key8 => 0x0009,
        VirtualKeyCode::Key9 => 0x000A,
        VirtualKeyCode::Key0 => 0x000B,
        VirtualKeyCode::Minus => 0x000C,
        VirtualKeyCode::Equals => 0x000D,
        VirtualKeyCode::Back => 0x000E,
        VirtualKeyCode::Tab => 0x000F,
        VirtualKeyCode::Q => 0x0010,
        VirtualKeyCode::W => 0x0011,
        VirtualKeyCode::E => 0x0012,
        VirtualKeyCode::R => 0x0013,
        VirtualKeyCode::T => 0x0014,
        VirtualKeyCode::Y => 0x0015,
        VirtualKeyCode::U => 0x0016,
        VirtualKeyCode::I => 0x0017,
        VirtualKeyCode::O => 0x0018,
        VirtualKeyCode::P => 0x0019,
        VirtualKeyCode::LBracket => 0x001A,
        VirtualKeyCode::RBracket => 0x001B,
        VirtualKeyCode::Return => 0x001C,
        VirtualKeyCode::LControl => 0x001D,
        VirtualKeyCode::A => 0x001E,
        VirtualKeyCode::S => 0x001F,
        VirtualKeyCode::D => 0x0020,
        VirtualKeyCode::F => 0x0021,
        VirtualKeyCode::G => 0x0022,
        VirtualKeyCode::H => 0x0023,
        VirtualKeyCode::J => 0x0024,
        VirtualKeyCode::K => 0x0025,
        VirtualKeyCode::L => 0x0026,
        VirtualKeyCode::Semicolon => 0x0027,
        VirtualKeyCode::Apostrophe => 0x0028,
        VirtualKeyCode::Grave => 0x0029,
        VirtualKeyCode::LShift => 0x002A,
        VirtualKeyCode::Backslash => 0x002B,
        VirtualKeyCode::Z => 0x002C,
        VirtualKeyCode::X => 0x002D,
        VirtualKeyCode::C => 0x002E,
        VirtualKeyCode::V => 0x002F,
        VirtualKeyCode::B => 0x0030,
        VirtualKeyCode::N => 0x0031,
        VirtualKeyCode::M => 0x0032,
        VirtualKeyCode::Comma => 0x0033,
        VirtualKeyCode::Period => 0x0034,
        VirtualKeyCode::Slash => 0x0035,
        VirtualKeyCode::RShift => 0x0036,
        VirtualKeyCode::NumpadMultiply => 0x0037,
        VirtualKeyCode::LAlt => 0x0038,
        VirtualKeyCode::Space => 0x0039,
        VirtualKeyCode::Capital => 0x003A,
        VirtualKeyCode::F1 => 0x003B,
        VirtualKeyCode::F2 => 0x003C,
        VirtualKeyCode::F3 => 0x003D,
        VirtualKeyCode::F4 => 0x003E,
        VirtualKeyCode::F5 => 0x003F,
        VirtualKeyCode::F6 => 0x0040,
        VirtualKeyCode::F7 => 0x0041,
        VirtualKeyCode::F8 => 0x0042,
        VirtualKeyCode::F9 => 0x0043,
        VirtualKeyCode::F10 => 0x0044,
        VirtualKeyCode::Pause => 0x0045,
        VirtualKeyCode::Scroll => 0x0046,
        VirtualKeyCode::Numpad7 => 0x0047,
        VirtualKeyCode::Numpad8 => 0x0048,
        VirtualKeyCode::Numpad9 => 0x0049,
        VirtualKeyCode::NumpadSubtract => 0x004A,
        VirtualKeyCode::Numpad4 => 0x004B,
        VirtualKeyCode::Numpad5 => 0x004C,
        VirtualKeyCode::Numpad6 => 0x004D,
        VirtualKeyCode::NumpadAdd => 0x004E,
        VirtualKeyCode::Numpad1 => 0x004F,
        VirtualKeyCode::Numpad2 => 0x0050,
        VirtualKeyCode::Numpad3 => 0x0051,
        VirtualKeyCode::Numpad0 => 0x0052,
        VirtualKeyCode::NumpadDecimal => 0x0053,
        VirtualKeyCode::F11 => 0x0057,
        VirtualKeyCode::F12 => 0x0058,
        VirtualKeyCode::F13 => 0x0064,
        VirtualKeyCode::F14 => 0x0065,
        VirtualKeyCode::F15 => 0x0066,
        VirtualKeyCode::NumpadEnter => 0xE01C,
        VirtualKeyCode::RControl => 0xE01D,
        VirtualKeyCode::NumpadDivide => 0xE035,
        VirtualKeyCode::RAlt => 0xE038,
        VirtualKeyCode::Numlock => 0xE045,
        VirtualKeyCode::Home => 0xE047,
        VirtualKeyCode::Up => 0xE048,
        VirtualKeyCode::PageUp => 0xE049,
        VirtualKeyCode::Left => 0xE04B,
        VirtualKeyCode::Right => 0xE04D,
        VirtualKeyCode::End => 0xE04F,
        VirtualKeyCode::Down => 0xE050,
        VirtualKeyCode::PageDown => 0xE051,
        VirtualKeyCode::Insert => 0xE052,
        VirtualKeyCode::Delete => 0xE053,
        VirtualKeyCode::LWin => 0xE05B,
        VirtualKeyCode::RWin => 0xE05C,
        VirtualKeyCode::Apps => 0xE05D,
        _ => todo!(),
    }
}
