#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::{parse_negotiation_request, parse_negotiation_response, X224TPDUType};

fuzz_target!(|data: &[u8]| {
    // let _ = parse_negotiation_request(X224TPDUType::ConnectionRequest, data);
    let _ = parse_negotiation_response(X224TPDUType::ConnectionConfirm, data);
});
