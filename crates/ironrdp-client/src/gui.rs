use anyhow::Context as _;
use softbuffer::GraphicsContext;
use tokio::sync::mpsc;
use winit::event::{self, Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use winit::window::{Window, WindowBuilder};

use crate::rdp::{RdpInputEvent, RdpOutputEvent};
use winit::dpi::LogicalPosition;

pub struct GuiContext {
    pub window: Window,
    pub event_loop: EventLoop<RdpOutputEvent>,
    pub graphics_context: GraphicsContext,
}

impl GuiContext {
    pub fn init() -> anyhow::Result<Self> {
        let event_loop = EventLoopBuilder::<RdpOutputEvent>::with_user_event().build();

        let window = WindowBuilder::new()
            .with_title("IronRDP")
            .build(&event_loop)
            .context("Unable to create winit Window")?;

        let graphics_context = unsafe { GraphicsContext::new(&window, &window) }
            .map_err(|e| anyhow::Error::msg(format!("Unable to initialize graphics context: {e}")))?;

        Ok(Self {
            window,
            event_loop,
            graphics_context,
        })
    }

    pub fn run(self, input_event_sender: mpsc::UnboundedSender<RdpInputEvent>) -> ! {
        let Self {
            window,
            event_loop,
            mut graphics_context,
        } = self;

        let (mut image_width, mut image_height) = {
            let window_size = window.inner_size();
            (
                u16::try_from(window_size.width).unwrap(),
                u16::try_from(window_size.height).unwrap(),
            )
        };
        let mut image_buffer = vec![0; usize::from(image_width) * usize::from(image_height)];

        let mut input_database = ironrdp::input::Database::new();

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;

            let image_width = &mut image_width;
            let image_height = &mut image_height;
            let image_buffer = &mut image_buffer;

            match event {
                Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                    WindowEvent::Resized(size) => {
                        let _ = input_event_sender.send(RdpInputEvent::Resize {
                            width: u16::try_from(size.width).unwrap(),
                            height: u16::try_from(size.height).unwrap(),
                        });
                    }
                    WindowEvent::CloseRequested => {
                        control_flow.set_exit();
                    }
                    WindowEvent::DroppedFile(_) => {
                        // TODO: File upload
                    }
                    WindowEvent::ReceivedCharacter(_) => {
                        // TODO: Unicode mode
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
                    graphics_context.set_buffer(image_buffer, *image_width, *image_height);
                }
                Event::UserEvent(RdpOutputEvent::Image { buffer, width, height }) => {
                    *image_buffer = buffer;
                    *image_width = width;
                    *image_height = height;

                    graphics_context.set_buffer(image_buffer, width, height);
                }
                Event::UserEvent(RdpOutputEvent::ConnectionFailure(error)) => {
                    error!(?error);
                    eprintln!("Connection error: {}", error.report());
                    control_flow.set_exit_with_code(exitcode::PROTOCOL);
                }
                Event::UserEvent(RdpOutputEvent::Terminated(result)) => {
                    let exit_code = match result {
                        Ok(()) => {
                            println!("Terminated gracefully");
                            exitcode::OK
                        }
                        Err(error) => {
                            error!(?error);
                            eprintln!("Active session error: {}", error.report());
                            exitcode::PROTOCOL
                        }
                    };

                    control_flow.set_exit_with_code(exit_code);
                }
                Event::UserEvent(RdpOutputEvent::PointerHidden) => {
                    window.set_cursor_visible(false);
                }
                Event::UserEvent(RdpOutputEvent::PointerDefault) => {
                    window.set_cursor_visible(true);
                }
                Event::UserEvent(RdpOutputEvent::PointerPosition { x, y }) => {
                    if let Err(e) = window.set_cursor_position(LogicalPosition::new(x as f64, y as f64)) {
                        eprintln!("Failed to set cursor position: {}", e);
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
