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

// Bulk decompression oracles. Each target is algorithm-pinned so libFuzzer
// can build a per-algorithm corpus. The `flags` byte uses the bit layout
// from `ironrdp-bulk::flags`: low nibble selects the algorithm (per
// `CompressionType::from_flags`), `PACKET_COMPRESSED (0x20)` gates whether
// the decompressor will actually run (otherwise it returns the source slice
// unchanged).

pub fn bulk_decompress_mppc(data: &[u8]) {
    use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

    // First byte selects RDP4 (low bit clear) vs RDP5 (low bit set) so a
    // single corpus exercises both MPPC modes via libFuzzer mutation across
    // the byte boundary.
    let Some((mode_byte, payload)) = data.split_first() else {
        return;
    };
    let (comp_type, algo_bits) = if mode_byte & 0x01 == 0 {
        (CompressionType::Rdp4, 0x00)
    } else {
        (CompressionType::Rdp5, 0x01)
    };
    let Ok(mut bulk) = BulkCompressor::new(comp_type) else {
        return;
    };
    let _ = bulk.decompress(payload, flags::PACKET_COMPRESSED | algo_bits);
}

pub fn bulk_decompress_ncrush(data: &[u8]) {
    use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

    let Ok(mut bulk) = BulkCompressor::new(CompressionType::Rdp6) else {
        return;
    };
    let _ = bulk.decompress(data, flags::PACKET_COMPRESSED | 0x02);
}

pub fn bulk_decompress_xcrush(data: &[u8]) {
    use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

    let Ok(mut bulk) = BulkCompressor::new(CompressionType::Rdp61) else {
        return;
    };
    let _ = bulk.decompress(data, flags::PACKET_COMPRESSED | 0x03);
}

/// Round-trip oracle: compress uncompressed input then decompress the result,
/// assert byte-equality with the original. `BulkCompressor` holds both halves;
/// a fresh compressor and decompressor are constructed per call to avoid
/// sliding-window state leaking between fuzz iterations.
///
/// # Panics
///
/// Panics (reporting the bug to libFuzzer) when:
/// - `decompress` returns `Err` on input that `compress` just produced
///   (asymmetric compress/decompress bug), or
/// - the decompressed output does not equal the original input
///   (silent corruption bug in either half).
#[expect(clippy::panic, reason = "panic is the libFuzzer bug-reporting mechanism")]
pub fn bulk_round_trip(data: &[u8]) {
    use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

    // First byte selects algorithm; remaining bytes are the uncompressed input.
    let Some((algo_byte, src)) = data.split_first() else {
        return;
    };
    let algo = match algo_byte & 0x03 {
        0x00 => CompressionType::Rdp4,
        0x01 => CompressionType::Rdp5,
        0x02 => CompressionType::Rdp6,
        _ => CompressionType::Rdp61,
    };

    let Ok(mut sender) = BulkCompressor::new(algo) else {
        return;
    };
    let Ok((compressed_size, compress_flags)) = sender.compress(src) else {
        return;
    };
    // Per `BulkCompressor::compress`'s contract, when `PACKET_COMPRESSED` is
    // cleared the caller transmits `src` unchanged; the output buffer holds
    // no meaningful data in that case. Selecting the wire payload here
    // exercises both the real compressed path and the decompressor's
    // pass-through branch on incompressible inputs.
    let payload = if compress_flags & flags::PACKET_COMPRESSED == 0 {
        src
    } else {
        sender.compressed_data(compressed_size)
    };

    let Ok(mut receiver) = BulkCompressor::new(algo) else {
        return;
    };
    let decompressed = receiver
        .decompress(payload, compress_flags)
        .unwrap_or_else(|e| panic!("bulk round-trip decompress failed for {algo:?}: {e:?}"));
    assert_eq!(decompressed, src, "bulk round-trip byte-equality failed for {algo:?}",);
}

