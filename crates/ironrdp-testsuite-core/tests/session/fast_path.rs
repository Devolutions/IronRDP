//! Regression tests for Fast-Path update processing.
//!
//! Covers software-rendered and accelerated pointer updates as well as bulk-compressed Fast-Path frames.

use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType, flags as bulk_flags};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::pointer::{
    ColorPointerAttribute, LargePointerAttribute, Point16, PointerAttribute, PointerUpdateData,
};
use ironrdp_pdu::rdp::client_info::CompressionType as PduCompressionType;
use ironrdp_pdu::rdp::headers::CompressionFlags;
use ironrdp_session::fast_path::{Processor, ProcessorBuilder, UpdateKind};
use ironrdp_session::image::DecodedImage;

const IMAGE_DIM: u16 = 64;
const RECT_DIM: u16 = 8;

fn processor_with_pointer(enable_server_pointer: bool, pointer_software_rendering: bool) -> Processor {
    ProcessorBuilder {
        io_channel_id: 0,
        user_channel_id: 0,
        share_id: 0,
        enable_server_pointer,
        pointer_software_rendering,
    }
    .build()
}

fn processor() -> Processor {
    processor_with_pointer(false, false)
}

fn software_pointer_fixture() -> (Processor, DecodedImage) {
    (
        processor_with_pointer(true, true),
        DecodedImage::new(PixelFormat::RgbA32, 4, 4),
    )
}

fn assert_visible_pixels(image: &DecodedImage, expected: &[u8]) {
    let visible: Vec<_> = image
        .data()
        .chunks_exact(4)
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect();
    assert_eq!(visible, expected);
}

const CURSOR_IMAGE: ColorPointerAttribute<'static> = ColorPointerAttribute {
    cache_index: 0,
    hot_spot: Point16 { x: 0, y: 0 },
    width: 1,
    height: 1,
    xor_mask: &[255, 255, 255, 0],
    and_mask: &[0x80, 0],
};

// oracle: mutant (software pointer shape updates leaving use_system_pointer set, omitting or misordering UpdateKind::PointerHidden, or SetPosition omitting UpdateKind::PointerPosition)
#[rstest::rstest]
#[case::color(PointerUpdateData::Color(CURSOR_IMAGE))]
#[case::new(PointerUpdateData::New(PointerAttribute { xor_bpp: 24, color_pointer: CURSOR_IMAGE }))]
#[case::large(PointerUpdateData::Large(LargePointerAttribute {
    xor_bpp: 24,
    cache_index: 0,
    hot_spot: Point16 { x: 0, y: 0 },
    width: 1,
    height: 1,
    xor_mask: &[255, 255, 255, 0],
    and_mask: &[0x80, 0],
}))]
fn pointer_shape_reactivates_software_movement_and_hiding(#[case] shape: PointerUpdateData<'_>) {
    let (mut processor, mut image) = software_pointer_fixture();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::SetHidden)
        .unwrap();
    let shape_updates = processor.process_pointer_update(&mut image, shape).unwrap();
    assert!(
        matches!(
            shape_updates.as_slice(),
            [UpdateKind::PointerHidden, UpdateKind::Region(_)]
        ),
        "software pointer shape activation must hide the system pointer before redrawing the pointer region"
    );
    let updates = processor
        .process_pointer_update(&mut image, PointerUpdateData::SetPosition(Point16 { x: 2, y: 1 }))
        .unwrap();
    assert!(
        updates
            .iter()
            .any(|update| matches!(update, UpdateKind::PointerPosition { x: 2, y: 1 })),
        "software rendering must also report the server's position to input consumers"
    );
    let mut expected = [0; 48];
    expected[18..21].copy_from_slice(&[255, 255, 255]);
    assert_visible_pixels(&image, &expected);
    processor
        .process_pointer_update(&mut image, PointerUpdateData::SetHidden)
        .unwrap();
    assert_visible_pixels(&image, &[0; 48]);
}

// oracle: mutant (accelerated pointer shape updates emitting UpdateKind::PointerHidden or drawing into the software framebuffer)
#[rstest::rstest]
#[case::color(PointerUpdateData::Color(CURSOR_IMAGE))]
#[case::new(PointerUpdateData::New(PointerAttribute { xor_bpp: 24, color_pointer: CURSOR_IMAGE }))]
#[case::large(PointerUpdateData::Large(LargePointerAttribute {
    xor_bpp: 24,
    cache_index: 0,
    hot_spot: Point16 { x: 0, y: 0 },
    width: 1,
    height: 1,
    xor_mask: &[255, 255, 255, 0],
    and_mask: &[0x80, 0],
}))]
fn accelerated_pointer_shape_emits_only_pointer_bitmap(#[case] shape: PointerUpdateData<'_>) {
    let mut processor = processor_with_pointer(true, false);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 4, 4);
    let updates = processor.process_pointer_update(&mut image, shape).unwrap();
    assert!(
        matches!(updates.as_slice(), [UpdateKind::PointerBitmap(_)]),
        "accelerated pointer shapes must emit only PointerBitmap without PointerHidden or software regions"
    );
    assert_visible_pixels(&image, &[0; 48]);
}

