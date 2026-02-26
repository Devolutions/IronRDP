use std::fmt::Debug;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvError, SendError, Sender};
use std::sync::{mpsc, Arc, PoisonError};
use std::thread;
use std::thread::JoinHandle;

use glutin::dpi::PhysicalSize;
use ironrdp::pdu::dvc::gfx;
use ironrdp::pdu::dvc::gfx::{Codec1Type, ServerPdu};
use ironrdp::pdu::geometry::Rectangle;
use thiserror::Error;

#[cfg(feature = "openh264")]
use crate::surface::SurfaceDecoders;
use crate::surface::{DataBuffer, Surfaces};

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

/// Runs the decode loop for graphics PDUs, using Cisco's prebuilt OpenH264
/// binary (loaded at runtime via `libloading`) for H.264 decoding.
#[cfg(feature = "openh264")]
fn handle_gfx_pdu(
    graphic_receiver: Receiver<ServerPdu>,
    gfx_dump_file: Option<PathBuf>,
    openh264_path: PathBuf,
    tx: Sender<RenderEvent>,
) -> Result<(), RendererError> {
    let mut file = gfx_dump_file.map(|file| File::create(file).unwrap());
    let mut decoders = SurfaceDecoders::new(openh264_path);
    loop {
        let message = graphic_receiver
            .recv()
            .map_err(|e| RendererError::ReceiveError(e.to_string()))?;

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
        info!("Got user event {:?}", message);
        match message {
            RenderEvent::Repaint => {
                surfaces.flush_output();
                let result = window.swap_buffers();
                if result.is_err() {
                    error!("Swap buffers error: {:?}", result);
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
                    info!("Ignore message: {:?}", pdu);
                }
            },
        }
    }
}

/// Launches two threads for graphics handling: one decodes EGFX PDUs (using
/// OpenH264 for H.264 when the `openh264` feature is enabled), and one paints
/// decoded frames onto the OpenGL canvas.
#[cfg(feature = "openh264")]
pub struct Renderer {
    render_proxy: Sender<RenderEvent>,
    _decode_thread: JoinHandle<Result<(), RendererError>>,
    _draw_thread: JoinHandle<Result<(), RendererError>>,
}

#[cfg(feature = "openh264")]
impl Renderer {
    pub fn new(
        window: glutin::ContextWrapper<glutin::NotCurrent, glutin::window::Window>,
        graphic_receiver: Receiver<ServerPdu>,
        gfx_dump_file: Option<PathBuf>,
        openh264_path: PathBuf,
    ) -> Renderer {
        let (tx, rx) = mpsc::channel::<RenderEvent>();
        let tx2 = tx.clone();
        let decode_thread = thread::spawn(move || {
            let result = handle_gfx_pdu(graphic_receiver, gfx_dump_file, openh264_path, tx2);
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

#[derive(Debug, Error)]
pub enum RendererError {
    #[error("unable to send message on channel {0}")]
    SendError(String),
    #[error("unable to receive message on channel {0}")]
    ReceiveError(String),
    #[cfg(feature = "openh264")]
    #[error("failed to decode OpenH264 stream {0}")]
    OpenH264Error(#[from] openh264::Error),
    #[error("graphics pipeline protocol error: {0}")]
    GraphicsPipelineError(#[from] gfx::GraphicsPipelineError),
    #[error("invalid surface id: {0}")]
    InvalidSurfaceId(u16),
    #[error("codec not supported: {0:?}")]
    UnsupportedCodec(Codec1Type),
    #[error("failed to decode rdp data")]
    DecodeError,
    #[error("lock poisoned")]
    LockPoisonedError,
}

impl<T> From<SendError<T>> for RendererError {
    fn from(e: SendError<T>) -> Self {
        RendererError::SendError(e.to_string())
    }
}

impl From<RecvError> for RendererError {
    fn from(e: RecvError) -> Self {
        RendererError::ReceiveError(e.to_string())
    }
}

impl<T> From<PoisonError<T>> for RendererError {
    fn from(_e: PoisonError<T>) -> Self {
        RendererError::LockPoisonedError
    }
}
