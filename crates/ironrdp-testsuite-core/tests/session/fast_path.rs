//! Regression tests for Fast-Path update processing.
//!
//! Covers software-rendered and accelerated pointer updates as well as bulk-compressed Fast-Path frames.

use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType, flags as bulk_flags};
use ironrdp_core::{ReadCursor, WriteBuf, decode_cursor, encode_vec};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::geometry::{ExclusiveRectangle, InclusiveRectangle};
use ironrdp_pdu::pointer::{
    ColorPointerAttribute, LargePointerAttribute, Point16, PointerAttribute, PointerUpdateData,
};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits, LargePointerSupportFlags};
use ironrdp_pdu::rdp::client_info::CompressionType as PduCompressionType;
use ironrdp_pdu::rdp::headers::CompressionFlags;
use ironrdp_pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu, SurfaceCommand};
use ironrdp_server::{
    BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat as ServerPixelFormat, RemoteFxCodec, UpdateEncoder,
    UpdateEncoderCodecs,
};
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
    render_all(&[frame.to_vec()])
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

/// One pixel of the bitmaps the surface bits tests draw, in the BGRX order of an uncompressed 32bpp surface bitmap.
const SURFACE_PIXEL: [u8; 4] = [0x30, 0x20, 0x10, 0xff];

/// Which of the two surface bits commands a test sends.
#[derive(Clone, Copy, Debug)]
enum SurfaceBitsKind {
    Set,
    Stream,
}

/// Encodes a FastPath frame with one surface bits command of `kind` at `destination`, carrying an uncompressed 32bpp
/// `width` by `height` bitmap of [`SURFACE_PIXEL`].
fn surface_bits_frame(kind: SurfaceBitsKind, destination: ExclusiveRectangle, width: u16, height: u16) -> Vec<u8> {
    uncompressed_surface_bits_frame(kind, 32, &SURFACE_PIXEL, destination, width, height)
}

/// Encodes a FastPath frame with one surface bits command of `kind` at `destination`, carrying an uncompressed `width`
/// by `height` bitmap of `bpp` bits per pixel whose every pixel is `pixel`.
fn uncompressed_surface_bits_frame(
    kind: SurfaceBitsKind,
    bpp: u8,
    pixel: &[u8],
    destination: ExclusiveRectangle,
    width: u16,
    height: u16,
) -> Vec<u8> {
    let pixels = pixel.repeat(usize::from(width) * usize::from(height));
    let bits = SurfaceBitsPdu {
        destination,
        extended_bitmap_data: ExtendedBitmapDataPdu {
            bpp,
            codec_id: 0, // CODEC_ID_NONE
            width,
            height,
            header: None,
            data: &pixels,
        },
    };
    let command = match kind {
        SurfaceBitsKind::Set => SurfaceCommand::SetSurfaceBits(bits),
        SurfaceBitsKind::Stream => SurfaceCommand::StreamSurfaceBits(bits),
    };
    surface_commands_frame(&encode_vec(&command).expect("encode surface command"))
}

/// Wraps encoded surface commands in a FastPath Surface Commands update frame.
fn surface_commands_frame(commands: &[u8]) -> Vec<u8> {
    fastpath_frame(
        &encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::SurfaceCommands,
            compression_flags: None,
            compression_type: None,
            data: commands,
        })
        .expect("encode FastPath surface commands update"),
    )
}

/// The red, green and blue of the pixel at `x`, `y`.
fn rgb_at(image: &DecodedImage, x: usize, y: usize) -> [u8; 3] {
    let offset = (y * usize::from(IMAGE_DIM) + x) * 4;
    let pixel = &image.data()[offset..offset + 4];
    [pixel[0], pixel[1], pixel[2]]
}

/// Asserts that a `width` by `height` bitmap of [`SURFACE_PIXEL`] was drawn with its top left at `left`, `top`, and
/// nothing anywhere else.
fn assert_drawn_at(image: &DecodedImage, left: usize, top: usize, width: usize, height: usize) {
    let drawn = [SURFACE_PIXEL[2], SURFACE_PIXEL[1], SURFACE_PIXEL[0]];
    assert_drawn_within(image, (left, top, width, height), drawn, drawn);
}

