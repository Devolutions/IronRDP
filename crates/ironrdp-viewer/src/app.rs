#![allow(clippy::print_stderr, clippy::print_stdout)] // allowed in this module only

use core::error::Error as _;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use ironrdp::client::rdp::{
    AutoReconnectDecision, RDPEI_MAX_TOUCH_CONTACTS, RdpInputEvent, RdpInputSender, RdpOutputEvent,
};
use ironrdp::rdpei::RdpeiInputState;
use ironrdp::rdpei::contact::{TouchFrameBuilder, TouchPhase};
use ironrdp_daemon::daemon::{Daemon, ResizeError};
use raw_window_handle::{DisplayHandle, HasDisplayHandle as _};
use smallvec::SmallVec;
use tracing::{debug, error, info, trace, warn};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, PhysicalPosition, PhysicalSize};
use winit::event::{self, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorIcon, CustomCursor, Window, WindowAttributes};

use crate::keymap::{is_modifier, map_key_code};

/// Emulated mouse operations kept while the session input queue is full; beyond this, moves are dropped.
const MAX_PENDING_TOUCH_MOUSE: usize = 256;

type WindowSurface = (Arc<Window>, softbuffer::Surface<DisplayHandle<'static>, Arc<Window>>);

/// Events delivered from the viewer-hosted RPC server to the window.
pub enum ViewerEvent {
    FrameAvailable,
    Shutdown,
}

/// Where local window input is sent.
enum InputTarget {
    Direct(RdpInputSender),
    Rpc(Arc<Daemon>),
}

/// A viewer application driven by the RDP client's native output events.
pub struct App {
    inner: RpcApp,
}

/// A viewer application driven by a viewer-hosted RPC daemon.
pub struct RpcApp {
    input_target: InputTarget,
    frame_wakeup: Option<Arc<AtomicBool>>,
    context: softbuffer::Context<DisplayHandle<'static>>,
    initial_window_size: PhysicalSize<u32>,
    window: Option<WindowSurface>,
    buffer: Vec<u32>,
    buffer_size: (u16, u16),
    input_database: ironrdp::input::Database,
    last_size: Option<PhysicalSize<u32>>,
    resize_timeout: Option<Instant>,
    /// What the server's RDPEI channel accepts; touch is emulated as mouse input otherwise.
    rdpei_input_state: RdpeiInputState,
    touch_frames: TouchFrameBuilder,
    /// The touch point that drives the emulated mouse.
    mouse_touch: Option<u64>,
    /// Emulated mouse input from touch that waits for room in the session input queue.
    pending_touch_mouse: Vec<ironrdp::input::Operation>,
    /// Origin of the touch frame timestamps.
    touch_epoch: Instant,
    close_requested: bool,
}

impl App {
    pub fn new(
        event_loop: &EventLoop<RdpOutputEvent>,
        input_event_sender: &RdpInputSender,
        initial_window_size: PhysicalSize<u32>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            inner: RpcApp::new_inner(
                event_loop,
                InputTarget::Direct(input_event_sender.clone()),
                None,
                initial_window_size,
            )?,
        })
    }
}

impl RpcApp {
    pub fn new(
        event_loop: &EventLoop<ViewerEvent>,
        daemon: Arc<Daemon>,
        frame_wakeup: Arc<AtomicBool>,
        initial_window_size: PhysicalSize<u32>,
    ) -> anyhow::Result<Self> {
        Self::new_inner(
            event_loop,
            InputTarget::Rpc(daemon),
            Some(frame_wakeup),
            initial_window_size,
        )
    }

