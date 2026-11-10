use ironrdp_graphics::rdp6::{BitmapDecodeError, BitmapStreamDecoder};
use rstest::rstest;

/// A body too short for the declared bitmap is rejected before the output buffer grows.
#[rstest]
// RLE, alpha plane present, ARGB, followed by a two-byte body.
#[case::rle(&[0x10, 0x00, 0x00])]
// No RLE, no alpha plane, ARGB, followed by a one-byte body and the padding byte.
#[case::raw(&[0x20, 0x00, 0x00])]
fn short_body_is_rejected_before_allocating(#[case] stream: &[u8]) {
    let mut output = Vec::new();

    let result = BitmapStreamDecoder::default().decode_bitmap_stream_to_rgb24(stream, &mut output, 4096, 4096);

    assert!(matches!(result, Err(BitmapDecodeError::InvalidUncompressedDataSize)));
    assert_eq!(output.capacity(), 0);
}

#[rstest]
#[case::plane_size(&[0x30, 0xF2], usize::MAX, 2)]
#[case::plane_size_wrapping_to_zero(&[0x20, 0x00], usize::MAX / 2 + 1, 2)]
#[case::planes_total(&[0x30, 0xF2], usize::MAX / 2, 1)]
#[case::planes_total_with_alpha(&[0x00, 0x00], usize::MAX / 4 + 1, 1)]
fn bitmap_size_overflowing_usize_is_rejected(#[case] stream: &[u8], #[case] width: usize, #[case] height: usize) {
    let mut output = Vec::new();

    let result = BitmapStreamDecoder::default().decode_bitmap_stream_to_rgb24(stream, &mut output, width, height);

    assert!(matches!(result, Err(BitmapDecodeError::InvalidUncompressedDataSize)));
}

// The shortest body each bitmap can have: every scan line of every plane is a single segment
// repeating zero, either 47 times (controlByte 0xF2) or 24 times (controlByte 0x81).
const RLE_ARGB_47X2: &[u8] = &[0x30, 0xF2, 0xF2, 0xF2, 0xF2, 0xF2, 0xF2];
const RLE_ALPHA_ARGB_47X2: &[u8] = &[0x10, 0xF2, 0xF2, 0xF2, 0xF2, 0xF2, 0xF2, 0xF2, 0xF2];
// Color loss level 1 with chroma subsampling: the two chroma planes are 24x2.
const RLE_SUBSAMPLED_AYCOCG_47X3: &[u8] = &[0x39, 0xF2, 0xF2, 0xF2, 0x81, 0x81, 0x81, 0x81];

#[rstest]
#[case::argb(RLE_ARGB_47X2, 47, 2)]
#[case::alpha_argb(RLE_ALPHA_ARGB_47X2, 47, 2)]
#[case::subsampled_aycocg(RLE_SUBSAMPLED_AYCOCG_47X3, 47, 3)]
fn rle_body_of_minimum_size_decodes(#[case] stream: &[u8], #[case] width: usize, #[case] height: usize) {
    let mut output = Vec::new();

    BitmapStreamDecoder::default()
        .decode_bitmap_stream_to_rgb24(stream, &mut output, width, height)
        .expect("a body of one segment per scan line is complete");

    assert_eq!(output, vec![0; width * height * 3]);
}

#[rstest]
#[case::argb(RLE_ARGB_47X2, 47, 2)]
#[case::alpha_argb(RLE_ALPHA_ARGB_47X2, 47, 2)]
#[case::subsampled_aycocg(RLE_SUBSAMPLED_AYCOCG_47X3, 47, 3)]
fn rle_body_one_byte_below_minimum_size_is_rejected(
    #[case] stream: &[u8],
    #[case] width: usize,
    #[case] height: usize,
) {
    let mut output = Vec::new();

    let result = BitmapStreamDecoder::default().decode_bitmap_stream_to_rgb24(
        &stream[..stream.len() - 1],
        &mut output,
        width,
        height,
    );

    assert!(matches!(result, Err(BitmapDecodeError::InvalidUncompressedDataSize)));
}
