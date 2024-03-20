use lazy_static::lazy_static;

use super::*;
use crate::{decode, encode_vec};

const BITMAP_BUFFER: [u8; 24] = [
    0x18, 0x00, // preferredBitsPerPixel
    0x01, 0x00, // receive1BitPerPixel
    0x01, 0x00, // receive4BitPerPixel
    0x01, 0x00, // receive8BitPerPixel,
    0x00, 0x05, // desktopWidth
    0x00, 0x04, // desktopHeight
    0x00, 0x00, // pad2octets
    0x01, 0x00, // desktopResizeFlag
    0x01, 0x00, // bitmapCompressionFlag
    0x00, // highColorFlags,
    0x08, // drawingFlags,
    0x01, 0x00, // multipleRectangleSupport
    0x00, 0x00, // pad2octetsB
];

lazy_static! {
    pub static ref BITMAP: Bitmap = Bitmap {
        pref_bits_per_pix: 24,
        desktop_width: 1280,
        desktop_height: 1024,
        desktop_resize_flag: true,
        drawing_flags: BitmapDrawingFlags::ALLOW_SKIP_ALPHA,
    };
}

#[test]
fn from_buffer_correctly_parses_bitmap_capset() {
    let buffer = BITMAP_BUFFER.as_ref();

    assert_eq!(*BITMAP, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_bitmap_capset() {
    let capset = BITMAP.clone();

    let buffer = encode_vec(&capset).unwrap();

    assert_eq!(buffer, BITMAP_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_bitmap_capset() {
    let correct_buffer_length = BITMAP_BUFFER.len();

    assert_eq!(correct_buffer_length, BITMAP.size());
}
