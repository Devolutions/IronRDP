use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineClient, GraphicsPipelineHandler, Surface};
use ironrdp_egfx::decode::{DecodedFrame, DecoderResult, H264Decoder};
use ironrdp_egfx::pdu::{
    CapabilitiesAdvertisePdu, CapabilitiesConfirmPdu, CapabilitiesV8Flags, CapabilitySet, Codec1Type, CreateSurfacePdu,
    DeleteSurfacePdu, EndFramePdu, GfxPdu, PixelFormat, ResetGraphicsPdu, StartFramePdu, Timestamp, WireToSurface1Pdu,
};
use ironrdp_graphics::zgfx::wrap_uncompressed;
use ironrdp_pdu::geometry::InclusiveRectangle;

// ============================================================================
// Test Handler
// ============================================================================

struct TestHandler {
    caps_confirmed: bool,
    bitmaps_received: Vec<(u16, Codec1Type)>,
    frames_completed: Vec<u32>,
    reset_count: u32,
}

impl TestHandler {
    fn new() -> Self {
        Self {
            caps_confirmed: false,
            bitmaps_received: Vec::new(),
            frames_completed: Vec::new(),
            reset_count: 0,
        }
    }
}

impl GraphicsPipelineHandler for TestHandler {
    fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {
        self.caps_confirmed = true;
    }

    fn on_reset_graphics(&mut self, _width: u32, _height: u32) {
        self.reset_count += 1;
    }

    fn on_surface_created(&mut self, _surface: &Surface) {}
    fn on_surface_deleted(&mut self, _surface_id: u16) {}
    fn on_surface_mapped(&mut self, _surface_id: u16, _x: u32, _y: u32) {}

    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        self.bitmaps_received.push((update.surface_id, update.codec_id));
    }

    fn on_frame_complete(&mut self, frame_id: u32) {
        self.frames_completed.push(frame_id);
    }

    fn on_close(&mut self) {}
    fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {}
}

// ============================================================================
// Mock H.264 Decoder
// ============================================================================

struct MockH264Decoder;

impl H264Decoder for MockH264Decoder {
    fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
        // Return a 16x16 solid red frame (macroblock-aligned minimum)
        let mut data = vec![0u8; 16 * 16 * 4];
        for pixel in data.chunks_exact_mut(4) {
            pixel[0] = 255; // R
            pixel[3] = 255; // A
        }
        Ok(DecodedFrame {
            data,
            width: 16,
            height: 16,
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn encode_pdu<T: Encode>(pdu: &T) -> Vec<u8> {
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).expect("encode failed");
    buf
}

/// Encode a GfxPdu and wrap in a ZGFX uncompressed segment descriptor.
/// The client's process() expects ZGFX-segmented input (it runs decompression first).
fn encode_for_process(pdu: &GfxPdu) -> Vec<u8> {
    let raw = encode_pdu(pdu);
    wrap_uncompressed(&raw)
}

fn decode_caps_from_message(msg: &ironrdp_dvc::DvcMessage) -> CapabilitiesAdvertisePdu {
    let encoded = encode_vec(msg.as_ref()).expect("encode should succeed");
    let mut cursor = ReadCursor::new(&encoded);
    let pdu = GfxPdu::decode(&mut cursor).expect("decode should succeed");
    match pdu {
        GfxPdu::CapabilitiesAdvertise(caps) => caps,
        other => panic!("expected CapabilitiesAdvertise, got {other:?}"),
    }
}

/// Create a client, send CapabilitiesConfirm V8 through process(), and create a surface.
fn setup_active_client_with_surface(
    decoder: Option<Box<dyn H264Decoder>>,
    surface_id: u16,
    width: u16,
    height: u16,
) -> GraphicsPipelineClient {
    let handler = TestHandler::new();
    let mut client = GraphicsPipelineClient::new(Box::new(handler), decoder);

    // Activate via CapabilitiesConfirm
    let confirm = GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::empty(),
    }));
    client
        .process(0, &encode_for_process(&confirm))
        .expect("confirm should succeed");

    // Create surface
    let create = GfxPdu::CreateSurface(CreateSurfacePdu {
        surface_id,
        width,
        height,
        pixel_format: PixelFormat::XRgb,
    });
    client
        .process(0, &encode_for_process(&create))
        .expect("create surface should succeed");

    client
}

// ============================================================================
// Tests: Capability Advertisement
// ============================================================================

#[test]
fn client_sends_capabilities_on_start() {
    let handler = TestHandler::new();
    let mut client = GraphicsPipelineClient::new(Box::new(handler), None);
    let messages = client.start(0).expect("start should succeed");
    assert_eq!(messages.len(), 1);
}

