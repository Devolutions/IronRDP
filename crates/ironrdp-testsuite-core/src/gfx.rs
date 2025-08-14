use ironrdp_pdu::rdp::vc::dvc::gfx::{ClientPdu, ServerPdu};

use crate::graphics_messages::{
    FRAME_ACKNOWLEDGE, FRAME_ACKNOWLEDGE_BUFFER, WIRE_TO_SURFACE_1, WIRE_TO_SURFACE_1_BUFFER,
};

pub const WIRE_TO_SURFACE_1_HEADER_BUFFER: [u8; 8] = [0x01, 0x00, 0x00, 0x00, 0xe2, 0x00, 0x00, 0x00];
pub const FRAME_ACKNOWLEDGE_HEADER_BUFFER: [u8; 8] = [0x0d, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00];

lazy_static! {
    pub static ref HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER: Vec<u8> =
        [&WIRE_TO_SURFACE_1_HEADER_BUFFER[..], &WIRE_TO_SURFACE_1_BUFFER[..],].concat();
    pub static ref HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER: Vec<u8> =
        [&FRAME_ACKNOWLEDGE_HEADER_BUFFER[..], &FRAME_ACKNOWLEDGE_BUFFER[..],].concat();
    pub static ref HEADER_WITH_WIRE_TO_SURFACE_1: ServerPdu = ServerPdu::WireToSurface1(WIRE_TO_SURFACE_1.clone());
    pub static ref HEADER_WITH_FRAME_ACKNOWLEDGE: ClientPdu = ClientPdu::FrameAcknowledge(FRAME_ACKNOWLEDGE.clone());
}