// oracle: mutant (SetPosition while the pointer is hidden omitting UpdateKind::PointerPosition or failing to retain coordinates for the next shape update)
#[test]
fn hidden_pointer_position_is_retained_when_shape_returns() {
    let (mut processor, mut image) = software_pointer_fixture();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::Color(CURSOR_IMAGE))
        .unwrap();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::SetHidden)
        .unwrap();
    let updates = processor
        .process_pointer_update(&mut image, PointerUpdateData::SetPosition(Point16 { x: 2, y: 1 }))
        .unwrap();
    assert!(
        updates
            .iter()
            .any(|update| matches!(update, UpdateKind::PointerPosition { x: 2, y: 1 })),
        "hidden pointer movement must still report the server's position to input consumers"
    );
    assert_visible_pixels(&image, &[0; 48]);
    processor
        .process_pointer_update(&mut image, PointerUpdateData::Color(CURSOR_IMAGE))
        .unwrap();
    let mut expected = [0; 48];
    expected[18..21].copy_from_slice(&[255, 255, 255]);
    assert_visible_pixels(&image, &expected);
}

// oracle: mutant (update_pointer restoring stale PointerLayer::Background pixels over desktop updates received while the pointer was hidden)
#[test]
fn restored_pointer_preserves_background_changes_while_hidden() {
    let (mut processor, mut image) = software_pointer_fixture();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::Color(CURSOR_IMAGE))
        .unwrap();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::SetHidden)
        .unwrap();
    let bitmap = encode_vec(&BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 3,
            },
            width: 4,
            height: 4,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &[100, 80, 30, 255].repeat(16),
        }],
    })
    .unwrap();
    let frame = fastpath_frame(
        &encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::Bitmap,
            compression_flags: None,
            compression_type: None,
            data: &bitmap,
        })
        .unwrap(),
    );
    processor
        .process(&mut image, &frame, &mut WriteBuf::new(), &mut None)
        .unwrap();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::SetPosition(Point16 { x: 2, y: 1 }))
        .unwrap();
    processor
        .process_pointer_update(&mut image, PointerUpdateData::Color(CURSOR_IMAGE))
        .unwrap();
    let mut expected = [30, 80, 100].repeat(16);
    expected[18..21].copy_from_slice(&[225, 175, 155]);
    assert_visible_pixels(&image, &expected);
}

/// Encodes the inner FastPath Bitmap update: a single uncompressed 8x8 32bpp
/// rectangle filled with a repetitive (and therefore compressible) pattern.
fn bitmap_update_payload() -> Vec<u8> {
    let pixels: Vec<u8> = (0..RECT_DIM * RECT_DIM)
        .flat_map(|_| [0x11u8, 0x22, 0x33, 0xff])
        .collect();

    let update = BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: RECT_DIM - 1,
                bottom: RECT_DIM - 1,
            },
            width: RECT_DIM,
            height: RECT_DIM,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &pixels,
        }],
    };

    encode_vec(&update).expect("encode bitmap update")
}

/// Wraps an encoded `FastPathUpdatePdu` in a FastPath output frame (the
/// `FastPathHeader` that `Processor::process` expects to read first).
fn fastpath_frame(update_pdu: &[u8]) -> Vec<u8> {
    let header = FastPathHeader::new(EncryptionFlags::empty(), update_pdu.len());
    let mut frame = encode_vec(&header).expect("encode FastPath header");
    frame.extend_from_slice(update_pdu);
    frame
}

/// Runs one FastPath frame through a fresh processor and returns the resulting
/// framebuffer.
fn render(frame: &[u8]) -> DecodedImage {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, IMAGE_DIM, IMAGE_DIM);
    let mut output = WriteBuf::new();
    let mut bulk_decompressor = Some(BulkCompressor::new(BulkCompressionType::Rdp5));
    processor()
        .process(&mut image, frame, &mut output, &mut bulk_decompressor)
        .expect("process FastPath frame");
    image
}