#[test]
fn client_filters_avc_caps_without_decoder() {
    let handler = TestHandler::new();
    let mut client = GraphicsPipelineClient::new(Box::new(handler), None);
    let messages = client.start(0).expect("start should succeed");
    assert_eq!(messages.len(), 1);

    let caps_pdu = decode_caps_from_message(&messages[0]);
    assert_eq!(
        caps_pdu.0.len(),
        1,
        "expected exactly one capability set when no decoder is present"
    );
    assert!(
        matches!(caps_pdu.0[0], CapabilitySet::V8 { .. }),
        "expected only V8 capability set without decoder, got {:?}",
        caps_pdu.0[0]
    );
}

#[test]
fn client_keeps_avc_caps_with_decoder() {
    let handler = TestHandler::new();
    let mut client = GraphicsPipelineClient::new(Box::new(handler), Some(Box::new(MockH264Decoder)));
    let messages = client.start(0).expect("start should succeed");
    assert_eq!(messages.len(), 1);

    let caps_pdu = decode_caps_from_message(&messages[0]);
    assert_eq!(
        caps_pdu.0.len(),
        3,
        "expected all three capability sets with decoder present"
    );
    assert!(matches!(caps_pdu.0[0], CapabilitySet::V10_7 { .. }));
    assert!(matches!(caps_pdu.0[1], CapabilitySet::V8_1 { .. }));
    assert!(matches!(caps_pdu.0[2], CapabilitySet::V8 { .. }));
}

// ============================================================================
// Tests: Frame Flow (via process() with encoded PDUs)
// ============================================================================

#[test]
fn client_sends_frame_ack_on_end_frame() {
    let mut client = setup_active_client_with_surface(None, 1, 4, 4);

    let end = GfxPdu::EndFrame(EndFramePdu { frame_id: 42 });
    let responses = client
        .process(0, &encode_for_process(&end))
        .expect("end frame should succeed");

    assert_eq!(responses.len(), 1, "should produce exactly one FrameAcknowledge");
    assert_eq!(client.total_frames_decoded(), 1);
}

#[test]
fn client_handles_uncompressed_via_process() {
    let mut client = setup_active_client_with_surface(None, 1, 4, 4);

    let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Uncompressed,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 0,
            top: 0,
            right: 3,
            bottom: 3,
        },
        bitmap_data: vec![0u8; 4 * 4 * 4],
    });
    client
        .process(0, &encode_for_process(&pdu))
        .expect("uncompressed should succeed");
}

#[test]
fn client_dispatches_avc420_via_process() {
    let mut client = setup_active_client_with_surface(Some(Box::new(MockH264Decoder)), 1, 16, 16);

    // Build minimal AVC420 bitmap stream
    let mut bitmap_data = Vec::new();
    bitmap_data.extend_from_slice(&1u32.to_le_bytes()); // nRect = 1
    bitmap_data.extend_from_slice(&0u16.to_le_bytes()); // left
    bitmap_data.extend_from_slice(&0u16.to_le_bytes()); // top
    bitmap_data.extend_from_slice(&15u16.to_le_bytes()); // right
    bitmap_data.extend_from_slice(&15u16.to_le_bytes()); // bottom
    bitmap_data.push(22); // qp
    bitmap_data.push(100); // quality
    bitmap_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]); // fake H.264

    let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Avc420,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 0,
            top: 0,
            right: 15,
            bottom: 15,
        },
        bitmap_data,
    });
    client
        .process(0, &encode_for_process(&pdu))
        .expect("AVC420 should succeed");
}

#[test]
fn client_skips_avc420_without_decoder() {
    let mut client = setup_active_client_with_surface(None, 1, 16, 16);

    let mut bitmap_data = Vec::new();
    bitmap_data.extend_from_slice(&1u32.to_le_bytes());
    bitmap_data.extend_from_slice(&0u16.to_le_bytes());
    bitmap_data.extend_from_slice(&0u16.to_le_bytes());
    bitmap_data.extend_from_slice(&15u16.to_le_bytes());
    bitmap_data.extend_from_slice(&15u16.to_le_bytes());
    bitmap_data.push(22);
    bitmap_data.push(100);
    bitmap_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]);

    let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Avc420,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 0,
            top: 0,
            right: 15,
            bottom: 15,
        },
        bitmap_data,
    });
    client
        .process(0, &encode_for_process(&pdu))
        .expect("should succeed without decoder");
}

#[test]
fn client_frame_ordering_via_process() {
    let mut client = setup_active_client_with_surface(None, 1, 4, 4);

    // StartFrame
    let start = GfxPdu::StartFrame(StartFramePdu {
        timestamp: Timestamp {
            milliseconds: 0,
            seconds: 0,
            minutes: 0,
            hours: 0,
        },
        frame_id: 1,
    });
    client.process(0, &encode_for_process(&start)).expect("start frame");

    // WireToSurface1 (uncompressed)
    let wire = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Uncompressed,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 0,
            top: 0,
            right: 3,
            bottom: 3,
        },
        bitmap_data: vec![0u8; 4 * 4 * 4],
    });
    client.process(0, &encode_for_process(&wire)).expect("wire to surface");

    // EndFrame should produce FrameAcknowledge
    let end = GfxPdu::EndFrame(EndFramePdu { frame_id: 1 });
    let responses = client.process(0, &encode_for_process(&end)).expect("end frame");

    assert_eq!(responses.len(), 1);
    assert_eq!(client.total_frames_decoded(), 1);
}

