use expect_test::expect;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::pointer::{
    CachedPointerAttribute, ColorPointerAttribute, LargePointerAttribute, Point16, PointerAttribute,
    PointerPositionAttribute,
};

fn expect_pointer_png(pointer: &DecodedPointer, expected_file_path: &str) {
    let path = format!("{}/test_data/{}", env!("CARGO_MANIFEST_DIR"), expected_file_path);

    if std::env::var("UPDATE_EXPECT").unwrap_or_default() == "1" {
        let mut encoded_png = vec![];

        let mut png = png::Encoder::new(&mut encoded_png, pointer.width as u32, pointer.height as u32);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&pointer.bitmap_data)
            .unwrap();
        std::fs::write(path, &encoded_png).unwrap();
        return;
    }

    if !std::path::Path::new(&path).exists() {
        panic!("Test file {} does not exist", path);
    }

    let png_buffer = std::fs::read(path).unwrap();
    let mut png_reader = png::Decoder::new(&png_buffer[..]).read_info().unwrap();
    let mut png_reader_buffer = vec![0u8; png_reader.output_buffer_size()];
    let frame_size = png_reader.next_frame(&mut png_reader_buffer).unwrap().buffer_size();
    let expected = &png_reader_buffer[..frame_size];
    assert_eq!(expected, &pointer.bitmap_data);
}