pub fn pdu_decode(data: &[u8]) {
    use ironrdp_core::decode;
    use ironrdp_egfx::pdu::{
        Avc420BitmapStream, Avc444BitmapStream, CacheToSurfacePdu, Color, GfxPdu, Point, QuantQuality,
        RawCapabilitySet as EgfxRawCapabilitySet,
    };
    use ironrdp_pdu::mcs::{ConnectInitial, ConnectResponse, McsMessage};
    use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest};
    use ironrdp_pdu::rdp::{ClientInfoPdu, capability_sets, headers, server_error_info, server_license, vc};
    use ironrdp_pdu::x224::X224;
    use ironrdp_pdu::{bitmap, codecs, fast_path, gcc, input, pcb, surface_commands};

    let _ = decode::<X224<ConnectionRequest>>(data);
    let _ = decode::<X224<ConnectionConfirm>>(data);
    let _ = decode::<X224<McsMessage<'_>>>(data);
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

    let _ = decode::<codecs::rfx::Block<'_>>(data);

    let _ = decode::<input::InputEventPdu>(data);
    let _ = decode::<input::InputEvent>(data);

    let _ = decode::<bitmap::rdp6::BitmapStream<'_>>(data);

    let _ = decode::<ironrdp_cliprdr::pdu::ClipboardPdu<'_>>(data);
    let _ = decode::<ironrdp_cliprdr::pdu::PackedFileList>(data);
    let _ = decode::<ironrdp_cliprdr::pdu::FileContentsRequest>(data);
    let _ = decode::<ironrdp_cliprdr::pdu::FileContentsResponse<'_>>(data);

    let _ = decode::<ironrdp_rdpdr::pdu::RdpdrPdu>(data);

    let _ = decode::<ironrdp_displaycontrol::pdu::DisplayControlPdu>(data);

    let _ = decode::<ironrdp_rdpsnd::pdu::ServerAudioOutputPdu<'_>>(data);
    let _ = decode::<ironrdp_rdpsnd::pdu::ClientAudioOutputPdu>(data);

    let _ = decode::<GfxPdu>(data);
    let _ = decode::<CacheToSurfacePdu>(data);
    let _ = decode::<EgfxRawCapabilitySet>(data);
    let _ = decode::<Avc420BitmapStream<'_>>(data);
    let _ = decode::<Avc444BitmapStream<'_>>(data);
    let _ = decode::<QuantQuality>(data);
    let _ = decode::<Point>(data);
    let _ = decode::<Color>(data);
}

/// Helper for [`pdu_round_trip`].
///
/// Exercises `decode` → `encode_vec` → re-`decode`, silently dropping `Err`
/// results from any stage. The oracle's value is in detecting INTERNAL
/// panics from inside the encoder/decoder (e.g., `unreachable!()` reached
/// on a valid decoded state), not in asserting Err-result symmetry. Many
/// `ironrdp-pdu` types have known asymmetric `Encode` impls that return
/// `"Encoding not implemented"` for variants the decoder still accepts;
/// those are tracked separately and not in scope for this oracle.
macro_rules! pdu_round_trip_one {
    ($data:expr, $ty:ty) => {{
        if let Ok(pdu) = ironrdp_core::decode::<$ty>($data) {
            if let Ok(encoded) = ironrdp_core::encode_vec(&pdu) {
                let _ = ironrdp_core::decode::<$ty>(&encoded);
            }
        }
    }};
}

