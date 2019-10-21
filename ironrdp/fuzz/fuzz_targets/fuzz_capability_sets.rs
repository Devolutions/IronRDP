#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::{CapabilitySet, PduParsing};

fuzz_target!(|data: &[u8]| {
    let _ = CapabilitySet::from_buffer(data);
});
