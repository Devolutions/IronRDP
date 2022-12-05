use std::fmt::Debug;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender};

use std::sync::{self, Arc};

use glutin::dpi::PhysicalPosition;

use glutin::event::{Event, WindowEvent};
use glutin::event_loop::ControlFlow;

use ironrdp::dvc::gfx::ServerPdu;
use ironrdp_session::{DvcHandlerFactory, ErasedWriter, GfxHandler};
use tokio::sync::Mutex;
use ui_core::renderer::Renderer;

use crate::RdpError;

use self::input::{handle_input_events, translate_input_event};

mod input;

#[derive(Debug, Clone)]
pub struct SimpleHandlerFactory {
    gfx_handler: MessagePassingGfxHandler,
}

impl SimpleHandlerFactory {
    pub fn new(gfx_handler: MessagePassingGfxHandler) -> Self {
        Self { gfx_handler }
    }
}

impl DvcHandlerFactory for SimpleHandlerFactory {
    fn graphics_handler(&self) -> Option<Box<dyn ironrdp_session::GfxHandler + Send>> {
        Some(Box::new(self.gfx_handler.clone()))
    }
}

#[derive(Debug, Clone)]
pub struct MessagePassingGfxHandler {
    channel: SyncSender<ServerPdu>,
}

impl MessagePassingGfxHandler {
    pub fn new(channel: SyncSender<ServerPdu>) -> Self {
        Self { channel }
    }
}

impl GfxHandler for MessagePassingGfxHandler {
    fn on_message(&self, message: ServerPdu) -> Result<Option<ironrdp::dvc::gfx::ClientPdu>, RdpError> {
        self.channel
            .send(message)
            .map_err(|e| RdpError::SendError(e.to_string()))?;
        Ok(None)
    }
}

pub struct UiContext {
    window: glutin::ContextWrapper<glutin::NotCurrent, glutin::window::Window>,
    event_loop: glutin::event_loop::EventLoop<UserEvent>,
}

impl UiContext {
    fn create_ui_context(
        width: i32,
        height: i32,
    ) -> (
        glutin::ContextWrapper<glutin::NotCurrent, glutin::window::Window>,
        glutin::event_loop::EventLoop<UserEvent>,
    ) {
        let event_loop = glutin::event_loop::EventLoopBuilder::with_user_event().build();
        let window_builder = glutin::window::WindowBuilder::new()
            .with_title("IronRDP Client")
            .with_resizable(false)
            .with_inner_size(glutin::dpi::PhysicalSize::new(width, height));
        let window = glutin::ContextBuilder::new()
            .with_vsync(true)
            .build_windowed(window_builder, &event_loop)
            .unwrap();
        (window, event_loop)
    }

    pub fn new(width: u16, height: u16) -> Self {
        let (window, event_loop) = UiContext::create_ui_context(width as i32, height as i32);
        UiContext { window, event_loop }
    }
}

#[derive(Debug)]
pub enum UserEvent {}

/// Launches the GUI. Because of the way UI programming works the event loop has to be run from main thread
pub fn launch_gui(
    context: UiContext,
    gfx_dump_file: Option<PathBuf>,
    graphic_receiver: Receiver<ServerPdu>,
    stream: Arc<Mutex<ErasedWriter>>,
) -> Result<(), RdpError> {
    let (sender, receiver) = sync::mpsc::channel();

    tokio::spawn(async move { handle_input_events(receiver, stream).await });

    let renderer = Renderer::new(context.window, graphic_receiver, gfx_dump_file);
    // We handle events differently between targets

    let mut last_position: Option<PhysicalPosition<f64>> = None;
    context.event_loop.run(move |main_event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match &main_event {
            Event::LoopDestroyed => {}
            Event::RedrawRequested(_) => {
                let res = renderer.repaint();
                if res.is_err() {
                    log::error!("Repaint send error: {:?}", res);
                }
            }
            Event::WindowEvent { ref event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::Resized(..) => {
                    // let width = new_size.width;
                    // let height = new_size.height;
                    // let scale_factor = window.window().scale_factor();
                    // info!("Scale factor: {} Window size: {:?}x {:?}", scale_factor, width, height);
                    // let layout_pdu = display::ClientPdu::DisplayControlMonitorLayout(MonitorLayoutPdu {
                    //     monitors: vec![Monitor {
                    //         left: 0,
                    //         top: 0,
                    //         width: width,
                    //         height: height,
                    //         flags: MonitorFlags::PRIMARY,
                    //         physical_width: 0,
                    //         physical_height: 0,
                    //         orientation: Orientation::Landscape,
                    //         desktop_scale_factor: 0,
                    //         device_scale_factor: 0,
                    //     }],
                    // });
                    // let mut data_buffer = Vec::new();
                    // layout_pdu.to_buffer(&mut data_buffer)?;
                    // if let (Some(x224_processor), Some(stream)) = (x224_processor.as_ref(), stream.as_mut()) {
                    //     let mut x224_processor = x224_processor.lock()?;
                    //     // Ignorable eror in case of display channel is not connected
                    //     let result =
                    //         x224_processor.send_dynamic(&mut *stream, x224::RDP8_DISPLAY_PIPELINE_NAME, data_buffer);
                    //     if result.is_err() {
                    //         log::error!("Monitor layour {:?}", result);
                    //     } else {
                    //         log::error!("Monitor layour success");
                    //     }
                    // }
                }
                WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::CursorMoved { .. } => {
                    if let Some(event) = translate_input_event(main_event, &mut last_position) {
                        let result = sender.send(event);
                        if result.is_err() {
                            log::error!("Send of event failed: {:?}", result);
                        }
                    }
                }
                _ => {}
            },
            _ => (),
        }
    })
}
