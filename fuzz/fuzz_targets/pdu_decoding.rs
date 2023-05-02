#![no_main]

use ironrdp_pdu::mcs::*;
use ironrdp_pdu::nego::*;
use ironrdp_pdu::rdp::*;
use ironrdp_pdu::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode::<ConnectionRequest>(data);
    let _ = decode::<ConnectionConfirm>(data);
    let _ = decode::<McsMessage>(data);
    let _ = ConnectInitial::from_buffer(data);
    let _ = ConnectResponse::from_buffer(data);
    let _ = ClientInfoPdu::from_buffer(data);
    let _ = capability_sets::CapabilitySet::from_buffer(data);
    let _ = headers::ShareControlHeader::from_buffer(data);
    let _ = PreconnectionPdu::from_buffer(data);
    let _ = server_error_info::ServerSetErrorInfoPdu::from_buffer(data);

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

    let _ = fast_path::FastPathHeader::from_buffer(data);
    let _ = fast_path::FastPathUpdatePdu::from_buffer(data);
    let _ = fast_path::FastPathUpdate::from_buffer_with_code(data, fast_path::UpdateCode::SurfaceCommands);

    let _ = surface_commands::SurfaceCommand::from_buffer(data);
    let _ = surface_commands::SurfaceBitsPdu::from_buffer(data);
    let _ = surface_commands::FrameMarkerPdu::from_buffer(data);
    let _ = surface_commands::ExtendedBitmapDataPdu::from_buffer(data);
    let _ = surface_commands::BitmapDataHeader::from_buffer(data);

    let _ = codecs::rfx::Headers::from_buffer(data);
    let _ = codecs::rfx::FrameAcknowledgePdu::from_buffer(data);
    let _ = codecs::rfx::ContextPdu::from_buffer(data);
    let _ = codecs::rfx::FrameBeginPdu::from_buffer(data);
    let _ = codecs::rfx::FrameEndPdu::from_buffer(data);
    let _ = codecs::rfx::RegionPdu::from_buffer(data);
    let _ = codecs::rfx::TileSetPdu::from_buffer(data);
    let _ = codecs::rfx::RfxRectangle::from_buffer(data);
    let _ = codecs::rfx::Quant::from_buffer(data);
    let _ = codecs::rfx::Tile::from_buffer(data);
    let _ = codecs::rfx::SyncPdu::from_buffer(data);
    let _ = codecs::rfx::CodecVersionsPdu::from_buffer(data);
    let _ = codecs::rfx::ChannelsPdu::from_buffer(data);
    let _ = codecs::rfx::Channel::from_buffer(data);

    let _ = input::InputEventPdu::from_buffer(data);
    let _ = input::InputEvent::from_buffer(data);

    let _ = bitmap::rdp6::BitmapStream::from_buffer(data);
});