#[test]
fn compressed_fastpath_update_decompresses_like_its_uncompressed_twin() {
    let payload = bitmap_update_payload();

    let uncompressed = fastpath_frame(
        &encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::Bitmap,
            compression_flags: None,
            compression_type: None,
            data: &payload,
        })
        .expect("encode uncompressed FastPath update"),
    );

    // Bulk-compress the same payload (RDP5 / MPPC) and wrap it as a compressed
    // FastPath update.
    let mut compressor = BulkCompressor::new(BulkCompressionType::Rdp5);
    let (size, flags) = compressor.compress(&payload).expect("bulk compress payload");
    assert_ne!(
        flags & bulk_flags::PACKET_COMPRESSED,
        0,
        "test payload must actually compress; adjust it if MPPC declines to compress"
    );
    let compressed_data = compressor.compressed_data(size).to_vec();

    let compressed_pdu = encode_vec(&FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::Bitmap,
        // Carry the compressor's control bits (notably PACKET_AT_FRONT) so the
        // decompressor resets its history at the start of the stream.
        compression_flags: Some(CompressionFlags::from_bits_retain(
            u8::try_from(flags & 0xe0).expect("control-flag byte fits in u8"),
        )),
        compression_type: Some(PduCompressionType::K64),
        data: &compressed_data,
    })
    .expect("encode compressed FastPath update");
    let compressed = fastpath_frame(&compressed_pdu);

    let from_uncompressed = render(&uncompressed);
    let from_compressed = render(&compressed);

    assert_eq!(
        from_uncompressed.data(),
        from_compressed.data(),
        "compressed FastPath update rendered differently from its uncompressed twin"
    );

    // Sanity check: the update actually drew something, so the equality above
    // compares real pixels rather than two blank frames.
    let blank = DecodedImage::new(PixelFormat::RgbA32, IMAGE_DIM, IMAGE_DIM);
    assert_ne!(
        from_uncompressed.data(),
        blank.data(),
        "the bitmap update should have modified the framebuffer"
    );
}

#[test]
fn fragmented_compressed_fastpath_update_decompresses_before_reassembly() {
    let payload = bitmap_update_payload();
    let split = payload.len() / 2;
    let mut sender = BulkCompressor::new(BulkCompressionType::Rdp5);

    let (first_size, first_flags) = sender.compress(&payload[..split]).expect("compress first fragment");
    assert_ne!(
        first_flags & bulk_flags::PACKET_COMPRESSED,
        0,
        "first fragment must compress"
    );
    let first_data = sender.compressed_data(first_size).to_vec();
    let first = fastpath_frame(
        &encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::First,
            update_code: UpdateCode::Bitmap,
            compression_flags: Some(CompressionFlags::from_bits_retain(
                u8::try_from(first_flags & 0xe0).expect("control-flag byte fits in u8"),
            )),
            compression_type: Some(PduCompressionType::K64),
            data: &first_data,
        })
        .expect("encode first FastPath fragment"),
    );

    let (last_size, last_flags) = sender.compress(&payload[split..]).expect("compress last fragment");
    assert_ne!(
        last_flags & bulk_flags::PACKET_COMPRESSED,
        0,
        "last fragment must compress"
    );
    let last_data = sender.compressed_data(last_size).to_vec();
    let last = fastpath_frame(
        &encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Last,
            update_code: UpdateCode::Bitmap,
            compression_flags: Some(CompressionFlags::from_bits_retain(
                u8::try_from(last_flags & 0xe0).expect("control-flag byte fits in u8"),
            )),
            compression_type: Some(PduCompressionType::K64),
            data: &last_data,
        })
        .expect("encode last FastPath fragment"),
    );

    let uncompressed = fastpath_frame(
        &encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::Bitmap,
            compression_flags: None,
            compression_type: None,
            data: &payload,
        })
        .expect("encode uncompressed FastPath update"),
    );
    let expected = render(&uncompressed);

    let mut image = DecodedImage::new(PixelFormat::RgbA32, IMAGE_DIM, IMAGE_DIM);
    let mut output = WriteBuf::new();
    let mut bulk_decompressor = Some(BulkCompressor::new(BulkCompressionType::Rdp5));
    let mut processor = processor();

    assert!(
        processor
            .process(&mut image, &first, &mut output, &mut bulk_decompressor)
            .expect("process first FastPath fragment")
            .is_empty()
    );
    assert!(
        !processor
            .process(&mut image, &last, &mut output, &mut bulk_decompressor)
            .expect("process last FastPath fragment")
            .is_empty()
    );
    assert_eq!(image.data(), expected.data());
}
