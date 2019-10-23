#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::gcc::{
    ClientClusterData, ClientGccBlocks, ConferenceCreateRequest, ConferenceCreateResponse,
    ServerGccBlocks,
};
use ironrdp::*;

fuzz_target!(|data: &[u8]| {
    let _ = Request::from_buffer(data);
    let _ = Response::from_buffer(data);
    let _ = McsPdu::from_buffer(data);
    let _ = ClientGccBlocks::from_buffer(data);
    let _ = ServerGccBlocks::from_buffer(data);
    let _ = ClientClusterData::from_buffer(data);
    let _ = ConferenceCreateRequest::from_buffer(data);
    let _ = ConferenceCreateResponse::from_buffer(data);
    let _ = ConnectInitial::from_buffer(data);
    let _ = ConnectResponse::from_buffer(data);
    let _ = ClientInfoPdu::from_buffer(data);
    let _ = ServerLicensePdu::from_buffer(data);
    let _ = CapabilitySet::from_buffer(data);
});
