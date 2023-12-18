#![allow(clippy::print_stderr, clippy::print_stdout)] // allowed in this module only

use std::num::NonZeroU32;

use anyhow::Context as _;
use tokio::sync::mpsc;
use winit::dpi::LogicalPosition;
use winit::event::{self, Event, WindowEvent};
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
                        let _ = input_event_sender.send(RdpInputEvent::Resize {
                            width: u16::try_from(size.width).unwrap(),
                            height: u16::try_from(size.height).unwrap(),
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
                        // TODO(#106): Unicode mode
                    }
                    WindowEvent::KeyboardInput { input, .. } => {
                        let scancode = ironrdp::input::Scancode::from_u16(u16::try_from(input.scancode).unwrap());

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
                        let operation = ironrdp::input::Operation::MouseMove(ironrdp::input::MousePosition {
                            x: position.x as u16,
                            y: position.y as u16,
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
