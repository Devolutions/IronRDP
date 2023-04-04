#![no_main]

use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
struct Input<'a> {
    src: &'a [u8],
    width: u8,
    height: u8,
}

fuzz_target!(|input: Input<'_>| {
    let mut out = Vec::new();

    let _ = ironrdp_graphics::rle::decompress_24_bpp(input.src, &mut out, input.width, input.height);
    let _ = ironrdp_graphics::rle::decompress_16_bpp(input.src, &mut out, input.width, input.height);
    let _ = ironrdp_graphics::rle::decompress_15_bpp(input.src, &mut out, input.width, input.height);
    let _ = ironrdp_graphics::rle::decompress_8_bpp(input.src, &mut out, input.width, input.height);
});