    fn new_inner<T: 'static>(
        event_loop: &EventLoop<T>,
        input_target: InputTarget,
        frame_wakeup: Option<Arc<AtomicBool>>,
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
            frame_wakeup,
            context,
            initial_window_size,
            window: None,
            buffer: Vec::new(),
            buffer_size: (0, 0),
            input_database,
            last_size: None,
            resize_timeout: None,
            rdpei_input_state: RdpeiInputState::default(),
            touch_frames: TouchFrameBuilder::new(RDPEI_MAX_TOUCH_CONTACTS),
            mouse_touch: None,
            pending_touch_mouse: Vec::new(),
            touch_epoch: Instant::now(),
            close_requested: false,
        })
    }

    fn is_expected_shutdown_error(error: &ironrdp::session::SessionError) -> bool {
        use ironrdp::core::DecodeErrorKind;
        use ironrdp::session::SessionErrorKind;

        match error.kind() {
            SessionErrorKind::Decode(decode) => matches!(decode.kind(), DecodeErrorKind::NotEnoughBytes { .. }),
            SessionErrorKind::Custom => {
                let mut source = error.source();
                while let Some(err) = source {
                    if let Some(decode) = err.downcast_ref::<ironrdp::core::DecodeError>() {
                        if matches!(decode.kind(), DecodeErrorKind::NotEnoughBytes { .. }) {
                            return true;
                        }
                    }
                    if let Some(io) = err.downcast_ref::<std::io::Error>() {
                        use std::io::ErrorKind;
                        if matches!(
                            io.kind(),
                            ErrorKind::UnexpectedEof
                                | ErrorKind::ConnectionAborted
                                | ErrorKind::ConnectionReset
                                | ErrorKind::BrokenPipe
                        ) {
                            return true;
                        }
                    }
                    source = err.source();
                }
                false
            }
            _ => false,
        }
    }

    fn touch_timestamp_us(&self) -> u64 {
        u64::try_from(self.touch_epoch.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// Sends the touch frames gathered since the last call as one RDPEI touch event.
    fn send_touch_frames(&mut self) {
        let InputTarget::Direct(input_event_sender) = &self.input_target else {
            return;
        };
        // A suspended server keeps its contacts, so their frames wait for the resume.
        if !self.touch_frames.has_pending_frames() || self.rdpei_input_state.suspended {
            return;
        }
        // Frames stay pending while the queue is full, so no contact transition is lost.
        let Ok(permit) = input_event_sender.try_reserve() else {
            return;
        };
        let now_us = self.touch_timestamp_us();
        if let Some(event) = self.touch_frames.take_event(now_us) {
            permit.send(RdpInputEvent::Touch(event));
        }
    }

    /// Routes one touch point to RDPEI, or to the emulated mouse when the server does not accept touch.
    ///
    /// winit on Windows also reports pen input as touch points, with pressure but without hover, tilt,
    /// rotation or buttons, so pens drive RDPEI touch contacts here.
    fn handle_touch(&mut self, touch: event::Touch) {
        let Some((window, _)) = self.window.as_ref() else {
            return;
        };
        let position = desktop_position(window.inner_size(), self.buffer_size, touch.location);
        let phase = match touch.phase {
            event::TouchPhase::Started => TouchPhase::Started,
            event::TouchPhase::Moved => TouchPhase::Moved,
            event::TouchPhase::Ended => TouchPhase::Ended,
            event::TouchPhase::Cancelled => TouchPhase::Canceled,
        };

        // A touch point that started on the emulated mouse stays there, so its button is released.
        let rdpei = matches!(self.input_target, InputTarget::Direct(_)) && self.rdpei_input_state.touch;
        if rdpei && self.mouse_touch != Some(touch.id) {
            #[expect(clippy::as_conversions, reason = "casting a normalized f64 force to u32")]
            let pressure = touch.force.map(|force| (force.normalized() * 1024.0).round() as u32);
            let timestamp_us = self.touch_timestamp_us();
            self.touch_frames.push(
                touch.id,
                phase,
                i32::from(position.x),
                i32::from(position.y),
                pressure,
                timestamp_us,
            );
            return;
        }

        // The first touch point drives the left mouse button, the way Windows promotes touch to mouse.
        // Moves without a touching point move the pointer, which follows a hovering pen.
        {
            use ironrdp::input::{MouseButton, Operation};

            let operations: SmallVec<[Operation; 2]> = match (phase, self.mouse_touch) {
                (TouchPhase::Started, None) => {
                    self.mouse_touch = Some(touch.id);
                    smallvec::smallvec![
                        Operation::MouseMove(position),
                        Operation::MouseButtonPressed(MouseButton::Left)
                    ]
                }
                (TouchPhase::Moved, None) => smallvec::smallvec![Operation::MouseMove(position)],
                (TouchPhase::Moved, Some(primary)) if primary == touch.id => {
                    smallvec::smallvec![Operation::MouseMove(position)]
                }
                (TouchPhase::Ended | TouchPhase::Canceled, Some(primary)) if primary == touch.id => {
                    self.mouse_touch = None;
                    smallvec::smallvec![
                        Operation::MouseMove(position),
                        Operation::MouseButtonReleased(MouseButton::Left)
                    ]
                }
                _ => SmallVec::new(),
            };
            self.pending_touch_mouse.extend(operations);
        }
        // While the queue stays full, only the oldest pointer moves are dropped; button changes are kept.
        if MAX_PENDING_TOUCH_MOUSE < self.pending_touch_mouse.len()
            && let Some(index) = self
                .pending_touch_mouse
                .iter()
                .position(|operation| matches!(operation, ironrdp::input::Operation::MouseMove(_)))
        {
            self.pending_touch_mouse.remove(index);
        }
        self.send_touch_mouse();
    }

    /// Sends the emulated mouse input from touch, keeping it while the session input queue is full.
    fn send_touch_mouse(&mut self) {
        if !self.pending_touch_mouse.is_empty()
            && apply_and_send_fast_path_events(
                &self.input_target,
                &mut self.input_database,
                self.pending_touch_mouse.iter().cloned(),
            )
        {
            self.pending_touch_mouse.clear();
        }
    }

    /// Lifts every touch contact and the emulated mouse button, for when the window loses focus.
    fn release_touch(&mut self) {
        let now_us = self.touch_timestamp_us();
        self.touch_frames.cancel_all(now_us);
        self.send_touch_frames();
        if self.mouse_touch.take().is_some() {
            self.pending_touch_mouse
                .push(ironrdp::input::Operation::MouseButtonReleased(
                    ironrdp::input::MouseButton::Left,
                ));
            self.send_touch_mouse();
        }
    }

    fn send_resize_event(&mut self) {
        let Some(size) = self.last_size else {
            return;
        };
        let Some((window, _)) = self.window.as_mut() else {
            return;
        };
        // The viewer window spans a single monitor, so the session gets one primary monitor at the
        // window DPI. A scale outside 100..=500 percent is clamped when the layout is encoded.
        #[expect(clippy::as_conversions, reason = "casting f64 to u32")]
        let scale_factor = (window.scale_factor() * 100.0).round() as u32;

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
            InputTarget::Rpc(daemon) => match daemon.try_resize(width, height, scale_factor) {
                Ok(()) => self.last_size = None,
                Err(ResizeError::Full) => self.resize_timeout = Some(Instant::now() + Duration::from_millis(10)),
                Err(error) => {
                    self.last_size = None;
                    warn!(?error, "Unable to resize the RPC-backed RDP session");
                }
            },
        }
    }

    fn update_rpc_frame(&mut self) {
        let InputTarget::Rpc(daemon) = &self.input_target else {
            return;
        };
        let Some(frame) = daemon.current_frame() else {
            return;
        };
        let (Some(width), Some(height)) = (
            NonZeroU32::new(u32::from(frame.width)),
            NonZeroU32::new(u32::from(frame.height)),
        ) else {
            return;
        };
        self.update_frame(frame.pixels, width, height);
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
    fn on_about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.send_touch_frames();
        self.send_touch_mouse();
        // Touch input that found the session input queue full is retried shortly, since draining the queue
        // does not wake the event loop.
        // Frames held for a suspended server wait for the resume event, which wakes the loop.
        let touch_pending = (self.touch_frames.has_pending_frames() && !self.rdpei_input_state.suspended)
            || !self.pending_touch_mouse.is_empty();
        let touch_retry = touch_pending.then(|| Instant::now() + Duration::from_millis(10));
        let deadline = touch_retry.into_iter().chain(self.resize_deadline()).min();
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    /// Sends a resize that is due and returns when the next one is.
    fn resize_deadline(&mut self) -> Option<Instant> {
        let timeout = self.resize_timeout?;
        if Instant::now() < timeout {
            return Some(timeout);
        }
        self.resize_timeout = None;
        self.send_resize_event();
        self.resize_timeout
    }

    fn on_resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = WindowAttributes::default()
            .with_title("IronRDP")
            .with_inner_size(self.initial_window_size);
        match event_loop.create_window(window_attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                if matches!(&self.input_target, InputTarget::Rpc(_)) {
                    window.set_cursor_visible(false);
                }
                let surface = softbuffer::Surface::new(&self.context, Arc::clone(&window)).expect("surface");
                self.window = Some((window, surface));
            }
            Err(error) => {
                error!(%error, "Failed to create window");
                event_loop.exit();
            }
        }
    }

    fn on_window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
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
            WindowEvent::ScaleFactorChanged { .. } => {
                // The window moved to a monitor with another DPI, or the monitor DPI changed.
                // Send the layout again with the matching desktop scale factor; winit usually
                // follows with a `Resized` event, which refreshes the size within the same debounce.
                self.last_size = Some(window.inner_size());
                self.resize_timeout = Some(Instant::now() + Duration::from_secs(1));
            }
            WindowEvent::CloseRequested => {
                self.close_requested = true;
                match &self.input_target {
                    InputTarget::Direct(input_event_sender) => input_event_sender.request_graceful_close(),
                    InputTarget::Rpc(daemon) => {
                        let _ = daemon.disconnect();
                        daemon.shutdown();
                    }
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
                let key_code = match event.physical_key {
                    PhysicalKey::Code(key_code) => key_code,
                    PhysicalKey::Unidentified(native_key_code) => {
                        warn!(?native_key_code, "Unsupported physical key; ignored");
                        return;
                    }
                };

                // `ModifiersChanged` is authoritative for these keys.
                if is_modifier(key_code) {
                    return;
                }

                let Some((scancode, release_only)) = map_key_code(key_code) else {
                    warn!(?key_code, "Unsupported physical key; ignored");
                    return;
                };

                let operations: SmallVec<[ironrdp::input::Operation; 2]> = match event.state {
                    event::ElementState::Pressed => {
                        smallvec::smallvec![ironrdp::input::Operation::KeyPressed(scancode)]
                    }
                    event::ElementState::Released if release_only => smallvec::smallvec![
                        ironrdp::input::Operation::KeyPressed(scancode),
                        ironrdp::input::Operation::KeyReleased(scancode),
                    ],
                    event::ElementState::Released => {
                        smallvec::smallvec![ironrdp::input::Operation::KeyReleased(scancode)]
                    }
                };

                apply_and_send_fast_path_events(&self.input_target, &mut self.input_database, operations);
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
                if is_touch_or_pen_mouse_message() {
                    return;
                }
                let position = desktop_position(window.inner_size(), self.buffer_size, position);
                let operation = ironrdp::input::Operation::MouseMove(position);

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
                if is_touch_or_pen_mouse_message() {
                    return;
                }
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
            WindowEvent::Touch(touch) => self.handle_touch(touch),
            WindowEvent::Focused(false) => self.release_touch(),
            WindowEvent::ActivationTokenDone { .. }
            | WindowEvent::Moved(_)
            | WindowEvent::Destroyed
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::Focused(true)
            | WindowEvent::Ime(_)
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::Occluded(_) => {
                // ignore
            }
        }
    }

    fn handle_viewer_event(&mut self, event_loop: &ActiveEventLoop, event: ViewerEvent) {
        match event {
            ViewerEvent::FrameAvailable => {
                if let Some(frame_wakeup) = &self.frame_wakeup {
                    frame_wakeup.store(false, Ordering::Release);
                }
                self.update_rpc_frame();
            }
            ViewerEvent::Shutdown => event_loop.exit(),
        }
    }

    fn handle_output_event(&mut self, event_loop: &ActiveEventLoop, event: RdpOutputEvent) {
        let Some((window, surface)) = self.window.as_mut() else {
            return;
        };
        match event {
            RdpOutputEvent::Connected => info!("RDP session connected"),
            RdpOutputEvent::MonitorLayout(monitors) => {
                debug!(monitor_count = monitors.len(), "Received remote monitor layout");
            }
            RdpOutputEvent::MonitorLayoutRejected => {
                warn!("The server cannot accept the requested monitor layout");
            }
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
                        if self.close_requested && Self::is_expected_shutdown_error(&error) {
                            println!(
                                "Terminated gracefully: {}",
                                ironrdp::session::GracefulDisconnectReason::UserInitiated
                            );
                            proc_exit::sysexits::OK
                        } else {
                            error!(?error);
                            eprintln!("Active session error: {}", error.report().with_locations());
                            proc_exit::sysexits::PROTOCOL_ERR
                        }
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
            RdpOutputEvent::AutoReconnecting {
                attempt,
                maximum_attempts,
                response,
                ..
            } => {
                warn!(attempt, maximum_attempts, "Stopping unsupported automatic reconnect");
                let _ = response.send(AutoReconnectDecision::Stop);
            }
            RdpOutputEvent::AutoReconnected => {
                info!("RDP session automatically reconnected");
            }
            RdpOutputEvent::ServerRedirected { target } => {
                info!(%target, "Server redirected the connection");
            }
            RdpOutputEvent::RdpeiInputState(state) => {
                debug!(?state, "RDPEI input state changed");
                // The server dropped the contacts with the channel state, so they are forgotten unsent.
                // Suspension keeps them: their frames go out when the server resumes input.
                if !state.touch {
                    self.touch_frames.reset();
                }
                self.rdpei_input_state = state;
                self.send_touch_frames();
            }
            RdpOutputEvent::RailHandshake {
                handshake_ex_flags,
                initialization_message_count,
                queued_execute_count,
            } => {
                debug!(
                    ?handshake_ex_flags,
                    initialization_message_count, queued_execute_count, "RAIL static channel initialized"
                );
            }
            RdpOutputEvent::RailDesktopSynchronized { released_execute_count } => {
                debug!(
                    released_execute_count,
                    "RAIL queued input released after desktop synchronization"
                );
            }
            RdpOutputEvent::RailPostHandshakeQueueReleased { released_execute_count } => {
                debug!(
                    released_execute_count,
                    "RAIL queued input released after handshake fallback"
                );
            }
            RdpOutputEvent::RailExecuteResult(result) => {
                debug!(?result, "RAIL execute completed");
            }
            RdpOutputEvent::RailExecuteFailed { flags, reason, .. } => {
                warn!(flags, ?reason, "RAIL execute could not be processed");
            }
            RdpOutputEvent::RailApplicationId {
                window_id,
                application_id,
                process_id,
                process_image_name,
            } => {
                debug!(
                    window_id,
                    %application_id,
                    ?process_id,
                    ?process_image_name,
                    "RAIL application identity received"
                );
            }
            RdpOutputEvent::RailControl(control) => {
                debug!(?control, "RAIL control received");
            }
            RdpOutputEvent::WindowingOrders(_) => {}
            // Only produced when the client is built with `.with_desktop_updates()`, which the
            // viewer does not opt into: it always presents full-frame `Image` snapshots instead.
            RdpOutputEvent::DesktopUpdate(_) => {}
        }
    }
}