// ============================================================================
// Tests: Surface Lifecycle (via process())
// ============================================================================

#[test]
fn client_creates_and_queries_surface() {
    let client = setup_active_client_with_surface(None, 7, 1920, 1080);

    let surface = client.get_surface(7);
    assert!(surface.is_some(), "surface 7 should exist after creation");
    assert_eq!(surface.unwrap().width, 1920);
    assert_eq!(surface.unwrap().height, 1080);

    // Nonexistent surface
    assert!(client.get_surface(99).is_none());
}

#[test]
fn client_deletes_surface_via_process() {
    let mut client = setup_active_client_with_surface(None, 5, 100, 100);
    assert!(client.get_surface(5).is_some());

    let delete = GfxPdu::DeleteSurface(DeleteSurfacePdu { surface_id: 5 });
    client
        .process(0, &encode_for_process(&delete))
        .expect("delete should succeed");

    assert!(client.get_surface(5).is_none(), "surface should be gone after delete");
}

#[test]
fn client_resets_surfaces_via_process() {
    let mut client = setup_active_client_with_surface(None, 1, 100, 100);

    // Create a second surface
    let create2 = GfxPdu::CreateSurface(CreateSurfacePdu {
        surface_id: 2,
        width: 200,
        height: 200,
        pixel_format: PixelFormat::XRgb,
    });
    client.process(0, &encode_for_process(&create2)).expect("create 2");
    assert!(client.get_surface(1).is_some());
    assert!(client.get_surface(2).is_some());

    // ResetGraphics should clear all surfaces
    let reset = GfxPdu::ResetGraphics(ResetGraphicsPdu {
        width: 1920,
        height: 1080,
        monitors: vec![],
    });
    client.process(0, &encode_for_process(&reset)).expect("reset");

    assert!(
        client.get_surface(1).is_none(),
        "surface 1 should be cleared after reset"
    );
    assert!(
        client.get_surface(2).is_none(),
        "surface 2 should be cleared after reset"
    );
}

// ============================================================================
// Tests: Error Handling
// ============================================================================

#[test]
fn client_rejects_wire_to_unknown_surface() {
    let mut client = setup_active_client_with_surface(None, 1, 4, 4);

    let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 99, // does not exist
        codec_id: Codec1Type::Uncompressed,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 0,
            top: 0,
            right: 3,
            bottom: 3,
        },
        bitmap_data: vec![0u8; 4 * 4 * 4],
    });
    let result = client.process(0, &encode_for_process(&pdu));
    assert!(result.is_err(), "should reject write to nonexistent surface");
}

#[test]
fn client_rejects_invalid_rectangle_ordering() {
    let mut client = setup_active_client_with_surface(None, 1, 100, 100);

    // left > right
    let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Uncompressed,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 50,
            top: 0,
            right: 10,
            bottom: 10,
        },
        bitmap_data: vec![0u8; 4],
    });
    let result = client.process(0, &encode_for_process(&pdu));
    assert!(result.is_err(), "left > right should be rejected");
}

#[test]
fn client_tolerates_out_of_bounds_rectangle() {
    let mut client = setup_active_client_with_surface(None, 1, 100, 100);

    // Rectangle exceeds surface dimensions. The client logs a warning
    // but continues processing (defensive: avoid disconnecting for a
    // recoverable server-side error).
    let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Uncompressed,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: InclusiveRectangle {
            left: 0,
            top: 0,
            right: 200, // exceeds surface width of 100
            bottom: 50,
        },
        bitmap_data: vec![0u8; 201 * 51 * 4],
    });
    let result = client.process(0, &encode_for_process(&pdu));
    assert!(
        result.is_ok(),
        "out-of-bounds rectangle should be tolerated (warn, not error)"
    );
}

// ============================================================================
// Tests: Multiple Frames
// ============================================================================

#[test]
fn client_tracks_frame_count_across_multiple_frames() {
    let mut client = setup_active_client_with_surface(None, 1, 4, 4);

    for frame_id in 1..=5 {
        let end = GfxPdu::EndFrame(EndFramePdu { frame_id });
        client.process(0, &encode_for_process(&end)).expect("end frame");
    }

    assert_eq!(client.total_frames_decoded(), 5);
}

// ============================================================================
// Tests: Close
// ============================================================================

#[test]
fn client_close_transitions_to_inactive() {
    let mut client = setup_active_client_with_surface(None, 1, 4, 4);
    assert!(client.is_active());

    client.close(0);
    assert!(!client.is_active(), "client should not be active after close");
}
