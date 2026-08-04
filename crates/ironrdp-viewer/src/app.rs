#![allow(clippy::print_stderr, clippy::print_stdout)] // allowed in this module only

use core::num::NonZeroU32;
use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use ironrdp::client::rdp::{RdpInputEvent, RdpInputSender, RdpOutputEvent};
use ironrdp_daemon::daemon::{Daemon, ResizeError};
use raw_window_handle::{DisplayHandle, HasDisplayHandle as _};
use smallvec::SmallVec;
use tracing::{debug, error, info, trace, warn};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, PhysicalSize};
use winit::event::{self, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::scancode::PhysicalKeyExtScancode as _;
use winit::window::{CursorIcon, CustomCursor, Window, WindowAttributes};

type WindowSurface = (Arc<Window>, softbuffer::Surface<DisplayHandle<'static>, Arc<Window>>);

/// Events delivered from the RDP engine and the viewer-hosted RPC server to the window.
pub enum ViewerEvent {
    Output(RdpOutputEvent),
    Frame {
        buffer: Vec<u32>,
        width: NonZeroU32,
        height: NonZeroU32,
    },
    Shutdown,
}

/// Where local window input is sent.
pub enum InputTarget {
    Direct(RdpInputSender),
    Rpc(Arc<Daemon>),
}

pub struct App {
    input_target: InputTarget,
    context: softbuffer::Context<DisplayHandle<'static>>,
    initial_window_size: PhysicalSize<u32>,
    window: Option<WindowSurface>,
    buffer: Vec<u32>,
    buffer_size: (u16, u16),
    input_database: ironrdp::input::Database,
    last_size: Option<PhysicalSize<u32>>,
    resize_timeout: Option<Instant>,
}

impl App {
    pub fn new(
        event_loop: &EventLoop<ViewerEvent>,
        input_event_sender: &RdpInputSender,
        initial_window_size: PhysicalSize<u32>,
    ) -> anyhow::Result<Self> {
        Self::new_with_input_target(
            event_loop,
            InputTarget::Direct(input_event_sender.clone()),
            initial_window_size,
        )
    }

    pub fn new_with_input_target(
        event_loop: &EventLoop<ViewerEvent>,
        input_target: InputTarget,
        initial_window_size: PhysicalSize<u32>,
    ) -> anyhow::Result<Self> {
        // SAFETY: We drop the softbuffer context right before the event loop is stopped, thus making this safe.
        // FIXME: This is not a sufficient proof and the API is actually unsound as-is.
        let display_handle = unsafe {
            core::mem::transmute::<DisplayHandle<'_>, DisplayHandle<'static>>(
                event_loop.display_handle().context("get display handle")?,
            )
        };
        let context = softbuffer::Context::new(display_handle)
            .map_err(|e| anyhow::anyhow!("unable to initialize softbuffer context: {e}"))?;

        let input_database = ironrdp::input::Database::new();
        Ok(Self {
            input_target,
            context,
            initial_window_size,
            window: None,
            buffer: Vec::new(),
            buffer_size: (0, 0),
            input_database,
            last_size: None,
            resize_timeout: None,
        })
    }

    fn send_resize_event(&mut self) {
        let Some(size) = self.last_size else {
            return;
        };
        let Some((window, _)) = self.window.as_mut() else {
            return;
        };
        #[expect(clippy::as_conversions, reason = "casting f64 to u32")]
        let scale_factor = (window.scale_factor() * 100.0) as u32;

        let width = u16::try_from(size.width).expect("reasonable width");
        let height = u16::try_from(size.height).expect("reasonable height");

        match &self.input_target {
            InputTarget::Direct(input_event_sender) => match input_event_sender.try_send(RdpInputEvent::Resize {
                width,
                height,
                scale_factor,
                // TODO: it should be possible to get the physical size here, however winit doesn't make it straightforward.
                // FreeRDP does it based on DPI reading grabbed via [`SDL_GetDisplayDPI`](https://wiki.libsdl.org/SDL2/SDL_GetDisplayDPI):
                // https://github.com/FreeRDP/FreeRDP/blob/ba8cf8cf2158018fb7abbedb51ab245f369be813/client/SDL/sdl_monitor.cpp#L250-L262
                // See also: https://github.com/rust-windowing/winit/issues/826
                physical_size: None,
            }) {
                Ok(()) => self.last_size = None,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    self.resize_timeout = Some(Instant::now() + Duration::from_millis(10));
                }
                Err(_) => {
                    self.last_size = None;
                    warn!("Unable to enqueue resize event because the RDP session is closed");
                }
            },
            InputTarget::Rpc(daemon) => match daemon.try_resize(width, height) {
                Ok(()) => self.last_size = None,
                Err(ResizeError::Full) => self.resize_timeout = Some(Instant::now() + Duration::from_millis(10)),
                Err(error) => {
                    self.last_size = None;
                    warn!(?error, "Unable to resize the RPC-backed RDP session");
                }
            },
        }
    }

    fn update_frame(&mut self, buffer: Vec<u32>, width: NonZeroU32, height: NonZeroU32) {
        let Some((window, surface)) = self.window.as_mut() else {
            return;
        };
        trace!(?width, ?height, "Received RPC-backed image");
        self.buffer_size = (
            u16::try_from(width.get()).expect("frame width fits in u16"),
            u16::try_from(height.get()).expect("frame height fits in u16"),
        );
        self.buffer = buffer;
        surface.resize(width, height).expect("surface resize");
        window.request_redraw();
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

impl ApplicationHandler<ViewerEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(timeout) = self.resize_timeout {
            if let Some(timeout) = timeout.checked_duration_since(Instant::now()) {
                event_loop.set_control_flow(ControlFlow::wait_duration(timeout));
            } else {
                self.resize_timeout = None;
                self.send_resize_event();
                if let Some(retry_timeout) = self.resize_timeout {
                    event_loop.set_control_flow(ControlFlow::wait_duration(
                        retry_timeout.saturating_duration_since(Instant::now()),
                    ));
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = WindowAttributes::default()
            .with_title("IronRDP")
            .with_inner_size(self.initial_window_size);
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
            WindowEvent::CloseRequested => match &self.input_target {
                InputTarget::Direct(input_event_sender) => input_event_sender.request_graceful_close(),
                InputTarget::Rpc(daemon) => {
                    let _ = daemon.disconnect();
                    event_loop.exit();
                }
            },
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
                    let scancode = match u16::try_from(scancode) {
                        Ok(scancode) => scancode,
                        Err(_) => {
                            warn!("Unsupported scancode: `{scancode:#X}`; ignored");
                            return;
                        }
                    };
                    let scancode = ironrdp::input::Scancode::from_u16(scancode);

                    let operation = match event.state {
                        event::ElementState::Pressed => ironrdp::input::Operation::KeyPressed(scancode),
                        event::ElementState::Released => ironrdp::input::Operation::KeyReleased(scancode),
                    };

                    apply_and_send_fast_path_events(
                        &self.input_target,
                        &mut self.input_database,
                        core::iter::once(operation),
                    );
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                const SHIFT_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(false, 0x2A);
                const CONTROL_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(false, 0x1D);
                const ALT_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(false, 0x38);
                const LOGO_LEFT: ironrdp::input::Scancode = ironrdp::input::Scancode::from_u8(true, 0x5B);

                let mut operations = SmallVec::<[ironrdp::input::Operation; 4]>::new();

                let mut add_operation = |pressed: bool, scancode: ironrdp::input::Scancode| {
                    let operation = if pressed {
                        ironrdp::input::Operation::KeyPressed(scancode)
                    } else {
                        ironrdp::input::Operation::KeyReleased(scancode)
                    };
                    operations.push(operation);
                };

                // NOTE: https://docs.rs/winit/0.30.12/src/winit/keyboard.rs.html#1737-1744
                //
                // We can’t use state.lshift_state(), state.lcontrol_state(), etc, because on some platforms such as
                // Linux, the modifiers change is hidden.
                //
                // > The exact modifier key is not used to represent modifiers state in the
                // > first place due to a fact that modifiers state could be changed without any
                // > key being pressed and on some platforms like Wayland/X11 which key resulted
                // > in modifiers change is hidden, also, not that it really matters.
                add_operation(modifiers.state().shift_key(), SHIFT_LEFT);
                add_operation(modifiers.state().control_key(), CONTROL_LEFT);
                add_operation(modifiers.state().alt_key(), ALT_LEFT);
                add_operation(modifiers.state().super_key(), LOGO_LEFT);

                apply_and_send_fast_path_events(&self.input_target, &mut self.input_database, operations);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let win_size = window.inner_size();
                #[expect(clippy::as_conversions, reason = "casting f64 to u16")]
                let x = (position.x / f64::from(win_size.width) * f64::from(self.buffer_size.0)) as u16;
                #[expect(clippy::as_conversions, reason = "casting f64 to u16")]
                let y = (position.y / f64::from(win_size.height) * f64::from(self.buffer_size.1)) as u16;
                let operation = ironrdp::input::Operation::MouseMove(ironrdp::input::MousePosition { x, y });

                apply_and_send_fast_path_events(
                    &self.input_target,
                    &mut self.input_database,
                    core::iter::once(operation),
                );
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let mut operations = SmallVec::<[ironrdp::input::Operation; 2]>::new();

                match delta {
                    event::MouseScrollDelta::LineDelta(delta_x, delta_y) => {
                        if delta_x.abs() > 0.001 {
                            operations.push(ironrdp::input::Operation::WheelRotations(
                                ironrdp::input::WheelRotations {
                                    is_vertical: false,
                                    #[expect(clippy::as_conversions, reason = "casting f32 to i16")]
                                    rotation_units: (delta_x * 100.) as i16,
                                },
                            ));
                        }

                        if delta_y.abs() > 0.001 {
                            operations.push(ironrdp::input::Operation::WheelRotations(
                                ironrdp::input::WheelRotations {
                                    is_vertical: true,
                                    #[expect(clippy::as_conversions, reason = "casting f32 to i16")]
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
                                    #[expect(clippy::as_conversions, reason = "casting f64 to i16")]
                                    rotation_units: delta.x as i16,
                                },
                            ));
                        }

                        if delta.y.abs() > 0.001 {
                            operations.push(ironrdp::input::Operation::WheelRotations(
                                ironrdp::input::WheelRotations {
                                    is_vertical: true,
                                    #[expect(clippy::as_conversions, reason = "casting f64 to i16")]
                                    rotation_units: delta.y as i16,
                                },
                            ));
                        }
                    }
                };

                apply_and_send_fast_path_events(&self.input_target, &mut self.input_database, operations);
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

                apply_and_send_fast_path_events(
                    &self.input_target,
                    &mut self.input_database,
                    core::iter::once(operation),
                );
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ViewerEvent) {
        let ViewerEvent::Output(event) = event else {
            match event {
                ViewerEvent::Frame { buffer, width, height } => self.update_frame(buffer, width, height),
                ViewerEvent::Shutdown => event_loop.exit(),
                ViewerEvent::Output(_) => unreachable!(),
            }
            return;
        };

        let Some((window, surface)) = self.window.as_mut() else {
            return;
        };
        match event {
            RdpOutputEvent::Connected => info!("RDP session connected"),
            RdpOutputEvent::LoginComplete => info!("RDP login complete"),
            RdpOutputEvent::PostLogonDisplayRedraw => info!("Requested post-logon display redraw"),
            RdpOutputEvent::MalformedBitmapDisplayRedraw => {
                warn!("Requested display redraw after discarding a malformed bitmap update");
            }
            RdpOutputEvent::Image { buffer, width, height } => {
                trace!(width = ?width, height = ?height, "Received image with size");
                trace!(window_physical_size = ?window.inner_size(), "Drawing image to the window with size");
                self.buffer_size = (width.get(), height.get());
                self.buffer = buffer;
                surface
                    .resize(NonZeroU32::from(width), NonZeroU32::from(height))
                    .expect("surface resize");

                window.request_redraw();
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                error!(?error);
                eprintln!("Connection error: {}", error.report().with_locations());
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
                        eprintln!("Active session error: {}", error.report().with_locations());
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
            RdpOutputEvent::DisplayResizeFallback(reason) => {
                warn!(
                    ?reason,
                    "Reconnecting because dynamic display resize could not complete"
                );
            }
        }
    }
}

fn apply_and_send_fast_path_events(
    input_target: &InputTarget,
    input_database: &mut ironrdp::input::Database,
    operations: impl IntoIterator<Item = ironrdp::input::Operation>,
) {
    match input_target {
        InputTarget::Direct(input_event_sender) => {
            let Ok(permit) = input_event_sender.try_reserve() else {
                return;
            };
            let input_events = input_database.apply(operations);
            if !input_events.is_empty() {
                permit.send(RdpInputEvent::FastPath(input_events));
            }
        }
        InputTarget::Rpc(daemon) => {
            for operation in operations {
                let _ = daemon.input(operation);
            }
        }
    }
}
