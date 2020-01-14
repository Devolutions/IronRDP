#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate ironrdp;

use ironrdp::*;
use ironrdp::rdp::*;

fuzz_target!(|data: &[u8]| {
    let _ = Request::from_buffer(data);
    let _ = Response::from_buffer(data);
    let _ = McsPdu::from_buffer(data);
    let _ = ConnectInitial::from_buffer(data);
    let _ = ConnectResponse::from_buffer(data);
    let _ = ClientInfoPdu::from_buffer(data);
    let _ = CapabilitySet::from_buffer(data);
    let _ = ShareControlHeader::from_buffer(data);

    let _ = gcc::ClientGccBlocks::from_buffer(data);
    let _ = gcc::ServerGccBlocks::from_buffer(data);
    let _ = gcc::ClientClusterData::from_buffer(data);
    let _ = gcc::ConferenceCreateRequest::from_buffer(data);
    let _ = gcc::ConferenceCreateResponse::from_buffer(data);

    let _ = server_license::ClientNewLicenseRequest::from_buffer(data);
    let _ = server_license::ClientPlatformChallengeResponse::from_buffer(data);
    let _ = server_license::InitialServerLicenseMessage::from_buffer(data);
    let _ = server_license::ServerLicenseRequest::from_buffer(data);
    let _ = server_license::InitialServerLicenseMessage::from_buffer(data);
    let _ = server_license::ServerPlatformChallenge::from_buffer(data);

    let _ = vc::ChannelPduHeader::from_buffer(data);
});
