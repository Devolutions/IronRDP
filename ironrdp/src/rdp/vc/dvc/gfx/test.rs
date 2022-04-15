use lazy_static::lazy_static;

use super::graphics_messages::test::{
    FRAME_ACKNOWLEDGE, FRAME_ACKNOWLEDGE_BUFFER, WIRE_TO_SURFACE_1, WIRE_TO_SURFACE_1_BITMAP_DATA,
    WIRE_TO_SURFACE_1_BUFFER,
};
use super::*;

const WIRE_TO_SURFACE_1_HEADER_BUFFER: [u8; 8] = [0x01, 0x00, 0x00, 0x00, 0xe2, 0x00, 0x00, 0x00];
const FRAME_ACKNOWLEDGE_HEADER_BUFFER: [u8; 8] = [0x0d, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00];

lazy_static! {
    static ref HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER: Vec<u8> =
        { [&WIRE_TO_SURFACE_1_HEADER_BUFFER[..], &WIRE_TO_SURFACE_1_BUFFER[..],].concat() };
    static ref HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER: Vec<u8> =
        { [&FRAME_ACKNOWLEDGE_HEADER_BUFFER[..], &FRAME_ACKNOWLEDGE_BUFFER[..],].concat() };
    static ref HEADER_WITH_WIRE_TO_SURFACE_1: ServerPdu = ServerPdu::WireToSurface1(WIRE_TO_SURFACE_1.clone());
    static ref HEADER_WITH_FRAME_ACKNOWLEDGE: ClientPdu = ClientPdu::FrameAcknowledge(FRAME_ACKNOWLEDGE.clone());
}

#[test]
fn from_buffer_correctly_parses_server_pdu() {
    let mut buffer = HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.as_slice();

    assert_eq!(
        *HEADER_WITH_WIRE_TO_SURFACE_1,
        ServerPdu::from_buffer(&mut buffer).unwrap()
    );
    assert_eq!(WIRE_TO_SURFACE_1_BITMAP_DATA.as_slice(), buffer);
}

#[test]
fn to_buffer_correctly_serializes_server_pdu() {
    let mut buffer = Vec::with_capacity(HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.len());
    HEADER_WITH_WIRE_TO_SURFACE_1.to_buffer(&mut buffer).unwrap();
    buffer.extend_from_slice(WIRE_TO_SURFACE_1_BITMAP_DATA.as_slice());

    assert_eq!(buffer, HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_pdu() {
    assert_eq!(
        HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.len(),
        HEADER_WITH_WIRE_TO_SURFACE_1.buffer_length() + WIRE_TO_SURFACE_1_BITMAP_DATA.len()
    );
}

#[test]
fn from_buffer_correctly_parses_client_pdu() {
    let buffer = HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.as_slice();

    assert_eq!(*HEADER_WITH_FRAME_ACKNOWLEDGE, ClientPdu::from_buffer(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    HEADER_WITH_FRAME_ACKNOWLEDGE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_pdu() {
    assert_eq!(
        HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.len(),
        HEADER_WITH_FRAME_ACKNOWLEDGE.buffer_length()
    );
}
