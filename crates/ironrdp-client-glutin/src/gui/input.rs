use std::sync::mpsc::Receiver;
use std::sync::Arc;

use futures_util::AsyncWriteExt;
use glutin::dpi::PhysicalPosition;
use glutin::event::{ElementState, Event, WindowEvent};
use ironrdp::pdu::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp::pdu::input::mouse::PointerFlags;
use ironrdp::pdu::input::MousePdu;
use ironrdp::session::ErasedWriter;
use tokio::sync::Mutex;

use super::UserEvent;

pub async fn handle_input_events(receiver: Receiver<FastPathInputEvent>, event_stream: Arc<Mutex<ErasedWriter>>) {
    loop {
        let mut fastpath_events = Vec::new();
        let event = receiver.recv().unwrap();
        fastpath_events.push(event);
        while let Ok(event) = receiver.try_recv() {
            fastpath_events.push(event);
        }
        let mut data: Vec<u8> = Vec::new();
        let input_pdu = FastPathInput(fastpath_events);
        input_pdu.to_buffer(&mut data).unwrap();
        let mut event_stream = event_stream.lock().await;
        let _result = event_stream.write_all(data.as_slice()).await;
        let _result = event_stream.flush().await;
    }
}

pub fn translate_input_event(
    event: Event<UserEvent>,
    last_position: &mut Option<PhysicalPosition<f64>>,
) -> Option<FastPathInputEvent> {
    match event {
        Event::WindowEvent { ref event, .. } => match event {
            WindowEvent::KeyboardInput {
                device_id: _,
                input,
                is_synthetic: _,
            } => {
                let scan_code = input.scancode & 0xff;

                let flags = match input.state {
                    ElementState::Pressed => KeyboardFlags::empty(),
                    ElementState::Released => KeyboardFlags::RELEASE,
                };
                Some(FastPathInputEvent::KeyboardEvent(flags, scan_code as u8))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(position) = last_position.as_ref() {
                    let button = match button {
                        glutin::event::MouseButton::Left => PointerFlags::LEFT_BUTTON,
                        glutin::event::MouseButton::Right => PointerFlags::RIGHT_BUTTON,
                        glutin::event::MouseButton::Middle => PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
                        glutin::event::MouseButton::Other(_) => PointerFlags::empty(),
                    };
                    let button_events = button
                        | match state {
                            ElementState::Pressed => PointerFlags::DOWN,
                            ElementState::Released => PointerFlags::empty(),
                        };
                    let pdu = MousePdu {
                        x_position: position.x as u16,
                        y_position: position.y as u16,
                        flags: button_events,
                        number_of_wheel_rotation_units: 0,
                    };

                    Some(FastPathInputEvent::MouseEvent(pdu))
                } else {
                    None
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                *last_position = Some(*position);

                let pdu = MousePdu {
                    x_position: position.x as u16,
                    y_position: position.y as u16,
                    flags: PointerFlags::MOVE,
                    number_of_wheel_rotation_units: 0,
                };

                Some(FastPathInputEvent::MouseEvent(pdu))
            }
            _ => None,
        },
        _ => None,
    }
}
