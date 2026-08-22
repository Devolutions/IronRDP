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
    let mut bulk = BulkCompressor::new(comp_type);
    let _ = bulk.decompress(payload, flags::PACKET_COMPRESSED | algo_bits);
}

pub fn bulk_decompress_ncrush(data: &[u8]) {
    use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

    let mut bulk = BulkCompressor::new(CompressionType::Rdp6);
    let _ = bulk.decompress(data, flags::PACKET_COMPRESSED | 0x02);
}

pub fn bulk_decompress_xcrush(data: &[u8]) {
    use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

    let mut bulk = BulkCompressor::new(CompressionType::Rdp61);
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

    let mut sender = BulkCompressor::new(algo);
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

    let mut receiver = BulkCompressor::new(algo);
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
    use ironrdp_pdu::rdp::{
        ClientInfoPdu, capability_sets, headers, multitransport, server_error_info, server_license, vc,
    };
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

    // Post-licensing multitransport bootstrapping: the connector try-decodes
    // arbitrary server bytes as a request to distinguish it from Demand Active.
    let _ = decode::<multitransport::MultitransportRequestPdu>(data);
    let _ = decode::<multitransport::MultitransportResponsePdu>(data);

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
/// Exercises `decode` → `encode_vec` → re-`decode` → re-`encode_vec`.
///
/// A failing `decode` of the fuzzer's input is expected and skipped, and a
/// failing `encode` is tolerated because several `ironrdp-pdu` types return
/// `"Encoding not implemented"` for variants the decoder still accepts.
///
/// A *successful* encode is asserted to be re-decodable, because at that point the
/// bytes were produced by this crate from a state this crate accepted. Emitting
/// bytes we cannot read back is an encoder/decoder disagreement, and on the wire
/// that is a peer refusing our PDU.
///
/// Byte stability across the round trip is deliberately NOT asserted. Several
/// decoders normalise: `LogonInfoVersion1` reads `domainNameSize`, range-checks it,
/// and then keeps only the trimmed string, so a PDU whose size field disagrees with
/// its own padding cannot re-encode to identical bytes no matter how correct both
/// halves are. That is a property of types that discard redundant wire fields, not
/// a defect, so asserting it would report design as breakage.
macro_rules! pdu_round_trip_one {
    ($data:expr, $ty:ty) => {{
        if let Ok(pdu) = ironrdp_core::decode::<$ty>($data) {
            if let Ok(encoded) = ironrdp_core::encode_vec(&pdu) {
                if let Err(e) = ironrdp_core::decode::<$ty>(&encoded) {
                    panic!(
                        "{}: encoded {} bytes that failed to decode again: {e}",
                        stringify!($ty),
                        encoded.len(),
                    );
                }
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
/// - An encoder that emits bytes it cannot read back.
///
/// What this does NOT catch:
///
/// - Encode returning `Err`. Many PDU types intentionally return errors for
///   partially-implemented variants; exercising them is the encoder
///   developer's responsibility, not this oracle's.
///
/// Re-decode returning `Err` used to be excluded here, on the grounds that an
/// encode/decode disagreement is not a memory-safety bug and could be tracked
/// separately. In practice it was not: the `BandwidthMeasureStop` asymmetry
/// fixed in the preceding commit went unnoticed because nothing asserted this,
/// and it was found by hand while writing an unrelated test. Emitting bytes we
/// cannot read back is a real defect on the wire, so it is asserted now.
///
/// Initial type coverage mirrors `pdu_decode` so the same corpus feeds both
/// oracles. As new PDU types gain `Encode` impls, they auto-extend coverage
/// here when added to the macro list below.
pub fn pdu_round_trip(data: &[u8]) {
    use ironrdp_pdu::mcs::{ConnectInitial, ConnectResponse, McsMessage};
    use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest};
    use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
    use ironrdp_pdu::rdp::headers::ShareControlHeader;
    use ironrdp_pdu::rdp::{self, ClientInfoPdu, multitransport, server_error_info, server_license, vc};
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

    // Multitransport bootstrapping (server request / client response)
    pdu_round_trip_one!(data, multitransport::MultitransportRequestPdu);
    pdu_round_trip_one!(data, multitransport::MultitransportResponsePdu);

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

    // Network auto-detect. The `BandwidthMeasureStop` encode/decode asymmetry fixed in
    // the preceding commit lives here; with the re-decode assertion above, this coverage
    // is what would have caught it.
    pdu_round_trip_one!(data, rdp::autodetect::AutoDetectReqPdu);
    pdu_round_trip_one!(data, rdp::autodetect::AutoDetectRspPdu);

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
/// What this does NOT catch: the OpenH264 input-construction wrapper,
/// multi-frame H.264 state. Those are sibling targets in the egfx
/// fuzz-coverage umbrella. ZGFX decompression is covered by
/// [`egfx_zgfx_decompress`].
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

/// AVC420 decode-side wrapper fuzz oracle.
///
/// Fuzzes the IronRDP wrapper layer between a wire `Avc420BitmapStream` and
/// the consumer's `H264Decoder`. Specifically targets `avc_to_annex_b`, the
/// AVC-length-prefix to Annex-B conversion that runs before OpenH264 sees
/// any bytes.
///
/// The oracle runs two paths on each input:
///
/// - Direct: call `avc_to_annex_b(data)` on the raw fuzz input. This
///   exercises the wrapper on arbitrary byte distributions, including
///   inputs that do not parse as `Avc420BitmapStream`.
/// - Decode-chain: try `Avc420BitmapStream::decode(data)`; on success, call
///   `avc_to_annex_b(stream.data)`. This exercises the wrapper on the
///   realistic post-decode payload distribution.
///
/// What this catches: panics in the wrapper, OOM via attacker-controlled
/// NAL length encoding, contract violations on the produced Annex-B byte
/// stream that downstream H264Decoder callers rely on.
///
/// What this does NOT catch: OpenH264 itself (covered by OSS-Fuzz), the
/// post-OpenH264 YUV-to-RGBA conversion path in `OpenH264Decoder::decode`,
/// AVC444 luma plus chroma split (covered by a sibling target).
pub fn egfx_avc420_decode(data: &[u8]) {
    use ironrdp_egfx::pdu::{Avc420BitmapStream, avc_to_annex_b};

    let _ = avc_to_annex_b(data);

    let mut cursor = ironrdp_core::ReadCursor::new(data);
    if let Ok(stream) = ironrdp_core::decode_cursor::<Avc420BitmapStream<'_>>(&mut cursor) {
        let _ = avc_to_annex_b(stream.data);
    }
}

/// Multi-frame oracle for the EGFX graphics pipeline client.
///
/// H.264 decoding maintains reference-picture state, SPS/PPS context, and
/// decoder configuration across frames; surface caching and codec dispatch
/// state in egfx all carry forward across PDUs. Single-shot fuzzers cannot
/// reach frame-to-frame state corruption because they construct a fresh
/// decoder per iteration. This oracle constructs ONE `GraphicsPipelineClient`
/// at iteration start and drives a sequence of `GfxPdu`s through it, exposing
/// cross-PDU state to the fuzzer.
///
/// Harness shape: `Arbitrary`-derived `Vec<GfxPdu>` (each variant `Arbitrary`
/// via the cascade in PR #1334). Each PDU is encoded back to wire bytes,
/// wrapped in a single uncompressed ZGFX segment, and fed to the client's
/// public `DvcProcessor::process` entry point. This exercises the same path
/// production traffic takes: ZGFX decompress -> `GfxPdu` decode -> dispatch
/// to per-variant handler -> state machine + surface cache update.
///
/// What this catches: panics or sanitizer reports along the dispatch + state
/// machine path when fed adversarially-ordered or malformed-payload PDUs;
/// inconsistent surface-cache state under attacker-controlled
/// CreateSurface / DeleteSurface / Map* orderings; corrupted frame-id state
/// from interleaved StartFrame / EndFrame / FrameAcknowledge sequences;
/// ZGFX-wrapper integration bugs separate from the standalone ZGFX coverage
/// in `egfx_zgfx_decompress`.
///
/// What this does NOT catch: cross-frame H.264 decoder state corruption.
/// The client is constructed with `h264_decoder: None`, so H264-bearing
/// PDUs (WireToSurface1 with AVC codecs) don't reach the H.264 decoder.
/// The standalone `egfx_avc420_decode` and `egfx_avc444_decode` targets
/// cover the H.264 wrapper. Wiring a real (or mock) H.264 decoder into
/// this harness can be a follow-up if frame-to-frame H.264 state coverage
/// surfaces as a gap.
pub fn egfx_multi_frame(data: &[u8]) {
    use arbitrary::{Arbitrary as _, Unstructured};
    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::pdu::GfxPdu;
    use ironrdp_graphics::zgfx::wrap_uncompressed;

    /// No-op handler. Every callback default-impls in the trait, so the empty
    /// struct gets all defaults for free. The handler exists to satisfy
    /// `GraphicsPipelineClient::new`'s API; the fuzz oracle does not inspect
    /// any of the dispatched events.
    struct NoOpHandler;
    impl GraphicsPipelineHandler for NoOpHandler {}

    let mut unstructured = Unstructured::new(data);
    let Ok(pdus) = Vec::<GfxPdu>::arbitrary(&mut unstructured) else {
        return;
    };

    let mut client = GraphicsPipelineClient::new(Box::new(NoOpHandler), None);

    // Initialise the channel state by invoking the DvcProcessor::start entry.
    // The returned advertise message is discarded; the call's side effect is
    // putting the client's internal state machine into its post-start state.
    const FUZZ_CHANNEL_ID: u32 = 0;
    let _ = client.start(FUZZ_CHANNEL_ID);

    for pdu in pdus {
        // Encode each PDU back to wire bytes so the client processes through
        // the same decode + dispatch path real traffic takes. Skip PDUs whose
        // encoder rejects the Arbitrary-generated values rather than aborting
        // the iteration; the next PDU may still exercise interesting state.
        let Ok(pdu_bytes) = encode_vec(&pdu) else {
            continue;
        };

        // Wrap the encoded PDU in an uncompressed ZGFX segment so the client's
        // ZGFX decompressor produces the PDU bytes unmodified. This bypasses
        // the ZGFX decoder layer (covered separately by egfx_zgfx_decompress)
        // and concentrates fuzz pressure on the dispatch + state machine.
        let payload = wrap_uncompressed(&pdu_bytes);

        // Errors and panics propagate to libFuzzer naturally; we discard the
        // Result since the oracle's job is to surface bugs, not to enforce
        // dispatcher semantics.
        let _ = client.process(FUZZ_CHANNEL_ID, &payload);
    }
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

/// The URBDRC (MS-RDPEUSB / USB redirection) client→server PDUs a server
/// decodes off the wire. `UrbdrcClientControlPdu` is the main-channel family;
/// `UrbdrcClientDevicePdu<Raw>` is the per-device family, which carries the URB
/// completions, IO-control completions and interface-info results.
///
/// Decoding `UrbdrcClientDevicePdu<Raw>` only slurps each URB *result* body into
/// `Raw` — the operation-specific reinterpretation of those length-prefixed
/// payloads (`SELECT_CONFIGURATION` / `SELECT_INTERFACE` / interface-info / isoch
/// results, the historical home of unchecked-read panics) happens later in the
/// stateful server handlers via `into_expected`, so `<Raw>` alone never reaches
/// them. Those result decoders are all public, so we also drive them directly on
/// the raw input to keep the whole bounds-checked decode surface fuzzed.
pub fn rdpeusb_decode(data: &[u8]) {
    use ironrdp_core::{ReadCursor, decode};
    use ironrdp_rdpeusb::pdu::completion::ts_urb_result::{
        Raw, TsUrbGetCurrFrameNumResult, TsUrbIsochTransferResult, TsUrbSelectConfigResult, TsUrbSelectInterfaceResult,
        TsUsbdInterfaceInfoResult, TsUsbdPipeInfoResult,
    };
    use ironrdp_rdpeusb::pdu::{UrbdrcClientControlPdu, UrbdrcClientDevicePdu};

    // Top-level client→server PDU families, off the two DVC channels.
    let _ = decode::<UrbdrcClientControlPdu>(data);
    let _ = decode::<UrbdrcClientDevicePdu<Raw>>(data);

    // The length-prefixed URB / interface-info *result* payload decoders the
    // per-device family reinterprets `Raw` into once correlated with a request.
    let _ = TsUrbSelectConfigResult::decode(&mut ReadCursor::new(data));
    let _ = TsUrbSelectInterfaceResult::decode(&mut ReadCursor::new(data));
    let _ = TsUrbGetCurrFrameNumResult::decode(&mut ReadCursor::new(data));
    let _ = TsUrbIsochTransferResult::decode(&mut ReadCursor::new(data));
    let _ = decode::<TsUsbdInterfaceInfoResult>(data);
    let _ = decode::<TsUsbdPipeInfoResult>(data);
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

// RDP-UDP transport oracles.
//
// The transport splits into three layers that fail in different ways, so the
// oracles are layered to match rather than pointing one decode target at all of
// it. From cheapest to most stateful:
//
//   1. `rdpeudp_pdu_decode` / `rdpemt_pdu_decode`: hostile bytes into every
//      wire structure. Crash-only, and the baseline the others build on.
//   2. `rdpeudp_pdu_round_trip` / `rdpemt_pdu_round_trip`: structure-aware.
//      These assert re-decode equality and, unlike the crash-only targets,
//      that `size()` agrees with what `encode` actually wrote. A size that
//      under-reports is how a decoder gets handed a truncated buffer.
//   3. `rdpeudp_prefix_transform`, `rdpeudp_ack_vector`,
//      `rdpeudp_connection`, `rdpemt_tunnel`: the pieces with state or
//      attacker-influenced allocation, where a crash-only oracle is blind.

/// Decodes hostile bytes as each RDP-UDP wire structure.
///
/// The first byte selects the structure so libFuzzer builds a per-structure
/// corpus rather than averaging one coverage signal across all of them.
pub fn rdpeudp_pdu_decode(data: &[u8]) {
    use ironrdp_core::decode;
    use ironrdp_rdpeudp::pdu::{
        AckOfAcksPayload, AckPayload, AckVectorPayload, CorrelationIdPayload, DataHeader, DelayAckInfoPayload,
        FecHeader, OverheadSizePayload, SynDataExPayload, SynDataPayload, V1AckOfAcksHeader, V1AckVectorHeader,
        V1Datagram, V2Header, V2Packet,
    };

    let Some((selector, payload)) = data.split_first() else {
        return;
    };

    match selector % 15 {
        0 => drop(decode::<V1Datagram>(payload)),
        1 => drop(decode::<FecHeader>(payload)),
        2 => drop(decode::<V1AckVectorHeader>(payload)),
        3 => drop(decode::<V1AckOfAcksHeader>(payload)),
        4 => drop(decode::<SynDataPayload>(payload)),
        5 => drop(decode::<SynDataExPayload>(payload)),
        6 => drop(decode::<CorrelationIdPayload>(payload)),
        7 => drop(decode::<V2Packet>(payload)),
        8 => drop(decode::<V2Header>(payload)),
        9 => drop(decode::<AckPayload>(payload)),
        10 => drop(decode::<AckVectorPayload>(payload)),
        11 => drop(decode::<AckOfAcksPayload>(payload)),
        12 => drop(decode::<DelayAckInfoPayload>(payload)),
        13 => drop(decode::<OverheadSizePayload>(payload)),
        _ => drop(decode::<DataHeader>(payload)),
    }
}

/// Decodes hostile bytes as each multitransport wire structure.
pub fn rdpemt_pdu_decode(data: &[u8]) {
    use ironrdp_core::decode;
    use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu};
    use ironrdp_rdpemt::pdu::{
        TunnelCreateRequest, TunnelCreateResponse, TunnelData, TunnelHeader, TunnelPdu, TunnelSubHeader,
    };

    let Some((selector, payload)) = data.split_first() else {
        return;
    };

    match selector % 8 {
        0 => drop(decode::<TunnelPdu>(payload)),
        1 => drop(decode::<TunnelHeader>(payload)),
        2 => drop(decode::<TunnelSubHeader>(payload)),
        3 => drop(decode::<TunnelCreateRequest>(payload)),
        4 => drop(decode::<TunnelCreateResponse>(payload)),
        5 => drop(decode::<TunnelData>(payload)),
        6 => drop(decode::<MultitransportRequestPdu>(payload)),
        _ => drop(decode::<MultitransportResponsePdu>(payload)),
    }
}

/// Round-trips structure-aware RDP-UDP PDUs and checks `size()` against the
/// bytes `encode` actually wrote.
///
/// The size check is the part a decode-only oracle cannot reach. `encode_vec`
/// allocates from `size()`, so a `size()` that over-reports leaves trailing
/// zeros that re-decode happens to tolerate, and one that under-reports would
/// have panicked inside the cursor long before any decoder saw it.
pub fn rdpeudp_pdu_round_trip(data: &[u8]) {
    use arbitrary::{Arbitrary as _, Unstructured};
    use ironrdp_core::{decode, encode_vec};
    use ironrdp_rdpeudp::pdu::{V1Datagram, V2Packet};

    fn check<T>(pdu: &T)
    where
        T: ironrdp_core::Encode + core::fmt::Debug + PartialEq + for<'a> ironrdp_core::Decode<'a>,
    {
        let Ok(encoded) = encode_vec(pdu) else {
            return;
        };

        assert_eq!(
            pdu.size(),
            encoded.len(),
            "size() disagrees with the encoded length for {pdu:?}"
        );

        // Re-decoding our own bytes must succeed, and re-encoding the result
        // must reproduce them exactly.
        //
        // The comparison is on bytes rather than values on purpose. The v1 and
        // v2 encoders derive the payload-gating flags from which `Option`
        // fields are populated, so an `Arbitrary` value that sets a flag
        // without the payload it gates is normalised on the way out. That is
        // the encoder behaving correctly. What must not happen is the wire
        // form changing on a second pass, which is what silent corruption
        // would look like from a peer.
        match decode::<T>(&encoded) {
            Ok(reparsed) => match encode_vec(&reparsed) {
                Ok(re_encoded) => assert_eq!(
                    encoded, re_encoded,
                    "re-encoding a decoded {pdu:?} produced different bytes"
                ),
                Err(e) => panic!("re-encoding a value we just decoded failed: {e}"),
            },
            Err(e) => panic!("re-decoding our own encoding failed: {e}"),
        }
    }

    let mut unstructured = Unstructured::new(data);
    let Ok(select_v2) = bool::arbitrary(&mut unstructured) else {
        return;
    };

    if select_v2 {
        if let Ok(packet) = V2Packet::arbitrary(&mut unstructured) {
            check(&packet);
        }
    } else if let Ok(datagram) = V1Datagram::arbitrary(&mut unstructured) {
        check(&datagram);
    }
}

/// Round-trips structure-aware multitransport PDUs with the same size check.
pub fn rdpemt_pdu_round_trip(data: &[u8]) {
    use arbitrary::{Arbitrary as _, Unstructured};
    use ironrdp_core::{decode, encode_vec};
    use ironrdp_rdpemt::pdu::TunnelPdu;

    let mut unstructured = Unstructured::new(data);
    let Ok(pdu) = TunnelPdu::arbitrary(&mut unstructured) else {
        return;
    };

    // `TunnelPdu` decodes as a union but each variant owns its encoder, so the
    // size check happens per variant.
    let encoded = match &pdu {
        TunnelPdu::CreateRequest(inner) => encode_and_check(inner),
        TunnelPdu::CreateResponse(inner) => encode_and_check(inner),
        TunnelPdu::Data(inner) => encode_and_check(inner),
    };

    let Some(encoded) = encoded else {
        return;
    };

    match decode::<TunnelPdu>(&encoded) {
        Ok(reparsed) => {
            let re_encoded = match &reparsed {
                TunnelPdu::CreateRequest(inner) => encode_vec(inner),
                TunnelPdu::CreateResponse(inner) => encode_vec(inner),
                TunnelPdu::Data(inner) => encode_vec(inner),
            };
            match re_encoded {
                Ok(bytes) => assert_eq!(encoded, bytes, "re-encoding a decoded {pdu:?} produced different bytes"),
                Err(e) => panic!("re-encoding a value we just decoded failed: {e}"),
            }
        }
        Err(e) => panic!("re-decoding our own encoding failed: {e}"),
    }
}

/// Encodes a PDU and asserts `size()` matches what was written.
fn encode_and_check<T: ironrdp_core::Encode + core::fmt::Debug>(pdu: &T) -> Option<Vec<u8>> {
    let encoded = ironrdp_core::encode_vec(pdu).ok()?;
    assert_eq!(
        pdu.size(),
        encoded.len(),
        "size() disagrees with the encoded length for {pdu:?}"
    );
    Some(encoded)
}

/// Exercises the RDP-UDP2 packet-prefix transform.
///
/// The transform prepends a prefix byte and then swaps it with byte 7, in
/// place, padding short packets first. It is the one place in the transport
/// that mutates a caller's buffer, it runs on every datagram in both
/// directions, and its edge cases are all at lengths near the swap offset.
/// Feeding it arbitrary lengths is the cheapest way to find an index that
/// escapes the buffer.
pub fn rdpeudp_prefix_transform(data: &[u8]) {
    use ironrdp_rdpeudp::pdu::{decode_with_prefix, encode_with_prefix};

    // Receive direction: hostile bytes straight off the wire.
    let mut wire = data.to_vec();
    let _ = decode_with_prefix(&mut wire);

    // Send direction, then back again. Whatever the encoder produces the
    // decoder must accept, and must hand back the bytes that went in.
    let mut encoded = Vec::new();
    if let Ok(written) = encode_with_prefix(data, false, &mut encoded) {
        let mut round_trip = encoded[..written].to_vec();
        match decode_with_prefix(&mut round_trip) {
            Ok((_prefix, body)) => {
                if data.is_empty() {
                    // The prefix byte carries Short_Packet_Length, and 3.1.1.1.5.2
                    // reads zero as "not a short packet" rather than as a length.
                    // An empty packet is therefore the one input the transform
                    // cannot round-trip: it comes back as the seven padding bytes.
                    // RDP-UDP2 never emits one, because the v2 header alone is two
                    // bytes, so this is a property of the encoding rather than a
                    // defect. It is asserted so that a change which makes empty
                    // packets representable does not pass unnoticed.
                    assert_eq!(body, &[0u8; 7], "empty packet did not decode as padding");
                } else {
                    assert_eq!(
                        body,
                        data,
                        "prefix transform did not round-trip a {}-byte packet",
                        data.len()
                    );
                }
            }
            Err(e) => panic!("decoder rejected our own prefix encoding: {e}"),
        }
    }
}

/// Drives the RDP-UDP connection state machine through an arbitrary sequence
/// of wire events and clock advances.
///
/// This is the oracle a decode target cannot stand in for. The transport keeps
/// send and receive windows, a retransmission timer, congestion state and a
/// handshake state machine, and the interesting failures are orderings rather
/// than single malformed datagrams: an ACK for a sequence never sent, a
/// retransmit timer firing after close, a window that advances past its own
/// base. Each step feeds one datagram or moves the clock, and the loop asserts
/// the invariants that must survive whatever order the fuzzer picks.
pub fn rdpeudp_connection(data: &[u8]) {
    use arbitrary::{Arbitrary as _, Unstructured};
    use ironrdp_core::encode_vec;
    use ironrdp_rdpeudp::pdu::{V1Datagram, V2Packet};
    use ironrdp_rdpeudp::{ConnectionConfig, MonotonicInstant, RdpeudpConnection};

    #[derive(Debug, arbitrary::Arbitrary)]
    enum Step {
        /// Feed a structurally valid v1 datagram.
        HandleV1(V1Datagram),
        /// Feed a structurally valid v2 packet.
        HandleV2(V2Packet),
        /// Feed raw bytes, which is what a hostile peer actually sends.
        HandleRaw(Vec<u8>),
        /// Move the clock forward and let timers fire.
        Advance(u16),
        /// Queue application data.
        Send(Vec<u8>),
        /// Close locally.
        Close,
    }

    let mut unstructured = Unstructured::new(data);
    let Ok(steps) = Vec::<Step>::arbitrary(&mut unstructured) else {
        return;
    };

    let config = ConnectionConfig {
        // A version 3 SYN carries this, so `connect` requires it. The value
        // is arbitrary here: the fuzzer drives the peer side, not the
        // multitransport request the hash would really come from.
        cookie_hash: Some([0u8; 32]),
        ..ConnectionConfig::default()
    };
    let mut now = MonotonicInstant::from_millis(0);
    let Ok(mut conn) = RdpeudpConnection::connect(config, now) else {
        return;
    };

    let mut was_closed = false;

    for step in steps {
        let mut advanced = false;

        match step {
            Step::HandleV1(datagram) => {
                if let Ok(mut bytes) = encode_vec(&datagram) {
                    let _ = conn.handle_datagram(&mut bytes, now);
                }
            }
            Step::HandleV2(packet) => {
                if let Ok(mut bytes) = encode_vec(&packet) {
                    let _ = conn.handle_datagram(&mut bytes, now);
                }
            }
            Step::HandleRaw(mut bytes) => {
                let _ = conn.handle_datagram(&mut bytes, now);
            }
            Step::Advance(millis) => {
                now = now + core::time::Duration::from_millis(u64::from(millis));
                conn.handle_timeout(now);
                advanced = true;
            }
            Step::Send(payload) => {
                let _ = conn.send(payload);
            }
            Step::Close => conn.close(),
        }

        // Close is terminal. A connection that reopens itself would let a peer
        // resurrect a session the application already tore down.
        if was_closed {
            assert!(conn.is_closed(), "a closed connection became open again");
            assert!(!conn.is_established(), "a closed connection reported established");
        }
        was_closed |= conn.is_closed();

        // Draining must terminate. A generator that yields forever is a hang,
        // which libFuzzer reports only as a timeout with no useful stack, so
        // bound it here and fail loudly instead.
        let mut transmits = 0u32;
        while conn.poll_transmit(now).is_some() {
            transmits += 1;
            assert!(transmits < 10_000, "poll_transmit did not drain");
        }

        let mut events = 0u32;
        while conn.poll_event().is_some() {
            events += 1;
            assert!(events < 10_000, "poll_event did not drain");
        }

        // `handle_timeout` must retire or re-arm every timer it fired. If one
        // is still due at the same instant afterwards, the driver's
        // `sleep_until` returns immediately, calls `handle_timeout` again, and
        // the loop spins at full CPU for as long as the connection lives. The
        // check is only meaningful right after a clock advance, because
        // between advances a timer legitimately stays due until the driver
        // gets to it.
        if advanced {
            if let Some(deadline) = conn.poll_timeout() {
                assert!(
                    deadline > now,
                    "handle_timeout left a timer due at {deadline:?} with now={now:?}, which spins the driver"
                );
            }
        }
    }
}

/// Drives the multitransport tunnel state machine from both sides.
///
/// The tunnel validates a request ID and a security cookie before it will
/// carry data, so the failure this targets is a sequence that reaches the
/// established state without presenting the right cookie.
pub fn rdpemt_tunnel(data: &[u8]) {
    use arbitrary::{Arbitrary as _, Unstructured};
    use ironrdp_core::encode_vec;
    use ironrdp_rdpemt::pdu::{SECURITY_COOKIE_LEN, TunnelPdu};
    use ironrdp_rdpemt::{RdpemtTunnel, Side, TunnelConfig};

    #[derive(Debug, arbitrary::Arbitrary)]
    enum Step {
        HandlePdu(TunnelPdu),
        HandleRaw(Vec<u8>),
        SendData(Vec<u8>),
    }

    #[derive(Debug, arbitrary::Arbitrary)]
    struct Session {
        server_side: bool,
        request_id: u32,
        security_cookie: [u8; SECURITY_COOKIE_LEN],
        steps: Vec<Step>,
    }

    let mut unstructured = Unstructured::new(data);
    let Ok(session) = Session::arbitrary(&mut unstructured) else {
        return;
    };

    let config = TunnelConfig {
        request_id: session.request_id,
        security_cookie: session.security_cookie,
    };
    let mut tunnel = if session.server_side {
        RdpemtTunnel::server(config)
    } else {
        RdpemtTunnel::client(config)
    };

    let expected_side = if session.server_side {
        Side::Server
    } else {
        Side::Client
    };

    for step in session.steps {
        match step {
            Step::HandlePdu(pdu) => {
                // Encode through the owning variant: `TunnelPdu` decodes as a
                // union but does not itself implement `Encode`.
                let bytes = match &pdu {
                    TunnelPdu::CreateRequest(inner) => encode_vec(inner),
                    TunnelPdu::CreateResponse(inner) => encode_vec(inner),
                    TunnelPdu::Data(inner) => encode_vec(inner),
                };
                if let Ok(bytes) = bytes {
                    let _ = tunnel.handle_pdu(&bytes);
                }
            }
            Step::HandleRaw(bytes) => {
                let _ = tunnel.handle_pdu(&bytes);
            }
            Step::SendData(payload) => {
                let _ = tunnel.send_data(&payload);
            }
        }

        // A tunnel never changes which end of the connection it is.
        assert_eq!(tunnel.side(), expected_side, "tunnel changed sides");

        // Established and failed are mutually exclusive; a tunnel that is both
        // would carry data over a handshake that was rejected.
        assert!(
            !(tunnel.is_established() && tunnel.is_failed()),
            "tunnel reported established and failed at once"
        );

        let mut pdus = 0u32;
        while tunnel.poll_pdu().is_some() {
            pdus += 1;
            assert!(pdus < 10_000, "poll_pdu did not drain");
        }

        let mut events = 0u32;
        while tunnel.poll_event().is_some() {
            events += 1;
            assert!(events < 10_000, "poll_event did not drain");
        }
    }
}

/// Bounds the allocation an ACK vector can induce.
///
/// `V1AckVectorHeader` and `AckVectorPayload` are governed by different specs
/// with different caps. [MS-RDPEUDP] 2.2.2.7 caps the v1 header at 2048
/// elements, run-length coded, so a short datagram can describe a very long
/// run. [MS-RDPEUDP2] 2.2.1.2.6's `AckVectorPayload` is tighter:
/// `codedAckVecSize` is a 7-bit field, masked to `0x7F` on decode, so
/// `ACK_VECTOR_MAX_ENTRIES` (127) is the real ceiling there, not 2048. The
/// crash-only decode target only notices if either allocation aborts the
/// process; this one asserts each decoder honours its own cap before it
/// allocates, which is the difference between rejecting a hostile datagram
/// and being killed by the OOM killer.
pub fn rdpeudp_ack_vector(data: &[u8]) {
    use ironrdp_core::{Encode as _, decode, encode_vec};
    use ironrdp_rdpeudp::pdu::v2_ack::ACK_VECTOR_MAX_ENTRIES;
    use ironrdp_rdpeudp::pdu::{AckVectorPayload, V1AckVectorHeader};

    /// The v1 cap from MS-RDPEUDP 2.2.2.7. `V1AckVectorHeader` is the only
    /// structure this bound actually governs; `AckVectorPayload` uses
    /// `ACK_VECTOR_MAX_ENTRIES` instead, see the function doc comment.
    const MAX_V1_ACK_VECTOR: usize = 2048;

    if let Ok(payload) = decode::<AckVectorPayload>(data) {
        assert!(
            payload.entries.len() <= ACK_VECTOR_MAX_ENTRIES,
            "decoded {} ACK vector entries, above the {ACK_VECTOR_MAX_ENTRIES}-entry cap",
            payload.entries.len()
        );

        // Re-encoding must not disagree about the length either: an encoder
        // that writes more than `size()` promised would overrun a caller's
        // buffer rather than return an error.
        if let Ok(encoded) = encode_vec(&payload) {
            assert_eq!(payload.size(), encoded.len(), "ACK vector size() disagrees with encode");
        }
    }

    if let Ok(header) = decode::<V1AckVectorHeader>(data) {
        assert!(
            header.elements.len() <= MAX_V1_ACK_VECTOR,
            "decoded {} v1 ACK vector elements, above the 2048 cap",
            header.elements.len()
        );
    }
}

/// ZGFX decompression oracle.
///
/// ZGFX is the egfx-specific compression scheme defined in MS-RDPEGFX
/// 2.2.5.1-2.2.5.3 (segmentation and encoding) and 3.1.9.1 (RDP 8.0 Bulk
/// Compression processing rules). It is distinct from `ironrdp-bulk`'s
/// MPPC/NCRUSH/XCRUSH (those carry connection-level RDP traffic; ZGFX wraps
/// individual EGFX PDU payloads).
/// The implementation lives in `ironrdp-graphics::zgfx` and uses a
/// 2.5 MB sliding-window history.
///
/// Each iteration constructs a fresh `Decompressor` so history state does not
/// leak between fuzz inputs. The output buffer is local to the iteration.
///
/// What this catches: panics in the segmented-PDU parser, OOB reads/writes
/// in the LZ77-style match-copy path, decompression bombs that exhaust the
/// output buffer, corrupted-history paths that desynchronize the circular
/// buffer's read/write cursors.
///
/// What this does NOT catch: cross-iteration state (history reuse across
/// PDU sequences). The multi-frame oracle target under #1316 covers that
/// shape when it lands. A memory-budget overlay is also planned once the
/// shape question on #1120 is settled; the panic/sanitizer-only oracle here
/// is the interim baseline.
pub fn egfx_zgfx_decompress(data: &[u8]) {
    use ironrdp_graphics::zgfx::Decompressor;

    let mut decompressor = Decompressor::new();
    let mut output = Vec::new();
    let _ = decompressor.decompress(data, &mut output);
}