impl ApplicationHandler<ViewerEvent> for RpcApp {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.on_about_to_wait(event_loop);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.on_resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: winit::window::WindowId, event: WindowEvent) {
        self.on_window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ViewerEvent) {
        self.handle_viewer_event(event_loop, event);
    }
}

impl ApplicationHandler<RdpOutputEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.on_about_to_wait(event_loop);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.on_resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: winit::window::WindowId, event: WindowEvent) {
        self.inner.on_window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RdpOutputEvent) {
        self.inner.handle_output_event(event_loop, event);
    }
}

/// Maps a window position onto the remote desktop, which the viewer stretches over the client area.
fn desktop_position(
    window_size: PhysicalSize<u32>,
    desktop_size: (u16, u16),
    position: PhysicalPosition<f64>,
) -> ironrdp::input::MousePosition {
    let scale = |position: f64, window: u32, desktop: u16| {
        let scaled = position / f64::from(window.max(1)) * f64::from(desktop);
        #[expect(clippy::as_conversions, reason = "casting a clamped f64 to u16")]
        let scaled = scaled.clamp(0.0, f64::from(desktop.saturating_sub(1))) as u16;
        scaled
    };
    ironrdp::input::MousePosition {
        x: scale(position.x, window_size.width, desktop_size.0),
        y: scale(position.y, window_size.height, desktop_size.1),
    }
}

