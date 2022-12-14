use std::fmt::Debug;
use std::fs::File;

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvError;
use std::sync::mpsc::SendError;
use std::sync::mpsc::Sender;

use std::sync::PoisonError;
use std::sync::{mpsc, Arc};
use std::thread;
use std::thread::JoinHandle;

use failure::Fail;
use glutin::dpi::PhysicalSize;
use ironrdp::PduParsing;

use ironrdp::dvc::gfx;
use ironrdp::dvc::gfx::Codec1Type;
use ironrdp::dvc::gfx::ServerPdu;

use ironrdp::impl_from_error;
use ironrdp::Rectangle;
use log::info;

use crate::surface::DataBuffer;

use crate::surface::SurfaceDecoders;
use crate::surface::Surfaces;

#[derive(Debug)]
enum RenderEvent {
    Paint((u16, DataBuffer)),
    Repaint,
    ServerPdu(ServerPdu),
}

#[derive(Clone)]
struct DataRegion {
    pub data: Vec<u8>,
    pub regions: Vec<Rectangle>,
}

impl Debug for DataRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataRegion")
            .field("data_len", &self.data.len())
            .field("regions", &self.regions)
            .finish()
    }
}

/// Runs the decode loop to decode any graphics PDU
fn handle_gfx_pdu(
    graphic_receiver: Receiver<ServerPdu>,
    gfx_dump_file: Option<PathBuf>,
    tx: Sender<RenderEvent>,
) -> Result<(), RendererError> {
    let mut file = gfx_dump_file.map(|file| File::create(file).unwrap());
    let mut decoders = SurfaceDecoders::new();
    loop {
        let message = graphic_receiver
            .recv()
            .map_err(|e| RendererError::RecieveError(e.to_string()))?;

        if let Some(file) = file.as_mut() {
            let _result = message.to_buffer(file);
        };
        match &message {
            ServerPdu::WireToSurface1(pdu) => {
                let surface_id = pdu.surface_id;
                let decoded = decoders.decode_wire_to_surface_1_pdu(pdu)?;
                tx.send(RenderEvent::Paint((surface_id, decoded)))?;
            }
            ServerPdu::CreateSurface(pdu) => {
                decoders.add(pdu.surface_id)?;
            }
            ServerPdu::DeleteSurface(pdu) => {
                decoders.remove(pdu.surface_id)?;
            }
            _ => {}
        };

        if !matches!(message, ServerPdu::WireToSurface1(..)) {
            tx.send(RenderEvent::ServerPdu(message))?;
        }
    }
}

/// Runs the paint loop to paint the decoded PDU onto the canvas
fn handle_draw(
    window: glutin::ContextWrapper<glutin::NotCurrent, glutin::window::Window>,
    rx: Receiver<RenderEvent>,
) -> Result<(), RendererError> {
    let window = unsafe { window.make_current().unwrap() };
    let shader_version = "#version 410";
    let gl = unsafe { glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _) };
    let gl = Arc::new(gl);
    let mut surfaces = Surfaces::new();
    loop {
        let message = rx.recv()?;
        log::info!("Got user event {:?}", message);
        match message {
            RenderEvent::Repaint => {
                surfaces.flush_output();
                let result = window.swap_buffers();
                if result.is_err() {
                    log::error!("Swap buffers error: {:?}", result);
                }
            }
            RenderEvent::Paint((surface_id, data)) => {
                surfaces.draw_scene(surface_id, data)?;
            }
            RenderEvent::ServerPdu(pdu) => match pdu {
                ServerPdu::CreateSurface(pdu) => {
                    surfaces.create_surface(pdu, gl.clone(), shader_version)?;
                }
                ServerPdu::DeleteSurface(pdu) => {
                    surfaces.delete_surface(pdu.surface_id);
                }
                ServerPdu::MapSurfaceToScaledOutput(pdu) => {
                    surfaces.map_surface_to_scaled_output(pdu)?;
                }
                ServerPdu::EndFrame(_) => {
                    window.window().request_redraw();
                }
                ServerPdu::ResetGraphics(pdu) => {
                    window.window().set_inner_size(PhysicalSize {
                        width: pdu.width,
                        height: pdu.height,
                    });
                }
                _ => {
                    log::info!("Ignore message: {:?}", pdu);
                }
            },
        }
    }
}

/// The renderer launches two threads to handle graphics messages.
/// The first thread takes any graphics PDU and decodes the messages.
/// The second thread paints the messages onto the canvas
pub struct Renderer {
    render_proxy: Sender<RenderEvent>,
    _decode_thread: JoinHandle<Result<(), RendererError>>,
    _draw_thread: JoinHandle<Result<(), RendererError>>,
}

impl Renderer {
    pub fn new(
        window: glutin::ContextWrapper<glutin::NotCurrent, glutin::window::Window>,
        graphic_receiver: Receiver<ServerPdu>,
        gfx_dump_file: Option<PathBuf>,
    ) -> Renderer {
        let (tx, rx) = mpsc::channel::<RenderEvent>();
        let tx2 = tx.clone();
        let decode_thread = thread::spawn(move || {
            let result = handle_gfx_pdu(graphic_receiver, gfx_dump_file, tx2);
            info!("Graphics handler result: {:?}", result);
            result
        });
        let draw_thread = thread::spawn(move || {
            let result = handle_draw(window, rx);
            info!("Draw handler result: {:?}", result);
            result
        });

        Renderer {
            render_proxy: tx,
            _decode_thread: decode_thread,
            _draw_thread: draw_thread,
        }
    }

    pub fn repaint(&self) -> Result<(), RendererError> {
        self.render_proxy.send(RenderEvent::Repaint)?;
        Ok(())
    }
}

#[derive(Debug, Fail)]
pub enum RendererError {
    #[fail(display = "Unable to send message on channel {}", _0)]
    SendError(String),
    #[fail(display = "Unable to recieve message on channel {}", _0)]
    RecieveError(String),
    #[fail(display = "Failed to decode openh264 stream {}", _0)]
    OpenH264Error(openh264::Error),
    #[fail(display = "Graphics pipeline protocol error: {}", _0)]
    GraphicsPipelineError(gfx::GraphicsPipelineError),
    #[fail(display = "Invalid surface id: {}", _0)]
    InvalidSurfaceId(u16),
    #[fail(display = "Codec not supported: {:?}", _0)]
    UnsupportedCodec(Codec1Type),
    #[fail(display = "Failed to decode rdp data")]
    DecodeError,
    #[fail(display = "Lock poisoned")]
    LockPoisonedError,
}

impl_from_error!(
    gfx::GraphicsPipelineError,
    RendererError,
    RendererError::GraphicsPipelineError
);

impl_from_error!(openh264::Error, RendererError, RendererError::OpenH264Error);

impl<T> From<SendError<T>> for RendererError {
    fn from(e: SendError<T>) -> Self {
        RendererError::SendError(e.to_string())
    }
}

impl From<RecvError> for RendererError {
    fn from(e: RecvError) -> Self {
        RendererError::RecieveError(e.to_string())
    }
}

impl<T> From<PoisonError<T>> for RendererError {
    fn from(_e: PoisonError<T>) -> Self {
        RendererError::LockPoisonedError
    }
}
