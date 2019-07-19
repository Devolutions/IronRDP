#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate bytes;
extern crate ironrdp;

use bytes::BytesMut;
use ironrdp::decode_x224;

fuzz_target!(|data: &[u8]| {
    let mut data_mut = BytesMut::from(data); 
    let _ = decode_x224(&mut data_mut);
});
