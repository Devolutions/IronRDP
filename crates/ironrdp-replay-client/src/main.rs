#[macro_use]
extern crate tracing;

use std::fs::File;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::exit;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

use clap::Parser;
use glutin::dpi::PhysicalSize;
use glutin::event::{Event, WindowEvent};
use glutin::event_loop::ControlFlow;
use ironrdp::pdu::dvc::gfx::{GraphicsPipelineError, ServerPdu};
use ironrdp_glutin_renderer::renderer::Renderer;

pub type Error = Box<dyn core::error::Error + Send + Sync + 'static>;

/// Devolutions IronRDP client
#[derive(Parser, Debug)]
#[clap(version, long_about = None)]
struct Args {
    /// A file to use for the data file
    #[clap(long, value_parser)]
    data_file: PathBuf,

    ////// Frame rate
    #[clap(long, value_parser, default_value_t = 1)]
    frame_rate: u32,

    // Close on completion
    #[clap(long, value_parser)]
    close: bool,
}

pub enum UserEvent {}

fn create_ui_context() -> (
    glutin::ContextWrapper<glutin::NotCurrent, glutin::window::Window>,
    glutin::event_loop::EventLoop<UserEvent>,
) {
    let event_loop = glutin::event_loop::EventLoopBuilder::with_user_event().build();
    let window_builder = glutin::window::WindowBuilder::new()
        .with_title("RDP Replay Helper!")
        .with_resizable(false)
        .with_inner_size(PhysicalSize { width: 0, height: 0 });
    let window = glutin::ContextBuilder::new()
        .with_vsync(true)
        .build_windowed(window_builder, &event_loop)
        .unwrap();
    (window, event_loop)
}

pub fn main() -> Result<(), Error> {
    tracing_subscriber::fmt().compact().init();

    let args = Args::parse();

    let (sender, receiver) = sync_channel(1);

    let (window, event_loop) = create_ui_context();
    let renderer = Renderer::new(window, receiver, None);

    thread::spawn(move || {
        let result = handle_file(sender, args);
        info!("Result: {:?}", result);
    });

    event_loop.run(move |main_event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match &main_event {
            Event::LoopDestroyed => {}
            Event::RedrawRequested(_) => {
                let res = renderer.repaint();
                if res.is_err() {
                    error!("Repaint send error: {:?}", res);
                }
            }
            Event::WindowEvent { ref event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::Resized(size) => {
                    info!("Window resized {:?}", size);
                }
                _ => {}
            },
            _ => (),
        }
    });
}

// Parse the graphics file and send it to renderer 1 event at a time
fn handle_file(sender: std::sync::mpsc::SyncSender<ServerPdu>, args: Args) -> Result<(), Error> {
    let file = File::open(args.data_file).unwrap();
    let delay = 1000 / args.frame_rate as u64;

    loop {
        let packet = ServerPdu::from_buffer(&file);
        if let Ok(packet) = packet {
            let wait = matches!(packet, ServerPdu::WireToSurface1(..));
            sender.send(packet)?;
            if wait {
                thread::sleep(Duration::from_millis(delay));
            }
        } else {
            let ignorable = if let Err(GraphicsPipelineError::IOError(e)) = packet.as_ref() {
                e.kind() == ErrorKind::UnexpectedEof
            } else {
                false
            };

            if !ignorable {
                error!("Error: {:?}", packet);
            }

            if args.close {
                exit(0);
            }
            return Err(Error::from("S".to_string()));
        }
    }
}