#[test]
fn new_pointer_32bpp() {
    let data = include_bytes!("../../test_data/pdu/pointer/new_pointer_32bpp.bin");
    let mut parsed = ironrdp_pdu::decode::<PointerAttribute>(data).unwrap();
    let decoded = DecodedPointer::decode_pointer_attribute(&parsed).unwrap();
    expect_pointer_png(&decoded, "pdu/pointer/new_pointer_32bpp.png");

    let mut encoded = vec![];
    ironrdp_pdu::encode_buf(&parsed, &mut encoded).unwrap();
    assert_eq!(&encoded, data);

    parsed.color_pointer.and_mask = &[];
    parsed.color_pointer.xor_mask = &[];
    expect![[r#"
        PointerAttribute {
            xor_bpp: 32,
            color_pointer: ColorPointerAttribute {
                cache_index: 0,
                hot_spot: Point16 {
                    x: 3,
                    y: 3,
                },
                width: 41,
                height: 39,
                xor_mask: [],
                and_mask: [],
            },
        }
    "#]]
    .assert_debug_eq(&parsed);
}

#[test]
fn large_pointer_32bpp() {
    let data = include_bytes!("../../test_data/pdu/pointer/large_pointer_32bpp.bin");
    let mut parsed = ironrdp_pdu::decode::<LargePointerAttribute>(data).unwrap();
    let decoded = DecodedPointer::decode_large_pointer_attribute(&parsed).unwrap();
    expect_pointer_png(&decoded, "pdu/pointer/large_pointer_32bpp.png");

    let mut encoded = vec![];
    ironrdp_pdu::encode_buf(&parsed, &mut encoded).unwrap();
    assert_eq!(&encoded, data);

    parsed.and_mask = &[];
    parsed.xor_mask = &[];
    expect![[r#"
        LargePointerAttribute {
            xor_bpp: 32,
            cache_index: 12,
            hot_spot: Point16 {
                x: 2,
                y: 0,
            },
            width: 112,
            height: 112,
            xor_mask: [],
            and_mask: [],
        }
    "#]]
    .assert_debug_eq(&parsed);
}

#[test]
fn color_pointer_24bpp() {
    let data = include_bytes!("../../test_data/pdu/pointer/color_pointer_24bpp.bin");
    let mut parsed = ironrdp_pdu::decode::<ColorPointerAttribute>(data).unwrap();
    let decoded = DecodedPointer::decode_color_pointer_attribute(&parsed).unwrap();
    expect_pointer_png(&decoded, "pdu/pointer/color_pointer_24bpp.png");

    let mut encoded = vec![];
    ironrdp_pdu::encode_buf(&parsed, &mut encoded).unwrap();
    assert_eq!(&encoded, data);

    parsed.and_mask = &[];
    parsed.xor_mask = &[];
    expect![[r#"
        ColorPointerAttribute {
            cache_index: 0,
            hot_spot: Point16 {
                x: 3,
                y: 11,
            },
            width: 41,
            height: 39,
            xor_mask: [],
            and_mask: [],
        }
    "#]]
    .assert_debug_eq(&parsed);
}

#[test]
fn color_pointer_1bpp() {
    // Hand-crafted cursor with transparent, black and inverted pixels
    const AND_MASK_1BPP: &[u8] = &[
        0b01111110, 0b00000000, 0b10011110, 0b00000000, 0b10000110, 0b00000000, 0b11000010, 0b00000000, 0b11000110,
        0b00000000, 0b11101010, 0b00000000, 0b11111100, 0b00000000,
    ];

    const XOR_MASK_1BPP: &[u8] = &[
        0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b00100000, 0b00000000, 0b00010000, 0b00000000, 0b00000000,
        0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b00000000,
    ];

    let value = PointerAttribute {
        xor_bpp: 1,
        color_pointer: ColorPointerAttribute {
            cache_index: 0,
            hot_spot: Point16 { x: 0, y: 0 },
            width: 7,
            height: 7,
            xor_mask: XOR_MASK_1BPP,
            and_mask: AND_MASK_1BPP,
        },
    };

    // Re-encode test
    let mut encoded_buffer = vec![];
    ironrdp_pdu::encode_buf(&value, &mut encoded_buffer).unwrap();
    let decoded_value = ironrdp_pdu::decode::<PointerAttribute>(&encoded_buffer).unwrap();
    assert_eq!(&decoded_value, &value);

    let decoded = DecodedPointer::decode_pointer_attribute(&value).unwrap();
    expect_pointer_png(&decoded, "pdu/pointer/color_pointer_1bpp.png");
}

#[test]
fn color_pointer_16bpp() {
    const AND_MASK_16BPP: &[u8] = &[0b10111110, 0b00000000, 0b01111110, 0b00000000];

    const XOR_MASK_16BPP: &[u8] = &[0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF];

    let value = PointerAttribute {
        xor_bpp: 16,
        color_pointer: ColorPointerAttribute {
            cache_index: 0,
            hot_spot: Point16 { x: 0, y: 0 },
            width: 2,
            height: 2,
            xor_mask: XOR_MASK_16BPP,
            and_mask: AND_MASK_16BPP,
        },
    };

    // Re-encode test
    let mut encoded_buffer = vec![];
    ironrdp_pdu::encode_buf(&value, &mut encoded_buffer).unwrap();
    let decoded_value = ironrdp_pdu::decode::<PointerAttribute>(&encoded_buffer).unwrap();
    assert_eq!(&decoded_value, &value);

    let decoded = DecodedPointer::decode_pointer_attribute(&value).unwrap();
    expect_pointer_png(&decoded, "pdu/pointer/color_pointer_16bpp.png");
}

#[test]
fn cached_pointer() {
    let value = CachedPointerAttribute { cache_index: 42 };
    let mut encoded = vec![];
    ironrdp_pdu::encode_buf(&value, &mut encoded).unwrap();
    let decoded = ironrdp_pdu::decode::<CachedPointerAttribute>(&encoded).unwrap();
    assert_eq!(&decoded, &value);
}

#[test]
fn set_pointer_position() {
    let value = PointerPositionAttribute { x: 12, y: 34 };
    let mut encoded = vec![];
    ironrdp_pdu::encode_buf(&value, &mut encoded).unwrap();
    let decoded = ironrdp_pdu::decode::<PointerPositionAttribute>(&encoded).unwrap();
    assert_eq!(&decoded, &value);
}