/// Asserts that every pixel of the `(left, top, width, height)` region has each of its red, green and blue between
/// `min` and `max`, and that every pixel outside it is black.
fn assert_drawn_within(
    image: &DecodedImage,
    (left, top, width, height): (usize, usize, usize, usize),
    min: [u8; 3],
    max: [u8; 3],
) {
    for y in 0..usize::from(IMAGE_DIM) {
        for x in 0..usize::from(IMAGE_DIM) {
            let rgb = rgb_at(image, x, y);
            if (left..left + width).contains(&x) && (top..top + height).contains(&y) {
                let within = (0..3).all(|c| (min[c]..=max[c]).contains(&rgb[c]));
                assert!(within, "pixel {x},{y} is {rgb:?}, not between {min:?} and {max:?}");
            } else {
                assert_eq!(rgb, [0, 0, 0], "pixel {x},{y} outside the bitmap");
            }
        }
    }
}

/// A server can send a destBottom of zero, which once took the client's surface processing down with an arithmetic
/// overflow. destRight and destBottom SHOULD be ignored, so the bitmap is drawn at the size it gives.
// oracle: spec MS-RDPBCGR §2.2.9.2.1
#[test]
fn surface_bits_with_a_zero_destination_bottom_are_drawn_at_the_bitmap_size() {
    let frame = surface_bits_frame(
        SurfaceBitsKind::Set,
        ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 2,
            bottom: 0,
        },
        2,
        2,
    );

    assert_drawn_at(&render(&frame), 0, 0, 2, 2);
}

/// The Extended Bitmap Data gives the size of the decoded image for both surface bits commands, so a destination that
/// disagrees with the bitmap doesn't change where it's drawn.
#[rstest::rstest]
// oracle: spec MS-RDPBCGR §2.2.9.2.1
#[case::set(SurfaceBitsKind::Set)]
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
#[case::stream(SurfaceBitsKind::Stream)]
fn surface_bits_are_drawn_at_the_bitmap_size_rather_than_the_destination_bounds(#[case] kind: SurfaceBitsKind) {
    let frame = surface_bits_frame(
        kind,
        ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 4,
            bottom: 4,
        },
        2,
        2,
    );

    assert_drawn_at(&render(&frame), 0, 0, 2, 2);
}

/// A bitmap is drawn with its top left at destLeft and destTop, so one sent away from the origin is drawn there and
/// nowhere else.
// oracle: spec MS-RDPBCGR §2.2.9.2.1
#[rstest::rstest]
#[case::right_of_the_origin(5, 0)]
#[case::below_the_origin(0, 9)]
#[case::away_from_both_edges(37, 21)]
fn surface_bits_are_drawn_at_their_destination_origin(#[case] left: u16, #[case] top: u16) {
    let frame = surface_bits_frame(
        SurfaceBitsKind::Set,
        ExclusiveRectangle {
            left,
            top,
            right: left + 3,
            bottom: top + 2,
        },
        3,
        2,
    );

    assert_drawn_at(&render(&frame), usize::from(left), usize::from(top), 3, 2);
}

/// Surface bits that cover no pixels, or that would run past the 16-bit coordinate range, draw nothing and leave the
/// session running.
#[rstest::rstest]
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
#[case::zero_width(ExclusiveRectangle { left: 0, top: 0, right: 0, bottom: 2 }, 0, 2)]
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
#[case::zero_height(ExclusiveRectangle { left: 0, top: 0, right: 2, bottom: 0 }, 2, 0)]
// oracle: mutant placing a bitmap that runs past 65535 with unchecked addition
#[case::past_the_coordinate_range(ExclusiveRectangle { left: u16::MAX, top: 0, right: 1, bottom: 1 }, 2, 1)]
fn surface_bits_that_cover_no_pixels_draw_nothing(
    #[case] destination: ExclusiveRectangle,
    #[case] width: u16,
    #[case] height: u16,
) {
    let image = render(&surface_bits_frame(SurfaceBitsKind::Set, destination, width, height));

    let blank = blank_image();
    assert_eq!(image.data(), blank.data());
}

/// An uncompressed bitmap is read with its own width as the stride at every color depth, so one sent with a larger
/// destination is drawn exactly as one sent with a destination of its own size, in the colors its pixels hold.
/// The 15 and 16 bpp pixels are one channel at full intensity, so a channel read from the wrong bits shows. How a 5 or
/// 6 bit channel widens to 8 bits isn't specified, so a full one only has to reach 248, which is 31 shifted left by 3.
/// 8bpp is left out because it needs a palette first, and it takes its stride from the same width.
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
// oracle: spec MS-RDPBCGR §2.2.1.3.2
#[rstest::rstest]
#[case::bpp_15_full_red(15, &[0x00, 0x7c], [248, 0, 0], [255, 0, 0])]
#[case::bpp_16_full_green(16, &[0xe0, 0x07], [0, 248, 0], [0, 255, 0])]
#[case::bpp_24(24, &[0x30, 0x20, 0x10], [0x10, 0x20, 0x30], [0x10, 0x20, 0x30])]
#[case::bpp_32(32, &SURFACE_PIXEL, [0x10, 0x20, 0x30], [0x10, 0x20, 0x30])]
fn uncompressed_surface_bits_are_read_with_the_bitmap_stride_at_every_depth(
    #[case] bpp: u8,
    #[case] pixel: &[u8],
    #[case] min: [u8; 3],
    #[case] max: [u8; 3],
) {
    let frame = |right, bottom| {
        uncompressed_surface_bits_frame(
            SurfaceBitsKind::Set,
            bpp,
            pixel,
            ExclusiveRectangle {
                left: 0,
                top: 0,
                right,
                bottom,
            },
            4,
            2,
        )
    };
    let expected = render(&frame(4, 2));
    assert_drawn_within(&expected, (0, 0, 4, 2), min, max);

    assert_eq!(render(&frame(8, 8)).data(), expected.data());
}

