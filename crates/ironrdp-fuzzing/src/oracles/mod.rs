//! Oracles.
//!
//! Oracles take a test case and determine whether we have a bug. For example,
//! one of the simplest oracles is to take a RDP PDU as our input test case,
//! encode and decode it, and (implicitly) check that no assertions
//! failed or segfaults happened. A more complicated oracle might compare the
//! result of two different implementations for the same thing, and
//! make sure that the two executions are observably identical (differential fuzzing).
//!
//! When an oracle finds a bug, it should report it to the fuzzing engine by
//! panicking.

use crate::generators::BitmapInput;

pub fn pdu_decode(data: &[u8]) {
    use ironrdp_pdu::mcs::*;
    use ironrdp_pdu::nego::*;
    use ironrdp_pdu::rdp::*;
    use ironrdp_pdu::*;

    let _ = decode::<ConnectionRequest>(data);
    let _ = decode::<ConnectionConfirm>(data);
    let _ = decode::<McsMessage>(data);
    let _ = ConnectInitial::from_buffer(data);
    let _ = ConnectResponse::from_buffer(data);
    let _ = ClientInfoPdu::from_buffer(data);
    let _ = capability_sets::CapabilitySet::from_buffer(data);
    let _ = headers::ShareControlHeader::from_buffer(data);
    let _ = decode::<pcb::PreconnectionBlob>(data);
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

    let _ = decode::<fast_path::FastPathHeader>(data);
    let _ = decode::<fast_path::FastPathUpdatePdu>(data);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::SurfaceCommands);

    let _ = decode::<surface_commands::SurfaceCommand>(data);
    let _ = decode::<surface_commands::SurfaceBitsPdu>(data);
    let _ = decode::<surface_commands::FrameMarkerPdu>(data);
    let _ = decode::<surface_commands::ExtendedBitmapDataPdu>(data);
    let _ = decode::<surface_commands::BitmapDataHeader>(data);

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

    let _ = decode::<bitmap::rdp6::BitmapStream>(data);

    let _ = decode::<ironrdp_cliprdr::pdu::ClipboardPdu>(data);
}

pub fn rle_decompress_bitmap(input: BitmapInput) {
    let mut out = Vec::new();

    let _ = ironrdp_graphics::rle::decompress_24_bpp(input.src, &mut out, input.width, input.height);
    let _ = ironrdp_graphics::rle::decompress_16_bpp(input.src, &mut out, input.width, input.height);
    let _ = ironrdp_graphics::rle::decompress_15_bpp(input.src, &mut out, input.width, input.height);
    let _ = ironrdp_graphics::rle::decompress_8_bpp(input.src, &mut out, input.width, input.height);
}

pub fn rdp6_decode_bitmap_stream_to_rgb24(input: BitmapInput) {
    use ironrdp_graphics::rdp6::BitmapStreamDecoder;

    let mut out = Vec::new();

    let _ = BitmapStreamDecoder::default().decode_bitmap_stream_to_rgb24(
        input.src,
        &mut out,
        input.width as usize,
        input.height as usize,
    );
}