/// Round-trip oracle: for each PDU type, exercise the
/// `decode` → `encode_vec` → re-`decode` pipeline.
///
/// The property tested is *no internal panic from inside the encoder or
/// decoder when fed a decoder-accepted input through both directions of the
/// round-trip*. Asymmetric `Err` returns (decoder accepts something the
/// encoder reports as `"Encoding not implemented"`, or vice-versa) are not
/// in scope: those are tolerated incomplete-impl cases tracked separately.
///
/// What this catches:
///
/// - `unreachable!()` reached during encoding of a valid decoded state (i.e.
///   the encoder's match arms are missing a variant the decoder produces).
/// - Integer overflow / index-out-of-bounds inside the encoder on
///   decoder-accepted inputs.
/// - Panics in the decoder when fed encoder-produced bytes (re-decode path).
///
/// What this does NOT catch:
///
/// - Encode returning `Err`. Many PDU types intentionally return errors for
///   partially-implemented variants; exercising them is the encoder
///   developer's responsibility, not this oracle's.
/// - Re-decode returning `Err`. Surfaces an asymmetry but not a memory-safety
///   bug; tracked via filed follow-up issues, not this oracle.
///
/// Type coverage matches `pdu_decode`: any input that exercises `pdu_decode`'s
/// decoder set also exercises this oracle's round-trip assertions by
/// construction. As new PDU types gain `Encode` impls, they auto-extend
/// coverage here when added to the macro list below.
pub fn pdu_round_trip(data: &[u8]) {
    use ironrdp_pdu::mcs::{ConnectInitial, ConnectResponse, McsMessage};
    use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest};
    use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
    use ironrdp_pdu::rdp::headers::ShareControlHeader;
    use ironrdp_pdu::rdp::{ClientInfoPdu, server_error_info, server_license, vc};
    use ironrdp_pdu::x224::X224;
    use ironrdp_pdu::{bitmap, codecs, fast_path, gcc, input, pcb, surface_commands};

    // Connection-time PDUs
    pdu_round_trip_one!(data, X224<ConnectionRequest>);
    pdu_round_trip_one!(data, X224<ConnectionConfirm>);
    pdu_round_trip_one!(data, X224<McsMessage<'_>>);
    pdu_round_trip_one!(data, ConnectInitial);
    pdu_round_trip_one!(data, ConnectResponse);
    pdu_round_trip_one!(data, ClientInfoPdu);
    pdu_round_trip_one!(data, pcb::PreconnectionBlob);
    pdu_round_trip_one!(data, server_error_info::ServerSetErrorInfoPdu);

    // Capability sharing
    pdu_round_trip_one!(data, CapabilitySet);
    pdu_round_trip_one!(data, ShareControlHeader);

    // GCC blocks and conference creation
    pdu_round_trip_one!(data, gcc::ClientGccBlocks);
    pdu_round_trip_one!(data, gcc::ServerGccBlocks);
    pdu_round_trip_one!(data, gcc::ClientClusterData);
    pdu_round_trip_one!(data, gcc::ConferenceCreateRequest);
    pdu_round_trip_one!(data, gcc::ConferenceCreateResponse);

    // Licensing
    pdu_round_trip_one!(data, server_license::LicensePdu);

    // Virtual channel header
    pdu_round_trip_one!(data, vc::ChannelPduHeader);

    // Fast-path framing
    pdu_round_trip_one!(data, fast_path::FastPathHeader);
    pdu_round_trip_one!(data, fast_path::FastPathUpdatePdu<'_>);

    // Surface commands
    pdu_round_trip_one!(data, surface_commands::SurfaceCommand<'_>);
    pdu_round_trip_one!(data, surface_commands::SurfaceBitsPdu<'_>);
    pdu_round_trip_one!(data, surface_commands::FrameMarkerPdu);
    pdu_round_trip_one!(data, surface_commands::ExtendedBitmapDataPdu<'_>);
    pdu_round_trip_one!(data, surface_commands::BitmapDataHeader);

    // Codecs
    pdu_round_trip_one!(data, codecs::rfx::Block<'_>);

    // Input
    pdu_round_trip_one!(data, input::InputEventPdu);
    pdu_round_trip_one!(data, input::InputEvent);

    // Bitmap RDP6
    pdu_round_trip_one!(data, bitmap::rdp6::BitmapStream<'_>);

    // Clipboard
    pdu_round_trip_one!(data, ironrdp_cliprdr::pdu::ClipboardPdu<'_>);
    pdu_round_trip_one!(data, ironrdp_cliprdr::pdu::PackedFileList);
    pdu_round_trip_one!(data, ironrdp_cliprdr::pdu::FileContentsRequest);
    pdu_round_trip_one!(data, ironrdp_cliprdr::pdu::FileContentsResponse<'_>);

    // RDPDR
    pdu_round_trip_one!(data, ironrdp_rdpdr::pdu::RdpdrPdu);

    // Display control
    pdu_round_trip_one!(data, ironrdp_displaycontrol::pdu::DisplayControlPdu);

    // RDPSND
    pdu_round_trip_one!(data, ironrdp_rdpsnd::pdu::ServerAudioOutputPdu<'_>);
    pdu_round_trip_one!(data, ironrdp_rdpsnd::pdu::ClientAudioOutputPdu);
}

/// Round-trip oracle for `ironrdp-egfx` PDU types: `decode` → `encode_vec` → re-`decode`.
///
/// Same shape and property as [`pdu_round_trip`] but scoped to `ironrdp-egfx`'s
/// own encoder surface. This is the egfx-scoped sibling of the `pdu_round_trip`
/// oracle and the first target under the egfx fuzz-coverage umbrella tracked at
/// the egfx-fuzz issue.
///
/// Coverage:
///
/// - `GfxPdu` is the top-level egfx command dispatch and transitively covers
///   `WireToSurface1Pdu`, `WireToSurface2Pdu`, `SolidFillPdu`,
///   `SurfaceToSurfacePdu`, `SurfaceToCachePdu`, `CacheToSurfacePdu`,
///   `EvictCacheEntryPdu`, `CreateSurfacePdu`, `DeleteSurfacePdu`,
///   `StartFramePdu`, `EndFramePdu`, `ResetGraphicsPdu`,
///   `MapSurfaceToOutputPdu`, `MapSurfaceToWindowPdu`,
///   `MapSurfaceToScaledOutputPdu`, `MapSurfaceToScaledWindowPdu`,
///   `FrameAcknowledgePdu`, `QoeFrameAcknowledgePdu`,
///   `DeleteEncodingContextPdu`, `CacheImportOfferPdu`, `CacheImportReplyPdu`.
/// - `CapabilitiesAdvertisePdu` and `CapabilitiesConfirmPdu` exercise the
///   capability-negotiation encoder surface (with `RawCapabilitySet` payloads
///   post-#1305's wire/typed split).
/// - `Avc420BitmapStream` and `Avc444BitmapStream` exercise the H.264 wire
///   container encoder.
///
/// What this catches: same as `pdu_round_trip` — `unreachable!()` reached on
/// decoder-accepted inputs, integer overflow / OOB in egfx encoders, panics
/// in the decoder when fed encoder-produced bytes.
///
/// What this does NOT catch: the OpenH264 input-construction wrapper, ZGFX
/// decompression, multi-frame H.264 state. Those are sibling targets in the
/// egfx fuzz-coverage umbrella.
pub fn egfx_round_trip(data: &[u8]) {
    use ironrdp_egfx::pdu::{
        Avc420BitmapStream, Avc444BitmapStream, CapabilitiesAdvertisePdu, CapabilitiesConfirmPdu, GfxPdu,
    };

    pdu_round_trip_one!(data, GfxPdu);
    pdu_round_trip_one!(data, CapabilitiesAdvertisePdu);
    pdu_round_trip_one!(data, CapabilitiesConfirmPdu);
    pdu_round_trip_one!(data, Avc420BitmapStream<'_>);
    pdu_round_trip_one!(data, Avc444BitmapStream<'_>);
}

/// Helper for [`message_decoding_invariants`].
///
/// On every successful `decode`, asserts that `pdu.size()` accurately reports
/// the encoded length: `pdu.size() == encode_vec(&pdu).len()`. This is the
/// `Encode` trait's implicit soundness contract; downstream callers rely on
/// `size()` for buffer sizing (`ensure_size!`, `cast_length!`, etc.) and a lie
/// here produces buffer overflows or under-allocations in encode paths.
///
/// Distinct from [`pdu_round_trip`]: that oracle silently drops `Err` and
/// catches panics only. This oracle ASSERTS on the size contract, so a
/// violation aborts the fuzz iteration as a libFuzzer crash.
///
/// What this catches (that `pdu_round_trip` does not):
///
/// - `Encode::size()` lies about its own size (returns N but `encode` writes
///   N+M bytes or fails after writing some bytes), causing buffer
///   over-allocation or under-allocation in callers.
/// - Decode-acceptable bytes that the encoder cannot reconstruct to the same
///   length (lossy decode that loses framing structure).
///
/// What this does NOT catch (covered by other oracles):
///
/// - Decode-time panics or OOM (covered by `pdu_decode`).
/// - Re-decode equality after round-trip (covered by `pdu_round_trip`'s
///   silent-drop pattern; assertion-based re-decode is intentionally out of
///   scope here to keep the oracle's failure mode unambiguous).
macro_rules! decode_size_invariant_one {
    ($data:expr, $ty:ty) => {{
        use ironrdp_core::Encode as _;
        let mut cursor = ironrdp_core::ReadCursor::new($data);
        if let Ok(pdu) = ironrdp_core::decode_cursor::<$ty>(&mut cursor) {
            if let Ok(encoded) = ironrdp_core::encode_vec(&pdu) {
                let size_reported = pdu.size();
                let actual_len = encoded.len();
                assert!(
                    size_reported == actual_len,
                    "{} violates Encode::size() contract: pdu.size() = {}, encode_vec(&pdu).len() = {}",
                    ::core::any::type_name::<$ty>(),
                    size_reported,
                    actual_len
                );
            }
        }
    }};
}

/// Message-decoding invariants oracle: for each PDU type, exercise the
/// `decode -> size() -> encode_vec` pipeline and assert that the type's
/// reported `size()` matches the actual encoded length.
///
/// The property tested is the `Encode` trait soundness contract: on any
/// decoder-accepted input, the decoded PDU's `size()` method must accurately
/// report the byte length the encoder will produce. Caller code uses `size()`
/// for buffer sizing (`ensure_size!`, `cast_length!`), so a violation
/// produces real downstream bugs (under-allocation -> truncated encode,
/// over-allocation -> wasted memory or buffer-overflow risk depending on
/// surrounding context).
///
/// The bug class is distinct from `pdu_round_trip`: that oracle catches
/// encoder panics on decoder-accepted inputs, while this one catches the
/// strictly weaker but distinct class of size-contract violations.
///
/// Type coverage matches `pdu_round_trip`: any input that exercises the
/// round-trip oracle's decoder set also exercises this oracle's
/// size-invariant assertion by construction.
pub fn message_decoding_invariants(data: &[u8]) {
    use ironrdp_pdu::mcs::{ConnectInitial, ConnectResponse, McsMessage};
    use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest};
    use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
    use ironrdp_pdu::rdp::headers::ShareControlHeader;
    use ironrdp_pdu::rdp::{ClientInfoPdu, server_error_info, server_license, vc};
    use ironrdp_pdu::x224::X224;
    use ironrdp_pdu::{bitmap, codecs, fast_path, gcc, input, pcb, surface_commands};

    // Connection-time PDUs
    decode_size_invariant_one!(data, X224<ConnectionRequest>);
    decode_size_invariant_one!(data, X224<ConnectionConfirm>);
    decode_size_invariant_one!(data, X224<McsMessage<'_>>);
    decode_size_invariant_one!(data, ConnectInitial);
    decode_size_invariant_one!(data, ConnectResponse);
    decode_size_invariant_one!(data, ClientInfoPdu);
    decode_size_invariant_one!(data, pcb::PreconnectionBlob);
    decode_size_invariant_one!(data, server_error_info::ServerSetErrorInfoPdu);

    // Capability sharing. `ShareControlHeader` transits through `CapabilitySet`'s
    // encoder via the Active variants, so the same encoder path exercises both.
    decode_size_invariant_one!(data, CapabilitySet);
    decode_size_invariant_one!(data, ShareControlHeader);

    // GCC blocks and conference creation
    decode_size_invariant_one!(data, gcc::ClientGccBlocks);
    decode_size_invariant_one!(data, gcc::ServerGccBlocks);
    decode_size_invariant_one!(data, gcc::ClientClusterData);
    decode_size_invariant_one!(data, gcc::ConferenceCreateRequest);
    decode_size_invariant_one!(data, gcc::ConferenceCreateResponse);

    // Licensing
    decode_size_invariant_one!(data, server_license::LicensePdu);

    // Virtual channel header
    decode_size_invariant_one!(data, vc::ChannelPduHeader);

    // Fast-path framing
    decode_size_invariant_one!(data, fast_path::FastPathHeader);
    decode_size_invariant_one!(data, fast_path::FastPathUpdatePdu<'_>);

    // Surface commands
    decode_size_invariant_one!(data, surface_commands::SurfaceCommand<'_>);
    decode_size_invariant_one!(data, surface_commands::SurfaceBitsPdu<'_>);
    decode_size_invariant_one!(data, surface_commands::FrameMarkerPdu);
    decode_size_invariant_one!(data, surface_commands::ExtendedBitmapDataPdu<'_>);
    decode_size_invariant_one!(data, surface_commands::BitmapDataHeader);

    // Codecs
    decode_size_invariant_one!(data, codecs::rfx::Block<'_>);

    // Input
    decode_size_invariant_one!(data, input::InputEventPdu);
    decode_size_invariant_one!(data, input::InputEvent);

    // Bitmap RDP6
    decode_size_invariant_one!(data, bitmap::rdp6::BitmapStream<'_>);
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
        usize::from(input.width),
        usize::from(input.height),
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
    use ironrdp_svc::SvcProcessor as _;

    let mut rdpdr = ironrdp_rdpdr::Rdpdr::new(Box::new(ironrdp_rdpdr::NoopRdpdrBackend), "Backend".to_owned())
        .with_smartcard(1)
        .with_drives(None);

    let _ = rdpdr.process(input);
}

pub fn cliprdr_channel_process(input: &[u8]) {
    use ironrdp_svc::SvcProcessor as _;

    let mut cliprdr = ironrdp_cliprdr::Cliprdr::<ironrdp_cliprdr::Client>::new(Box::new(NoopCliprdrFuzzBackend));
    let _ = cliprdr.process(input);
}

/// Minimal backend for fuzzing that enables file transfer capabilities
/// so the fuzzer can exercise lock, file list, and file contents paths.
#[derive(Debug)]
struct NoopCliprdrFuzzBackend;

ironrdp_core::impl_as_any!(NoopCliprdrFuzzBackend);

impl ironrdp_cliprdr::backend::CliprdrBackend for NoopCliprdrFuzzBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }

    fn client_capabilities(&self) -> ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags {
        use ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags;
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _: &[ironrdp_cliprdr::pdu::ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _: ironrdp_cliprdr::pdu::FormatDataRequest) {}
    fn on_format_data_response(&mut self, _: ironrdp_cliprdr::pdu::FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _: ironrdp_cliprdr::pdu::FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: ironrdp_cliprdr::pdu::FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: ironrdp_cliprdr::pdu::LockDataId) {}
    fn on_unlock(&mut self, _: ironrdp_cliprdr::pdu::LockDataId) {}

    // Fixed clock so fuzz runs are reproducible regardless of wall-clock timing
    fn now_ms(&self) -> u64 {
        0
    }
}
