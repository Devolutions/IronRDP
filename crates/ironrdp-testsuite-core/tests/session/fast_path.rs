//! Regression tests for bulk-compressed FastPath updates.
//!
//! A FastPath `Processor` always owns a bulk decompressor: the library builds
//! one in `ProcessorBuilder::build`. A server can send a compressed FastPath
//! update (for example a full-frame redraw after a resize) regardless of what
//! the client advertised, so the decompressor must always be present. These
//! tests pin that a compressed update decodes to exactly the same pixels as its
//! uncompressed twin rather than being dropped or aborting the session.

use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType, flags as bulk_flags};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp_pdu::fast_path::{
    Compression as FpCompression, EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode,
};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::rdp::client_info::CompressionType as PduCompressionType;
use ironrdp_pdu::rdp::headers::CompressionFlags;
use ironrdp_session::fast_path::{Processor, ProcessorBuilder};
use ironrdp_session::image::DecodedImage;

const IMAGE_DIM: u16 = 64;
const RECT_DIM: u16 = 8;

fn processor() -> Processor {
    ProcessorBuilder {
        io_channel_id: 0,
        user_channel_id: 0,
        share_id: 0,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build()
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
    processor()
        .process(&mut image, frame, &mut output)
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

    let mut compressed_pdu = encode_vec(&FastPathUpdatePdu {
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
    // `FastPathUpdatePdu::encode` writes the update header byte before it sets the
    // COMPRESSION_USED bit, so the encoded header does not flag the trailing
    // compression byte. Set it here so the PDU decodes as compressed (idempotent
    // if the encoder is corrected).
    compressed_pdu[0] |= FpCompression::COMPRESSION_USED.bits() << 6;
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
