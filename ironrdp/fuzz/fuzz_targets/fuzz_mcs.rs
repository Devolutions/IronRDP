#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::PduParsing;
use ironrdp::McsPdu;

fuzz_target!(|data: &[u8]| {
    let _ = McsPdu::from_buffer(data);
});
