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
    let _ = decode::<McsMessage<'_>>(data);
    let _ = decode::<ConnectInitial>(data);
    let _ = decode::<ConnectResponse>(data);
    let _ = decode::<ClientInfoPdu>(data);
    let _ = decode::<capability_sets::CapabilitySet>(data);
    let _ = decode::<headers::ShareControlHeader>(data);
    let _ = decode::<pcb::PreconnectionBlob>(data);
    let _ = decode::<server_error_info::ServerSetErrorInfoPdu>(data);

    let _ = decode::<gcc::ClientGccBlocks>(data);
    let _ = decode::<gcc::ServerGccBlocks>(data);
    let _ = decode::<gcc::ClientClusterData>(data);
    let _ = decode::<gcc::ConferenceCreateRequest>(data);
    let _ = decode::<gcc::ConferenceCreateResponse>(data);

    let _ = decode::<server_license::LicensePdu>(data);
    let _ = decode::<server_license::LicensePdu>(data);
    let _ = decode::<server_license::LicensePdu>(data);
    let _ = decode::<server_license::LicensePdu>(data);
    let _ = decode::<server_license::LicensePdu>(data);

    let _ = decode::<vc::ChannelPduHeader>(data);

    let _ = decode::<fast_path::FastPathHeader>(data);
    let _ = decode::<fast_path::FastPathUpdatePdu<'_>>(data);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::Orders);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::Bitmap);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::Palette);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::Synchronize);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::SurfaceCommands);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::HiddenPointer);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::DefaultPointer);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::PositionPointer);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::ColorPointer);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::CachedPointer);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::NewPointer);
    let _ = fast_path::FastPathUpdate::decode_with_code(data, fast_path::UpdateCode::LargePointer);

    let _ = decode::<surface_commands::SurfaceCommand<'_>>(data);
    let _ = decode::<surface_commands::SurfaceBitsPdu<'_>>(data);
    let _ = decode::<surface_commands::FrameMarkerPdu>(data);
    let _ = decode::<surface_commands::ExtendedBitmapDataPdu<'_>>(data);
    let _ = decode::<surface_commands::BitmapDataHeader>(data);

    let _ = codecs::rfx::Headers::from_buffer(data);
    let _ = decode::<codecs::rfx::FrameAcknowledgePdu>(data);
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
    let _ = codecs::rfx::RfxChannel::from_buffer(data);

    let _ = decode::<input::InputEventPdu>(data);
    let _ = decode::<input::InputEvent>(data);

    let _ = decode::<bitmap::rdp6::BitmapStream<'_>>(data);

    let _ = decode::<ironrdp_cliprdr::pdu::ClipboardPdu<'_>>(data);

    let _ = decode::<ironrdp_rdpdr::pdu::RdpdrPdu>(data);

    let _ = decode::<ironrdp_displaycontrol::pdu::DisplayControlPdu>(data);
}

pub fn rle_decompress_bitmap(input: BitmapInput<'_>) {
    let mut out = Vec::new();

    let _ = ironrdp_graphics::rle::decompress_24_bpp(input.src, &mut out, input.width.into(), input.height.into());
    let _ = ironrdp_graphics::rle::decompress_16_bpp(input.src, &mut out, input.width.into(), input.height.into());
    let _ = ironrdp_graphics::rle::decompress_15_bpp(input.src, &mut out, input.width.into(), input.height.into());
    let _ = ironrdp_graphics::rle::decompress_8_bpp(input.src, &mut out, input.width.into(), input.height.into());
}

pub fn rdp6_encode_bitmap_stream(input: &BitmapInput<'_>) {
    use ironrdp_graphics::rdp6::{BitmapStreamEncoder, RgbAChannels, RgbChannels};

    let mut out = vec![0; input.src.len() * 2];

    let _ = BitmapStreamEncoder::new(input.width.into(), input.height.into()).encode_bitmap::<RgbChannels>(
        input.src,
        out.as_mut_slice(),
        false,
    );

    let _ = BitmapStreamEncoder::new(input.width.into(), input.height.into()).encode_bitmap::<RgbAChannels>(
        input.src,
        out.as_mut_slice(),
        true,
    );
}

pub fn rdp6_decode_bitmap_stream_to_rgb24(input: &BitmapInput<'_>) {
    use ironrdp_graphics::rdp6::BitmapStreamDecoder;

    let mut out = Vec::new();

    let _ = BitmapStreamDecoder::default().decode_bitmap_stream_to_rgb24(
        input.src,
        &mut out,
        input.width as usize,
        input.height as usize,
    );
}

pub fn cliprdr_format(input: &[u8]) {
    use ironrdp_cliprdr_format::bitmap::{dib_to_png, dibv5_to_png, png_to_cf_dib, png_to_cf_dibv5};
    use ironrdp_cliprdr_format::html::{cf_html_to_plain_html, plain_html_to_cf_html};

    let _ = png_to_cf_dib(input);
    let _ = png_to_cf_dibv5(input);

    let _ = dib_to_png(input);
    let _ = dibv5_to_png(input);

    let _ = cf_html_to_plain_html(input);

    if let Ok(input) = core::str::from_utf8(input) {
        let _ = plain_html_to_cf_html(input);
    }
}

pub fn channel_process(input: &[u8]) {
    use ironrdp_svc::SvcProcessor;

    let mut rdpdr = ironrdp_rdpdr::Rdpdr::new(Box::new(ironrdp_rdpdr::NoopRdpdrBackend), "Backend".to_owned())
        .with_smartcard(1)
        .with_drives(None);

    let _ = rdpdr.process(input);
}
