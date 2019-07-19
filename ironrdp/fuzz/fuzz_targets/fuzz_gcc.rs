#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::{PduParsing, gcc::ClientGccBlocks};

fuzz_target!(|data: &[u8]| {
    let _ = ClientGccBlocks::from_buffer(data);
});