/// Whether the mouse message being handled is one Windows synthesized from touch or pen input.
///
/// The viewer handles touch and pen through `WindowEvent::Touch`, so these duplicates are dropped.
#[cfg(windows)]
fn is_touch_or_pen_mouse_message() -> bool {
    // `MI_WP_SIGNATURE` in the message extra info marks mouse input promoted from touch or pen.
    const MI_WP_SIGNATURE: isize = 0xFF51_5700;
    const SIGNATURE_MASK: isize = 0xFFFF_FF00;

    // UNTESTED: winit dispatches mouse messages to the handler synchronously, so the extra info belongs to
    // the message being handled.
    // If touch input moves or clicks the remote mouse twice, check that winit still does so.
    // SAFETY: `GetMessageExtraInfo` has no preconditions.
    let extra_info = unsafe { windows::Win32::UI::WindowsAndMessaging::GetMessageExtraInfo() };
    extra_info.0 & SIGNATURE_MASK == MI_WP_SIGNATURE
}

#[cfg(not(windows))]
fn is_touch_or_pen_mouse_message() -> bool {
    false
}

/// Returns false when the session input queue is full and nothing was applied.
fn apply_and_send_fast_path_events(
    input_target: &InputTarget,
    input_database: &mut ironrdp::input::Database,
    operations: impl IntoIterator<Item = ironrdp::input::Operation>,
) -> bool {
    match input_target {
        InputTarget::Direct(input_event_sender) => {
            let permit = match input_event_sender.try_reserve() {
                Ok(permit) => permit,
                Err(tokio::sync::mpsc::error::TrySendError::Full(())) => return false,
                Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => return true,
            };
            let input_events = input_database.apply(operations);
            if !input_events.is_empty() {
                permit.send(RdpInputEvent::FastPath(input_events));
            }
            true
        }
        InputTarget::Rpc(daemon) => {
            let _ = daemon.input_operations(operations);
            true
        }
    }
}
