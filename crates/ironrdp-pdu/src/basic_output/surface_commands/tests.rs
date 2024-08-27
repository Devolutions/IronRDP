use lazy_static::lazy_static;

use super::*;
use crate::geometry::ExclusiveRectangle;
use ironrdp_core::{decode, encode};

const FRAME_MARKER_BUFFER: [u8; 8] = [0x4, 0x0, 0x0, 0x0, 0x5, 0x0, 0x0, 0x0];

const SURFACE_BITS_BUFFER: [u8; 1217] = [
    0x6, 0x0, 0x0, 0x0, 0x0, 0x0, 0x80, 0x7, 0x38, 0x4, 0x20, 0x0, 0x0, 0x3, 0x80, 0x7, 0x38, 0x4, 0xab, 0x4, 0x0, 0x0,
    0xc4, 0xcc, 0xe, 0x0, 0x0, 0x0, 0x1, 0x0, 0x4, 0x0, 0x0, 0x0, 0x1, 0x0, 0xc6, 0xcc, 0x17, 0x0, 0x0, 0x0, 0x1, 0x0,
    0x1, 0x1, 0x0, 0x70, 0x7, 0xa0, 0x0, 0x10, 0x0, 0xc0, 0x0, 0xc1, 0xca, 0x1, 0x0, 0xc7, 0xcc, 0x7e, 0x4, 0x0, 0x0,
    0x1, 0x0, 0xc2, 0xca, 0x0, 0x0, 0x51, 0x50, 0x1, 0x40, 0x4, 0x0, 0x63, 0x4, 0x0, 0x0, 0x66, 0x66, 0x77, 0x88, 0x98,
    0xc3, 0xca, 0x25, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1d, 0x0, 0x2, 0x0, 0x0, 0x1, 0xd, 0x0, 0x5, 0x0, 0x6, 0x41, 0xb8,
    0xbc, 0x5e, 0x7e, 0x2f, 0x3f, 0x17, 0x9f, 0x8b, 0xc3, 0xf9, 0x7c, 0xbe, 0x7e, 0x7e, 0x2f, 0x43, 0xb3, 0x85, 0x68,
    0xe4, 0x70, 0x46, 0x8e, 0x47, 0xa, 0xc8, 0xe4, 0x60, 0x46, 0x47, 0x23, 0x5, 0x64, 0x72, 0x30, 0x23, 0x23, 0x91,
    0x82, 0xb2, 0x39, 0x18, 0x11, 0x91, 0xc8, 0xc1, 0x59, 0x1c, 0x8c, 0x8, 0xc8, 0xe4, 0x60, 0xac, 0x8e, 0x46, 0x4,
    0x64, 0x72, 0x30, 0x56, 0x47, 0x23, 0x2, 0x32, 0x39, 0x18, 0x0, 0xcd, 0xb0, 0x34, 0x1a, 0x1a, 0x1a, 0x34, 0xd6,
    0xb7, 0xe7, 0xc7, 0xc7, 0x4e, 0x9d, 0x3a, 0x69, 0x0, 0x1, 0xf, 0x20, 0xc8, 0x32, 0x19, 0x18, 0x0, 0xf, 0xe6, 0x43,
    0xe4, 0x7c, 0x8f, 0xa7, 0xd7, 0xdf, 0xbf, 0x89, 0x32, 0x8e, 0x82, 0x13, 0xff, 0xe0, 0x84, 0x82, 0x1f, 0xfe, 0x60,
    0x1c, 0xa0, 0x83, 0xff, 0xcc, 0x0, 0xa2, 0x82, 0xf, 0xc6, 0x0, 0x52, 0x82, 0xf, 0xc6, 0x0, 0xa5, 0x4, 0x1f, 0xfe,
    0x60, 0x5, 0x14, 0x10, 0x7e, 0x30, 0x0, 0x18, 0xdc, 0x2e, 0x2e, 0x5c, 0xdb, 0x7f, 0x8f, 0xd3, 0xc9, 0x46, 0x0,
    0x22, 0x10, 0x10, 0xa, 0x13, 0xcb, 0xcb, 0x20, 0x0, 0x7e, 0x11, 0x13, 0xa8, 0x82, 0xd8, 0x8d, 0xc4, 0xc5, 0x88,
    0x4f, 0xf4, 0x9, 0xff, 0xff, 0xd1, 0x6, 0xf8, 0x88, 0x13, 0xe2, 0x20, 0x32, 0x65, 0xaf, 0x1e, 0x38, 0x18, 0x4c,
    0x25, 0x4a, 0xc0, 0x27, 0x80, 0x1a, 0xb, 0xdc, 0x1, 0x47, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xbb, 0x0,
    0xa4, 0xd, 0x1, 0x25, 0x80, 0xa8, 0x18, 0x4a, 0x40, 0x18, 0x0, 0x48, 0xa2, 0xa8, 0x0, 0x10, 0x74, 0xd6, 0x80, 0x8,
    0xc4, 0x1b, 0x89, 0x10, 0x0, 0x28, 0xdf, 0xff, 0xf6, 0xa1, 0xc0, 0x0, 0x70, 0xde, 0x4, 0xf2, 0x0, 0x7, 0xd, 0x73,
    0xe4, 0x0, 0x0, 0x0, 0x1f, 0x10, 0x40, 0xb4, 0x4, 0x85, 0xa0, 0x48, 0xb2, 0x40, 0x0, 0x0, 0x0, 0x8, 0x6, 0x0, 0xc3,
    0xca, 0x30, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1d, 0x0, 0x3, 0x0, 0x13, 0x1, 0x5, 0x0, 0x5, 0x0, 0x6, 0x49, 0xc9,
    0x81, 0xf2, 0x39, 0x30, 0x23, 0x23, 0x91, 0x81, 0x19, 0x1c, 0x8c, 0x15, 0x91, 0xc8, 0xc0, 0x8c, 0x8e, 0x46, 0xa,
    0xc8, 0xe4, 0x60, 0x46, 0x47, 0x23, 0x5, 0x64, 0x72, 0x30, 0x23, 0x23, 0x91, 0x82, 0xb2, 0x39, 0x18, 0x11, 0x91,
    0xc8, 0xc1, 0x59, 0x1c, 0x8c, 0x8, 0xc8, 0xe4, 0x60, 0xac, 0x8e, 0x46, 0x4, 0x64, 0x72, 0x30, 0x56, 0x47, 0x23,
    0x2, 0x32, 0x39, 0x18, 0x2b, 0x23, 0x91, 0x81, 0x19, 0x1c, 0x8c, 0x15, 0x91, 0xc8, 0xc0, 0x8c, 0x8e, 0x46, 0xa,
    0xc8, 0xe4, 0x60, 0x46, 0x47, 0x23, 0x5, 0x64, 0x72, 0x30, 0x23, 0x23, 0x91, 0x82, 0xb2, 0x39, 0x18, 0x11, 0x91,
    0xc8, 0xc1, 0x59, 0x1c, 0x8c, 0x8, 0xc8, 0xe4, 0x60, 0xac, 0x8e, 0x46, 0x4, 0x64, 0x72, 0x30, 0x0, 0x4, 0x19, 0x48,
    0x0, 0x40, 0x8, 0x7, 0xff, 0x46, 0x24, 0x1, 0x0, 0x83, 0xef, 0xa, 0x4, 0x13, 0xe8, 0x42, 0x41, 0xf, 0xff, 0x30,
    0xe, 0x50, 0x41, 0xff, 0xe6, 0x0, 0x51, 0x41, 0x7, 0xe3, 0x0, 0x29, 0x41, 0x7, 0xe3, 0x0, 0x52, 0x82, 0xf, 0xff,
    0x30, 0x2, 0x8a, 0x8, 0x3f, 0x18, 0x1, 0x4a, 0x8, 0x3f, 0x18, 0x2, 0x94, 0x10, 0x7f, 0xf9, 0x80, 0x14, 0x50, 0x41,
    0xf8, 0xc0, 0xa, 0x50, 0x41, 0xf8, 0xc0, 0x14, 0xa0, 0x83, 0xff, 0xcc, 0x0, 0xa2, 0x82, 0xf, 0xc6, 0x0, 0x52, 0x82,
    0xf, 0xc6, 0x0, 0x0, 0x10, 0x7f, 0x88, 0x1, 0x46, 0xf8, 0x80, 0x53, 0x70, 0x80, 0xbb, 0x84, 0x13, 0x70, 0x84, 0x6e,
    0x10, 0x80, 0x9f, 0x10, 0x81, 0xbe, 0x21, 0x0, 0x1, 0x9, 0xff, 0xff, 0xff, 0xfd, 0x4b, 0xd, 0xb, 0xc8, 0x20, 0xf6,
    0x1a, 0x5e, 0x4c, 0x32, 0xc3, 0x5b, 0x9e, 0x44, 0x40, 0x5, 0xd, 0x6e, 0x79, 0x11, 0x0, 0x14, 0x37, 0xf8, 0x27,
    0x90, 0x0, 0x38, 0x6f, 0x2, 0x79, 0x0, 0x3, 0x86, 0xff, 0x4, 0xf2, 0x0, 0x7, 0xd, 0x73, 0xe4, 0x0, 0x0, 0x0, 0x8,
    0x6, 0x0, 0x0, 0x0, 0x8, 0x6, 0x0, 0xc3, 0xca, 0x30, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1d, 0x0, 0x4, 0x0, 0x13, 0x1,
    0x5, 0x0, 0x5, 0x0, 0x6, 0x49, 0xc9, 0x81, 0xf2, 0x39, 0x30, 0x23, 0x23, 0x91, 0x81, 0x19, 0x1c, 0x8c, 0x15, 0x91,
    0xc8, 0xc0, 0x8c, 0x8e, 0x46, 0xa, 0xc8, 0xe4, 0x60, 0x46, 0x47, 0x23, 0x5, 0x64, 0x72, 0x30, 0x23, 0x23, 0x91,
    0x82, 0xb2, 0x39, 0x18, 0x11, 0x91, 0xc8, 0xc1, 0x59, 0x1c, 0x8c, 0x8, 0xc8, 0xe4, 0x60, 0xac, 0x8e, 0x46, 0x4,
    0x64, 0x72, 0x30, 0x56, 0x47, 0x23, 0x2, 0x32, 0x39, 0x18, 0x2b, 0x23, 0x91, 0x81, 0x19, 0x1c, 0x8c, 0x15, 0x91,
    0xc8, 0xc0, 0x8c, 0x8e, 0x46, 0xa, 0xc8, 0xe4, 0x60, 0x46, 0x47, 0x23, 0x5, 0x64, 0x72, 0x30, 0x23, 0x23, 0x91,
    0x82, 0xb2, 0x39, 0x18, 0x11, 0x91, 0xc8, 0xc1, 0x59, 0x1c, 0x8c, 0x8, 0xc8, 0xe4, 0x60, 0xac, 0x8e, 0x46, 0x4,
    0x64, 0x72, 0x30, 0x0, 0x4, 0x19, 0x48, 0x0, 0x40, 0x8, 0x7, 0xff, 0x46, 0x24, 0x1, 0x0, 0x83, 0xef, 0xa, 0x4,
    0x13, 0xe8, 0x42, 0x41, 0xf, 0xff, 0x30, 0xe, 0x50, 0x41, 0xff, 0xe6, 0x0, 0x51, 0x41, 0x7, 0xe3, 0x0, 0x29, 0x41,
    0x7, 0xe3, 0x0, 0x52, 0x82, 0xf, 0xff, 0x30, 0x2, 0x8a, 0x8, 0x3f, 0x18, 0x1, 0x4a, 0x8, 0x3f, 0x18, 0x2, 0x94,
    0x10, 0x7f, 0xf9, 0x80, 0x14, 0x50, 0x41, 0xf8, 0xc0, 0xa, 0x50, 0x41, 0xf8, 0xc0, 0x14, 0xa0, 0x83, 0xff, 0xcc,
    0x0, 0xa2, 0x82, 0xf, 0xc6, 0x0, 0x52, 0x82, 0xf, 0xc6, 0x0, 0x0, 0x10, 0x7f, 0x88, 0x1, 0x46, 0xf8, 0x80, 0x53,
    0x70, 0x80, 0xbb, 0x84, 0x13, 0x70, 0x84, 0x6e, 0x10, 0x80, 0x9f, 0x10, 0x81, 0xbe, 0x21, 0x0, 0x1, 0x9, 0xff,
    0xff, 0xff, 0xfd, 0x4b, 0xd, 0xb, 0xc8, 0x20, 0xf6, 0x1a, 0x5e, 0x4c, 0x32, 0xc3, 0x5b, 0x9e, 0x44, 0x40, 0x5, 0xd,
    0x6e, 0x79, 0x11, 0x0, 0x14, 0x37, 0xf8, 0x27, 0x90, 0x0, 0x38, 0x6f, 0x2, 0x79, 0x0, 0x3, 0x86, 0xff, 0x4, 0xf2,
    0x0, 0x7, 0xd, 0x73, 0xe4, 0x0, 0x0, 0x0, 0x8, 0x6, 0x0, 0x0, 0x0, 0x8, 0x6, 0x0, 0xc3, 0xca, 0xde, 0x0, 0x0, 0x0,
    0x0, 0x0, 0x0, 0x1d, 0x0, 0x5, 0x0, 0xc1, 0x0, 0x5, 0x0, 0x5, 0x0, 0x6, 0x49, 0xc9, 0x81, 0xf2, 0x39, 0x30, 0x23,
    0x23, 0x91, 0x81, 0x19, 0x1c, 0x8c, 0x15, 0x91, 0xc8, 0xc0, 0x8c, 0x8e, 0x46, 0xa, 0xc8, 0xe4, 0x60, 0x46, 0x47,
    0x23, 0x5, 0x64, 0x72, 0x30, 0x23, 0xb, 0xc5, 0xe7, 0xe2, 0xf3, 0xf1, 0x79, 0xf8, 0xbc, 0xfc, 0x5e, 0x7e, 0x2f,
    0x3f, 0x17, 0x9f, 0x8b, 0xcf, 0xc5, 0xe7, 0xe2, 0xf3, 0xf1, 0x78, 0xc, 0x1e, 0x81, 0xd0, 0x74, 0x3a, 0x18, 0x0,
    0xb, 0xf5, 0x20, 0x1, 0x0, 0x20, 0x1f, 0xfd, 0x31, 0x20, 0x8, 0x4, 0x1f, 0x54, 0x28, 0x10, 0x47, 0xd0, 0x84, 0x82,
    0x13, 0xe8, 0x19, 0x21, 0xf, 0xff, 0x30, 0x5, 0xa4, 0x81, 0x52, 0x7d, 0x2e, 0x97, 0x5f, 0x4b, 0xcf, 0xc5, 0xe7,
    0xe2, 0xf0, 0x60, 0x40, 0x80, 0x8a, 0x1e, 0x8f, 0x4c, 0x0, 0x4, 0x4, 0x85, 0x9, 0x6f, 0xff, 0xfa, 0x1, 0x4d, 0xf1,
    0x2, 0xb7, 0xa8, 0x14, 0x84, 0x92, 0x5, 0xa6, 0x16, 0x84, 0x16, 0x8c, 0x5e, 0x1, 0x24, 0x3f, 0xff, 0x80, 0x98,
    0xe1, 0x2, 0x7, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xf2, 0xb0, 0x1a, 0x21, 0xe4, 0x4, 0x35, 0x80, 0xd2,
    0x3c, 0x84, 0x30, 0x3, 0x81, 0xa9, 0xa3, 0xc7, 0x80, 0xc, 0x0, 0x41, 0x24, 0x20, 0x0, 0x5, 0x12, 0xaa, 0xa0, 0x1,
    0xc4, 0xe5, 0x54, 0x0, 0x72, 0x72, 0xaa, 0x0, 0x72, 0x4b, 0x50, 0x0, 0x0, 0x8, 0x6, 0x0, 0x0, 0x0, 0x8, 0x6, 0x0,
    0xc5, 0xcc, 0x8, 0x0, 0x0, 0x0, 0x1, 0x0,
];

