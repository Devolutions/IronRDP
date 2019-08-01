use lazy_static::lazy_static;

use super::*;

const BRUSH_BUFFER: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

lazy_static! {
    pub static ref BRUSH: Brush = Brush {
        support_level: SupportLevel::Color8x8,
    };
}

#[test]
fn from_buffer_successfully_parses_brush_capset() {
    assert_eq!(Brush::from_buffer(BRUSH_BUFFER.as_ref()).unwrap(), *BRUSH);
}

#[test]
fn to_buffer_successfully_serializes_brush_capset() {
    let mut buffer = Vec::new();

    BRUSH.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, BRUSH_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_input_capset() {
    assert_eq!(BRUSH_BUFFER.len(), BRUSH.buffer_length());
}
