#![no_main]

use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
struct Input<'a> {
    src: &'a [u8],
    width: u8,
    height: u8,
}

fuzz_target!(|input: Input<'_>| {
    let mut out = Vec::new();

    let _ = BitmapStreamDecoder::default().decode_bitmap_stream_to_rgb24(
        input.src,
        &mut out,
        input.width as usize,
        input.height as usize,
    );
});
