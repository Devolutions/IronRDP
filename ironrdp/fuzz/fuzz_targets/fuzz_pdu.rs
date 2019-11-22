#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::gcc::*;
use ironrdp::*;
use ironrdp::rdp::server_license::*;

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
    let _ = ClientNewLicenseRequest::from_buffer(data);
    let _ = ClientPlatformChallengeResponse::from_buffer(data);
    let _ = InitialServerLicenseMessage::from_buffer(data);
    let _ = ServerLicenseRequest::from_buffer(data);
    let _ = InitialServerLicenseMessage::from_buffer(data);
    let _ = ServerPlatformChallenge::from_buffer(data);
    let _ = CapabilitySet::from_buffer(data);
});
