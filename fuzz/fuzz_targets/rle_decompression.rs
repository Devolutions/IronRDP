#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: ironrdp_fuzzing::generators::BitmapInput<'_>| {
    ironrdp_fuzzing::oracles::rle_decompress_bitmap(input);
});