const FRAME_MARKER_PDU: SurfaceCommand<'_> = SurfaceCommand::FrameMarker(FrameMarkerPdu {
    frame_action: FrameAction::Begin,
    frame_id: Some(5),
});

lazy_static! {
    static ref SURFACE_BITS_PDU: SurfaceCommand<'static> = SurfaceCommand::StreamSurfaceBits(SurfaceBitsPdu {
        destination: ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        },
        extended_bitmap_data: ExtendedBitmapDataPdu {
            bpp: 32,
            codec_id: 3,
            width: 1920,
            height: 1080,
            header: None,
            data: &SURFACE_BITS_BUFFER[22..],
        },
    });
}

#[test]
fn from_buffer_correctly_parses_surface_command_frame_marker() {
    assert_eq!(
        FRAME_MARKER_PDU,
        decode::<SurfaceCommand<'_>>(FRAME_MARKER_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_surface_command_frame_marker() {
    let expected = FRAME_MARKER_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FRAME_MARKER_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_surface_command_frame_marker() {
    assert_eq!(FRAME_MARKER_BUFFER.len(), FRAME_MARKER_PDU.size());
}

#[test]
fn from_buffer_correctly_parses_surface_command_bits() {
    assert_eq!(
        *SURFACE_BITS_PDU,
        decode::<SurfaceCommand<'_>>(SURFACE_BITS_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_surface_command_bits() {
    let expected = SURFACE_BITS_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&*SURFACE_BITS_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_surface_command_bits() {
    assert_eq!(SURFACE_BITS_BUFFER.len(), SURFACE_BITS_PDU.size());
}
