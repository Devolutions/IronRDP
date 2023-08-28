#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: ironrdp_fuzzing::generators::BitmapInput<'_>| {
    ironrdp_fuzzing::oracles::rdp6_encode_bitmap_stream(&input);
    ironrdp_fuzzing::oracles::rdp6_decode_bitmap_stream_to_rgb24(&input);
});
