use std::sync::mpsc::Receiver;
use std::sync::Arc;

use futures_util::AsyncWriteExt;
use glutin::dpi::PhysicalPosition;
use glutin::event::ElementState;
use glutin::event::{Event, WindowEvent};

use ironrdp::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp::input::mouse::{ButtonEvents, MovementEvents, WheelEvents};
use ironrdp::input::MousePdu;
use ironrdp::PduParsing;
use ironrdp_session::ErasedWriter;
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
                    ElementState::Released => KeyboardFlags::FASTPATH_INPUT_KBDFLAGS_RELEASE,
                };
                Some(FastPathInputEvent::KeyboardEvent(flags, scan_code as u8))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(position) = last_position.as_ref() {
                    let button = match button {
                        glutin::event::MouseButton::Left => ButtonEvents::LEFT_BUTTON,
                        glutin::event::MouseButton::Right => ButtonEvents::RIGHT_BUTTON,
                        glutin::event::MouseButton::Middle => ButtonEvents::MIDDLE_BUTTON_OR_WHEEL,
                        glutin::event::MouseButton::Other(_) => ButtonEvents::empty(),
                    };
                    let button_events = button
                        | match state {
                            ElementState::Pressed => ButtonEvents::DOWN,
                            ElementState::Released => ButtonEvents::empty(),
                        };
                    let pdu = MousePdu {
                        x_position: position.x as u16,
                        y_position: position.y as u16,
                        wheel_events: WheelEvents::empty(),
                        movement_events: MovementEvents::empty(),
                        button_events,
                        number_of_wheel_rotations: 0,
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
                    wheel_events: WheelEvents::empty(),
                    movement_events: MovementEvents::MOVE,
                    button_events: ButtonEvents::empty(),
                    number_of_wheel_rotations: 0,
                };

                Some(FastPathInputEvent::MouseEvent(pdu))
            }
            _ => None,
        },
        _ => None,
    }
}