/// A change made to every surface bits command IronRDP's server sends, before the client processes it.
#[derive(Clone, Copy, Debug)]
enum SurfaceBitsEdit {
    None,
    ZeroDestinationRightAndBottom,
    ZeroBitmapSize,
}

impl SurfaceBitsEdit {
    fn apply(self, bits: &mut SurfaceBitsPdu<'_>) {
        match self {
            Self::None => {}
            Self::ZeroDestinationRightAndBottom => {
                bits.destination.right = 0;
                bits.destination.bottom = 0;
            }
            Self::ZeroBitmapSize => {
                bits.extended_bitmap_data.width = 0;
                bits.extended_bitmap_data.height = 0;
            }
        }
    }
}

/// An encoder that sends whole-desktop bitmaps as surface bits with `codecs`, as IronRDP's server does.
fn surface_bits_encoder(codecs: UpdateEncoderCodecs) -> UpdateEncoder {
    UpdateEncoder::new(
        DesktopSize {
            width: IMAGE_DIM,
            height: IMAGE_DIM,
        },
        CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS,
        codecs,
        16 * 1024 * 1024,
        0,
        0,
        LargePointerSupportFlags::empty(),
    )
    .expect("build the update encoder")
}

/// The FastPath frames `encoder` sends for one whole-desktop bitmap of `pixel` (BGRX), with `edit` applied to each
/// surface bits command in them.
async fn encoded_surface_frames(encoder: &mut UpdateEncoder, pixel: [u8; 4], edit: SurfaceBitsEdit) -> Vec<Vec<u8>> {
    let stride = usize::from(IMAGE_DIM) * 4;
    let bitmap = BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(IMAGE_DIM).expect("non-zero width"),
        height: NonZeroU16::new(IMAGE_DIM).expect("non-zero height"),
        format: ServerPixelFormat::BgrX32,
        data: pixel.repeat(usize::from(IMAGE_DIM) * usize::from(IMAGE_DIM)).into(),
        stride: NonZeroUsize::new(stride).expect("non-zero stride"),
    };
    let mut updates = encoder.update(DisplayUpdate::Bitmap(bitmap));
    let mut frames = Vec::new();
    while let Some(fragmenter) = updates.next().await {
        let fragmenter = fragmenter.expect("encode the bitmap");
        let mut cursor = ReadCursor::new(&fragmenter.data);
        let mut commands = Vec::new();
        while !cursor.is_empty() {
            let mut command = decode_cursor::<SurfaceCommand<'_>>(&mut cursor).expect("decode a surface command");
            if let SurfaceCommand::SetSurfaceBits(bits) | SurfaceCommand::StreamSurfaceBits(bits) = &mut command {
                edit.apply(bits);
            }
            commands.extend(encode_vec(&command).expect("encode a surface command"));
        }
        frames.push(surface_commands_frame(&commands));
    }
    frames
}

/// The framebuffer a test draws into, and the baseline for one that should stay blank.
fn blank_image() -> DecodedImage {
    DecodedImage::new(PixelFormat::RgbA32, IMAGE_DIM, IMAGE_DIM)
}

/// Runs `frames` through one processor, in order, and returns the resulting framebuffer.
fn render_all(frames: &[Vec<u8>]) -> DecodedImage {
    let mut image = blank_image();
    let mut output = WriteBuf::new();
    let mut bulk_decompressor = Some(BulkCompressor::new(BulkCompressionType::Rdp5));
    let mut processor = processor();
    for frame in frames {
        processor
            .process(&mut image, frame, &mut output, &mut bulk_decompressor)
            .expect("process FastPath frame");
    }
    image
}

fn remotefx_encoder() -> UpdateEncoder {
    let mut codecs = UpdateEncoderCodecs::new();
    codecs.set_remotefx_codec(Some((EntropyBits::Rlgr3, 3, RemoteFxCodec::RemoteFx)));
    surface_bits_encoder(codecs)
}

