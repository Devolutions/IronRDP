use ironrdp_core::{decode, encode};
use lazy_static::lazy_static;

use super::*;
use crate::geometry::InclusiveRectangle;

const FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER: [u8; 2] = [0x80, 0x08];
const FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER: [u8; 3] = [0x80, 0x81, 0xE7];
const FAST_PATH_UPDATE_PDU_BUFFER: [u8; 19] = [
    0x4, 0x10, 0x0, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x4, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0,
];
const FAST_PATH_UPDATE_PDU_WITH_LONG_LEN_BUFFER: [u8; 19] = [
    0x4, 0xff, 0x0, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x4, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0,
];
const FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER: [u8; 3] = [0x80, 0x80, 0x08];

const FAST_PATH_HEADER_WITH_SHORT_LEN_PDU: FastPathHeader = FastPathHeader {
    flags: EncryptionFlags::ENCRYPTED,
    data_length: 6,
    forced_long_length: false,
};
const FAST_PATH_HEADER_WITH_LONG_LEN_PDU: FastPathHeader = FastPathHeader {
    flags: EncryptionFlags::ENCRYPTED,
    data_length: 484,
    forced_long_length: false,
};
const FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU: FastPathHeader = FastPathHeader {
    flags: EncryptionFlags::ENCRYPTED,
    data_length: 5,
    forced_long_length: true,
};

lazy_static! {
    static ref FAST_PATH_UPDATE_PDU: FastPathUpdatePdu<'static> = FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::SurfaceCommands,
        compression_flags: None,
        compression_type: None,
        data: &FAST_PATH_UPDATE_PDU_BUFFER[3..],
    };
}

#[test]
fn from_buffer_correctly_parses_fast_path_header_with_short_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_SHORT_LEN_PDU,
        decode::<FastPathHeader>(FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_fast_path_header_with_short_length() {
    let expected = FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FAST_PATH_HEADER_WITH_SHORT_LEN_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_header_with_short_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER.len(),
        FAST_PATH_HEADER_WITH_SHORT_LEN_PDU.size()
    );
}

#[test]
fn from_buffer_correctly_parses_fast_path_header_with_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_LONG_LEN_PDU,
        decode::<FastPathHeader>(FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_fast_path_header_with_long_length() {
    let expected = FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FAST_PATH_HEADER_WITH_LONG_LEN_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_header_with_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER.len(),
        FAST_PATH_HEADER_WITH_LONG_LEN_PDU.size()
    );
}

#[test]
fn from_buffer_correctly_parses_fast_path_header_with_forced_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU,
        decode::<FastPathHeader>(FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_fast_path_header_with_forced_long_length() {
    let expected = FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_header_with_forced_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER.len(),
        FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU.size()
    );
}

#[test]
fn from_buffer_correctly_parses_fast_path_update() {
    assert_eq!(
        *FAST_PATH_UPDATE_PDU,
        decode::<FastPathUpdatePdu<'_>>(FAST_PATH_UPDATE_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn from_buffer_returns_error_on_long_length_for_fast_path_update() {
    assert!(decode::<FastPathUpdatePdu<'_>>(FAST_PATH_UPDATE_PDU_WITH_LONG_LEN_BUFFER.as_ref()).is_err());
}

#[test]
fn to_buffer_correctly_serializes_fast_path_update() {
    let expected = FAST_PATH_UPDATE_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&*FAST_PATH_UPDATE_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_update() {
    assert_eq!(FAST_PATH_UPDATE_PDU_BUFFER.len(), FAST_PATH_UPDATE_PDU.size());
}

#[test]
fn buffer_size_boundary_fast_path_update() {
    let fph = FastPathHeader {
        flags: EncryptionFlags::ENCRYPTED,
        data_length: 125,
        forced_long_length: false,
    };
    assert_eq!(fph.size(), 2);
    let fph = FastPathHeader {
        flags: EncryptionFlags::ENCRYPTED,
        data_length: 126,
        forced_long_length: false,
    };
    assert_eq!(fph.size(), 3);
}

#[test]
fn decode_fast_path_bitmap_update_with_single_uncompressed_16bpp_rect() {
    // Fast-Path Bitmap data layout:
    // u16 flags, u16 numberRectangles, then one TS_BITMAP_DATA rectangle
    // TS_BITMAP_DATA (uncompressed):
    // InclusiveRectangle (8 bytes), width(u16), height(u16), bpp(u16), flags(u16)=0,
    // dataLen(u16)=2, data[2]
    let data: [u8; 2 + 2 + 8 + 2 + 2 + 2 + 2 + 2 + 2] = [
        // flags
        0x00, 0x00,
        // numberRectangles
        0x01, 0x00,
        // rectangle: left, top, right, bottom (all zeros)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        // width=1, height=1
        0x01, 0x00, 0x01, 0x00,
        // bpp = 16
        0x10, 0x00,
        // compression flags = 0 (uncompressed)
        0x00, 0x00,
        // data length = 2
        0x02, 0x00,
        // pixel data
        0xAA, 0xBB,
    ];

    let update = FastPathUpdate::decode_with_code(&data, UpdateCode::Bitmap).expect("decode fast-path bitmap");

    match update {
        FastPathUpdate::Bitmap(bmp) => {
            assert_eq!(bmp.rectangles.len(), 1);
            let r = &bmp.rectangles[0];
            assert_eq!(r.rectangle, InclusiveRectangle { left: 0, top: 0, right: 0, bottom: 0 });
            assert_eq!(r.width, 1);
            assert_eq!(r.height, 1);
            assert_eq!(r.bits_per_pixel, 16);
            assert!(r.compression_flags.is_empty());
            assert!(r.compressed_data_header.is_none());
            assert_eq!(r.bitmap_data, &[0xAA, 0xBB]);
        }
        _ => panic!("expected Bitmap update"),
    }
}

#[test]
fn encode_fast_path_bitmap_update_with_single_uncompressed_16bpp_rect() {
    // Build a FastPathUpdate::Bitmap with a single uncompressed 16bpp TS_BITMAP_DATA and
    // verify it encodes to the expected fast-path payload bytes (flags + nrect + rect).
    let rect = InclusiveRectangle { left: 0, top: 0, right: 0, bottom: 0 };
    let bmp = BitmapUpdateData {
        rectangles: vec![super::super::bitmap::BitmapData {
            rectangle: rect,
            width: 1,
            height: 1,
            bits_per_pixel: 16,
            compression_flags: super::super::bitmap::Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &[0xAA, 0xBB],
        }],
    };
    let update = FastPathUpdate::Bitmap(bmp);

    let mut buf = vec![0u8; update.size()];
    encode(&update, &mut buf).expect("encode fast-path bitmap");

    let expected: [u8; 2 + 2 + 8 + 2 + 2 + 2 + 2 + 2 + 2] = [
        0x00, 0x00, // flags
        0x01, 0x00, // numberRectangles
        0x00, 0x00, 0x00, 0x00, // left, top
        0x00, 0x00, 0x00, 0x00, // right, bottom
        0x01, 0x00, // width
        0x01, 0x00, // height
        0x10, 0x00, // bpp
        0x00, 0x00, // compression flags
        0x02, 0x00, // data length
        0xAA, 0xBB, // data
    ];

    assert_eq!(buf, expected);
}
