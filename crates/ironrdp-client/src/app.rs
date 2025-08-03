#![allow(clippy::print_stderr, clippy::print_stdout)] // allowed in this module only

use core::num::NonZeroU32;
use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use raw_window_handle::{DisplayHandle, HasDisplayHandle as _};
use tokio::sync::mpsc;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, PhysicalSize};
use winit::event::{self, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::scancode::PhysicalKeyExtScancode as _;
use winit::window::{CursorIcon, CustomCursor, Window, WindowAttributes};

use crate::rdp::{RdpInputEvent, RdpOutputEvent};

type WindowSurface = (Arc<Window>, softbuffer::Surface<DisplayHandle<'static>, Arc<Window>>);

pub struct App {
    input_event_sender: mpsc::UnboundedSender<RdpInputEvent>,
    context: softbuffer::Context<DisplayHandle<'static>>,
    window: Option<WindowSurface>,
    buffer: Vec<u32>,
    buffer_size: (u16, u16),
    input_database: ironrdp::input::Database,
    last_size: Option<PhysicalSize<u32>>,
    resize_timeout: Option<Instant>,
}

impl App {
    pub fn new(
        event_loop: &EventLoop<RdpOutputEvent>,
        input_event_sender: &mpsc::UnboundedSender<RdpInputEvent>,
    ) -> anyhow::Result<Self> {
        // SAFETY: We drop the softbuffer context right before the event loop is stopped, thus making this safe.
        // FIXME: This is not a sufficient proof and the API is actually unsound as-is.
        let display_handle = unsafe {
            core::mem::transmute::<DisplayHandle<'_>, DisplayHandle<'static>>(event_loop.display_handle().unwrap())
        };
        let context = softbuffer::Context::new(display_handle)
            .map_err(|e| anyhow::anyhow!("unable to initialize softbuffer context: {e}"))?;

        let input_database = ironrdp::input::Database::new();
        Ok(Self {
            input_event_sender: input_event_sender.clone(),
            context,
            window: None,
            buffer: Vec::new(),
            buffer_size: (0, 0),
            input_database,
            last_size: None,
            resize_timeout: None,
        })
    }

    fn send_resize_event(&mut self) {
        let Some(size) = self.last_size.take() else {
            return;
        };
        let Some((window, _)) = self.window.as_mut() else {
            return;
        };
        let scale_factor = (window.scale_factor() * 100.0) as u32;

        let _ = self.input_event_sender.send(RdpInputEvent::Resize {
            width: u16::try_from(size.width).unwrap(),
            height: u16::try_from(size.height).unwrap(),
            scale_factor,
            // TODO: it should be possible to get the physical size here, however winit doesn't make it straightforward.
            // FreeRDP does it based on DPI reading grabbed via [`SDL_GetDisplayDPI`](https://wiki.libsdl.org/SDL2/SDL_GetDisplayDPI):
            // https://github.com/FreeRDP/FreeRDP/blob/ba8cf8cf2158018fb7abbedb51ab245f369be813/client/SDL/sdl_monitor.cpp#L250-L262
            // See also: https://github.com/rust-windowing/winit/issues/826
            physical_size: None,
        });
    }

    fn draw(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let Some((_, surface)) = self.window.as_mut() else {
            return;
        };
        let mut sb_buffer = surface.buffer_mut().expect("surface buffer");
        sb_buffer.copy_from_slice(self.buffer.as_slice());
        sb_buffer.present().expect("buffer present");
    }
}

impl ApplicationHandler<RdpOutputEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(timeout) = self.resize_timeout {
            if let Some(timeout) = timeout.checked_duration_since(Instant::now()) {
                event_loop.set_control_flow(ControlFlow::wait_duration(timeout));
            } else {
                self.send_resize_event();
                self.resize_timeout = None;
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = WindowAttributes::default().with_title("IronRDP");
        match event_loop.create_window(window_attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                let surface = softbuffer::Surface::new(&self.context, Arc::clone(&window)).expect("surface");
                self.window = Some((window, surface));
            }
            Err(error) => {
                error!(%error, "Failed to create window");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: winit::window::WindowId, event: WindowEvent) {
        let Some((window, _)) = self.window.as_mut() else {
            return;
        };
        if window_id != window.id() {
            return;
        }

        match event {
            WindowEvent::Resized(size) => {
                self.last_size = Some(size);
                self.resize_timeout = Some(Instant::now() + Duration::from_secs(1));
            }
            WindowEvent::CloseRequested => {
                if self.input_event_sender.send(RdpInputEvent::Close).is_err() {
                    error!("Failed to send graceful shutdown event, closing the window");
                    event_loop.exit();
                }
            }
            WindowEvent::DroppedFile(_) => {
                // TODO(#110): File upload
            }
            // WindowEvent::ReceivedCharacter(_) => {
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
            // }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(scancode) = event.physical_key.to_scancode() {
                    let scancode = ironrdp::input::Scancode::from_u16(u16::try_from(scancode).unwrap());

                    let operation = match event.state {
                        event::ElementState::Pressed => ironrdp::input::Operation::KeyPressed(scancode),
                        event::ElementState::Released => ironrdp::input::Operation::KeyReleased(scancode),
                    };

                    let input_events = self.input_database.apply(core::iter::once(operation));

                    send_fast_path_events(&self.input_event_sender, input_events);
                }
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

                add_operation(state.state().shift_key(), SHIFT_LEFT);
                add_operation(state.state().control_key(), CONTROL_LEFT);
                add_operation(state.state().alt_key(), ALT_LEFT);
                add_operation(state.state().super_key(), LOGO_LEFT);

                let input_events = self.input_database.apply(operations);

                send_fast_path_events(&self.input_event_sender, input_events);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let win_size = window.inner_size();
                let x = (position.x / win_size.width as f64 * self.buffer_size.0 as f64) as u16;
                let y = (position.y / win_size.height as f64 * self.buffer_size.1 as f64) as u16;
                let operation = ironrdp::input::Operation::MouseMove(ironrdp::input::MousePosition { x, y });

                let input_events = self.input_database.apply(core::iter::once(operation));

                send_fast_path_events(&self.input_event_sender, input_events);
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

                let input_events = self.input_database.apply(operations);

                send_fast_path_events(&self.input_event_sender, input_events);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let mouse_button = match button {
                    event::MouseButton::Left => ironrdp::input::MouseButton::Left,
                    event::MouseButton::Right => ironrdp::input::MouseButton::Right,
                    event::MouseButton::Middle => ironrdp::input::MouseButton::Middle,
                    event::MouseButton::Back => ironrdp::input::MouseButton::X1,
                    event::MouseButton::Forward => ironrdp::input::MouseButton::X2,
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
                    event::ElementState::Released => ironrdp::input::Operation::MouseButtonReleased(mouse_button),
                };

                let input_events = self.input_database.apply(core::iter::once(operation));

                send_fast_path_events(&self.input_event_sender, input_events);
            }
            WindowEvent::RedrawRequested => {
                self.draw();
            }
            WindowEvent::ActivationTokenDone { .. }
            | WindowEvent::Moved(_)
            | WindowEvent::Destroyed
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::Focused(_)
            | WindowEvent::Ime(_)
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::Occluded(_) => {
                // ignore
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RdpOutputEvent) {
        let Some((window, surface)) = self.window.as_mut() else {
            return;
        };
        match event {
            RdpOutputEvent::Image { buffer, width, height } => {
                trace!(width = ?width, height = ?height, "Received image with size");
                trace!(window_physical_size = ?window.inner_size(), "Drawing image to the window with size");
                self.buffer_size = (width, height);
                self.buffer = buffer;
                surface
                    .resize(
                        NonZeroU32::new(u32::from(width)).unwrap(),
                        NonZeroU32::new(u32::from(height)).unwrap(),
                    )
                    .expect("surface resize");

                window.request_redraw();
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                error!(?error);
                eprintln!("Connection error: {}", error.report());
                // TODO set proc_exit::sysexits::PROTOCOL_ERR.as_raw());
                event_loop.exit();
            }
            RdpOutputEvent::Terminated(result) => {
                let _exit_code = match result {
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
                // TODO set exit_code.as_raw());
                event_loop.exit();
            }
            RdpOutputEvent::PointerHidden => {
                window.set_cursor_visible(false);
            }
            RdpOutputEvent::PointerDefault => {
                window.set_cursor(CursorIcon::default());
                window.set_cursor_visible(true);
            }
            RdpOutputEvent::PointerPosition { x, y } => {
                if let Err(error) = window.set_cursor_position(LogicalPosition::new(x, y)) {
                    error!(?error, "Failed to set cursor position");
                }
            }
            RdpOutputEvent::PointerBitmap(pointer) => {
                debug!(width = ?pointer.width, height = ?pointer.height, "Received pointer bitmap");
                match CustomCursor::from_rgba(
                    pointer.bitmap_data.clone(),
                    pointer.width,
                    pointer.height,
                    pointer.hotspot_x,
                    pointer.hotspot_y,
                ) {
                    Ok(cursor) => window.set_cursor(event_loop.create_custom_cursor(cursor)),
                    Err(error) => error!(?error, "Failed to set cursor bitmap"),
                }
                window.set_cursor_visible(true);
            }
        }
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