/// The RemoteFX decoder places its tiles from destLeft and destTop and clips them to its channel's size (MS-RDPRFX
/// 2.2.2.1.3), so a RemoteFX frame is drawn the same whatever its destRight, destBottom, width and height say.
#[rstest::rstest]
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
// oracle: spec MS-RDPRFX §2.2.2.1.3
#[case::zero_destination_right_and_bottom(SurfaceBitsEdit::ZeroDestinationRightAndBottom)]
// oracle: spec MS-RDPRFX §2.2.2.1.3
#[case::zero_bitmap_size(SurfaceBitsEdit::ZeroBitmapSize)]
#[tokio::test]
async fn a_remotefx_frame_is_drawn_whatever_its_destination_bounds_and_bitmap_size(#[case] edit: SurfaceBitsEdit) {
    let pixel = [0x40, 0x80, 0xc0, 0xff];
    let expected = render_all(&encoded_surface_frames(&mut remotefx_encoder(), pixel, SurfaceBitsEdit::None).await);
    let blank = blank_image();
    assert_ne!(
        expected.data(),
        blank.data(),
        "the RemoteFX frame should draw something"
    );

    let edited = render_all(&encoded_surface_frames(&mut remotefx_encoder(), pixel, edit).await);

    assert_eq!(edited.data(), expected.data());
}

fn qoi_encoder() -> UpdateEncoder {
    let mut codecs = UpdateEncoderCodecs::new();
    codecs.set_qoi(Some(0x0A));
    surface_bits_encoder(codecs)
}

fn qoiz_encoder() -> UpdateEncoder {
    let mut codecs = UpdateEncoderCodecs::new();
    codecs.set_qoiz(Some(0x0B));
    surface_bits_encoder(codecs)
}

/// A QOI frame is placed by its bitmap's size, so it's drawn the same whatever its destRight and destBottom say.
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
#[tokio::test]
async fn a_qoi_frame_is_drawn_whatever_its_destination_bounds() {
    let pixel = [0x40, 0x80, 0xc0, 0xff];
    let unedited = render_all(&encoded_surface_frames(&mut qoi_encoder(), pixel, SurfaceBitsEdit::None).await);
    assert_ne!(
        unedited.data(),
        blank_image().data(),
        "the QOI frame should draw something"
    );

    let edited = render_all(
        &encoded_surface_frames(
            &mut qoi_encoder(),
            pixel,
            SurfaceBitsEdit::ZeroDestinationRightAndBottom,
        )
        .await,
    );

    assert_eq!(edited.data(), unedited.data());
}

/// A QOI image carries its own size, which has to match the bitmap's, so a QOI frame whose bitmap claims no pixels
/// draws nothing, and the session continues.
// oracle: spec MS-RDPBCGR §2.2.9.2.1.1
#[tokio::test]
async fn a_qoi_frame_whose_bitmap_claims_no_pixels_draws_nothing() {
    let pixel = [0x40, 0x80, 0xc0, 0xff];

    let edited = render_all(&encoded_surface_frames(&mut qoi_encoder(), pixel, SurfaceBitsEdit::ZeroBitmapSize).await);

    assert_eq!(edited.data(), blank_image().data());
}

/// QOIZ frames share one zstd stream, so a frame that draws nothing because its bitmap claims no pixels is still
/// decompressed, and the frame after it decodes as if nothing had been claimed.
// oracle: mutant skipping the decompression of a QOIZ frame whose bitmap has no pixels
#[tokio::test]
async fn a_qoiz_frame_with_no_pixels_keeps_the_stream_for_the_next_one() {
    let (first, second) = ([0x10, 0x20, 0x30, 0xff], [0x60, 0x50, 0x40, 0xff]);

    let mut encoder = qoiz_encoder();
    let mut frames = encoded_surface_frames(&mut encoder, first, SurfaceBitsEdit::None).await;
    frames.extend(encoded_surface_frames(&mut encoder, second, SurfaceBitsEdit::None).await);
    let expected = render_all(&frames);

    let mut encoder = qoiz_encoder();
    let mut frames = encoded_surface_frames(&mut encoder, first, SurfaceBitsEdit::ZeroBitmapSize).await;
    frames.extend(encoded_surface_frames(&mut encoder, second, SurfaceBitsEdit::None).await);
    let edited = render_all(&frames);

    let blank = blank_image();
    assert_ne!(
        expected.data(),
        blank.data(),
        "the second QOIZ frame should draw something"
    );
    assert_eq!(edited.data(), expected.data());
}
