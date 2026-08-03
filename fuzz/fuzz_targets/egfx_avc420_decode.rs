#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ironrdp_fuzzing::oracles::egfx_avc420_decode(data);
});
