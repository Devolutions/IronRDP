use ironrdp_core::{Encode, WriteCursor};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_egfx::pdu::{
    Avc420Region, CapabilitiesAdvertisePdu, CapabilitiesV8Flags, CapabilitiesV10Flags, CapabilitiesV81Flags,
    CapabilitySet, GfxPdu,
};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer, QoeMetrics, Surface};

// ============================================================================
// Test Handler
// ============================================================================

struct TestHandler {
    ready_called: bool,
    negotiated: Option<CapabilitySet>,
    frame_acks: Vec<(u32, u32)>,
    surfaces_created: Vec<u16>,
    surfaces_deleted: Vec<u16>,
}

impl TestHandler {
    fn new() -> Self {
        Self {
            ready_called: false,
            negotiated: None,
            frame_acks: Vec::new(),
            surfaces_created: Vec::new(),
            surfaces_deleted: Vec::new(),
        }
    }
}

impl GraphicsPipelineHandler for TestHandler {
    fn capabilities_advertise(&mut self, _pdu: &CapabilitiesAdvertisePdu) {}

    fn on_ready(&mut self, negotiated: &CapabilitySet) {
        self.ready_called = true;
        self.negotiated = Some(negotiated.clone());
    }

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32) {
        self.frame_acks.push((frame_id, queue_depth));
    }

    fn on_qoe_metrics(&mut self, _metrics: QoeMetrics) {}

    fn on_surface_created(&mut self, surface: &Surface) {
        self.surfaces_created.push(surface.id);
    }

    fn on_surface_deleted(&mut self, surface_id: u16) {
        self.surfaces_deleted.push(surface_id);
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Encode a PDU to bytes for sending to server's process() method
fn encode_pdu<T: Encode>(pdu: &T) -> Vec<u8> {
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).expect("encode failed");
    buf
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_server_creation() {
    let handler = Box::new(TestHandler::new());
    let server = GraphicsPipelineServer::new(handler);

    assert!(!server.is_ready());
    assert_eq!(server.frames_in_flight(), 0);
    assert!(!server.supports_avc420());
    assert!(!server.supports_avc444());
}

#[test]
fn test_capability_negotiation_v8() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Simulate client sending CapabilitiesAdvertise
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));

    let payload = encode_pdu(&client_caps_pdu);
    let output = server.process(0, &payload).expect("process failed");

    // Server should be ready now
    assert!(server.is_ready());

    // Should output CapabilitiesConfirm
    assert_eq!(output.len(), 1);
}

#[test]
fn test_capability_negotiation_v81_avc420() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8_1 {
        flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
    }]));

    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    assert!(server.is_ready());
    assert!(server.supports_avc420());
    assert!(!server.supports_avc444());
}

#[test]
fn test_capability_negotiation_v10_avc444() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V10 {
        flags: CapabilitiesV10Flags::SMALL_CACHE,
    }]));

    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    assert!(server.is_ready());
    assert!(server.supports_avc420());
    assert!(server.supports_avc444());
}

#[test]
fn test_server_not_ready_before_capabilities() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Server should not accept frames before capability negotiation
    let h264_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
    let regions = vec![Avc420Region::full_frame(1920, 1080, 22)];

    let result = server.send_avc420_frame(0, &h264_data, &regions, 0);
    assert!(result.is_none());
}

#[test]
fn test_surface_lifecycle() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Negotiate capabilities first
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8_1 {
        flags: CapabilitiesV81Flags::AVC420_ENABLED,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    assert!(server.is_ready());

    // Create surface
    let surface_id = server.create_surface(1920, 1080);
    assert!(surface_id.is_some());
    let sid = surface_id.unwrap();

    // Verify surface exists
    let surface = server.get_surface(sid);
    assert!(surface.is_some());
    assert_eq!(surface.unwrap().width, 1920);
    assert_eq!(surface.unwrap().height, 1080);

    // Map to output
    assert!(server.map_surface_to_output(sid, 0, 0));

    // Delete surface
    assert!(server.delete_surface(sid));
    assert!(server.get_surface(sid).is_none());

    // Drain output: ResetGraphics (auto-sent before first surface), CreateSurface,
    // MapSurfaceToOutput, DeleteSurface
    let output = server.drain_output();
    assert_eq!(output.len(), 4);
}

#[test]
fn test_resize() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Negotiate capabilities
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    // Create a surface
    let surface_id = server.create_surface(1920, 1080).unwrap();

    // Resize
    server.resize(2560, 1440);

    // Surface should be deleted
    assert!(server.get_surface(surface_id).is_none());

    // Output dimensions should be updated
    assert_eq!(server.output_dimensions(), (2560, 1440));

    // Should have output PDUs
    assert!(server.has_pending_output());
}

#[test]
fn test_frame_flow_control() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);
    server.set_max_frames_in_flight(2);

    // Negotiate capabilities with AVC420
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8_1 {
        flags: CapabilitiesV81Flags::AVC420_ENABLED,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    // Create surface
    let surface_id = server.create_surface(1920, 1080).unwrap();
    server.drain_output(); // Clear setup PDUs

    let h264_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
    let regions = vec![Avc420Region::full_frame(1920, 1080, 22)];

    // First two frames should succeed
    let frame1 = server.send_avc420_frame(surface_id, &h264_data, &regions, 0);
    assert!(frame1.is_some());

    let frame2 = server.send_avc420_frame(surface_id, &h264_data, &regions, 16);
    assert!(frame2.is_some());

    // Check backpressure is active
    assert!(server.should_backpressure());
    assert_eq!(server.frames_in_flight(), 2);

    // Third frame should fail due to backpressure
    let frame3 = server.send_avc420_frame(surface_id, &h264_data, &regions, 33);
    assert!(frame3.is_none());
}

// ============================================================================
// QoE Statistics Tests
// ============================================================================

#[test]
fn test_qoe_snapshot_none_before_data() {
    let handler = Box::new(TestHandler::new());
    let server = GraphicsPipelineServer::new(handler);

    // No QoE reports yet.
    assert!(server.qoe_snapshot().is_none());
}

#[test]
fn test_qoe_snapshot_after_frame_ack() {
    use ironrdp_egfx::pdu::{FrameAcknowledgePdu, QueueDepth};

    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Negotiate capabilities.
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8_1 {
        flags: CapabilitiesV81Flags::AVC420_ENABLED,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    // Create surface and send a frame.
    let surface_id = server.create_surface(1920, 1080).unwrap();
    server.drain_output();

    let h264_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
    let regions = vec![Avc420Region::full_frame(1920, 1080, 22)];
    let frame_id = server.send_avc420_frame(surface_id, &h264_data, &regions, 0);
    assert!(frame_id.is_some());

    // Simulate client frame acknowledgment.
    let ack_pdu = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
        frame_id: frame_id.unwrap(),
        queue_depth: QueueDepth::AvailableBytes(1),
        total_frames_decoded: 1,
    });
    let ack_payload = encode_pdu(&ack_pdu);
    let _output = server.process(0, &ack_payload).expect("process failed");

    // QoE snapshot should now have RTT data (no QoE reports, but RTT from ack).
    let snapshot = server.qoe_snapshot();
    assert!(snapshot.is_some());

    let snap = snapshot.unwrap();
    assert_eq!(snap.total_rtt_samples, 1);
    // RTT should be some small value (frame was just sent).
    assert!(snap.avg_rtt_ms < 1000.0);
    // No QoE reports yet.
    assert_eq!(snap.total_qoe_reports, 0);
}

#[test]
fn test_qoe_snapshot_after_qoe_report() {
    use ironrdp_egfx::pdu::QoeFrameAcknowledgePdu;

    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Negotiate capabilities (V10 for QoE support).
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V10 {
        flags: CapabilitiesV10Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    // Simulate QoE report.
    let qoe_pdu = GfxPdu::QoeFrameAcknowledge(QoeFrameAcknowledgePdu {
        frame_id: 0,
        timestamp: 12345,
        time_diff_se: 100,
        time_diff_dr: 4500,
    });
    let qoe_payload = encode_pdu(&qoe_pdu);
    let _output = server.process(0, &qoe_payload).expect("process failed");

    let snapshot = server.qoe_snapshot();
    assert!(snapshot.is_some());

    let snap = snapshot.unwrap();
    assert_eq!(snap.total_qoe_reports, 1);
    assert_eq!(snap.latest_decode_render_us, 4500);
    assert!((snap.avg_decode_render_us - 4500.0).abs() < 0.1);
    assert_eq!(snap.min_decode_render_us, 4500);
    assert_eq!(snap.max_decode_render_us, 4500);
}

#[test]
fn test_qoe_reset() {
    use ironrdp_egfx::pdu::QoeFrameAcknowledgePdu;

    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Negotiate.
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V10 {
        flags: CapabilitiesV10Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    // Add a QoE report.
    let qoe_pdu = GfxPdu::QoeFrameAcknowledge(QoeFrameAcknowledgePdu {
        frame_id: 0,
        timestamp: 1000,
        time_diff_se: 50,
        time_diff_dr: 3000,
    });
    let qoe_payload = encode_pdu(&qoe_pdu);
    let _output = server.process(0, &qoe_payload).expect("process failed");
    assert!(server.qoe_snapshot().is_some());

    // Reset clears all statistics.
    server.reset_qoe();
    assert!(server.qoe_snapshot().is_none());
}

// ============================================================================
// Uncompressed Frame Tests
// ============================================================================

#[test]
fn test_send_uncompressed_frame_queues_correctly() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // V8 client: EGFX but no H.264
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    let surface_id = server.create_surface(64, 64).unwrap();
    server.map_surface_to_output(surface_id, 0, 0);
    server.drain_output(); // Clear setup PDUs

    // 64x64 XRGB = 16384 bytes
    let pixel_data = vec![0xFFu8; 64 * 64 * 4];
    let frame_id = server.send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 0);
    assert!(frame_id.is_some());

    // Output: StartFrame + WireToSurface1 + EndFrame
    let output = server.drain_output();
    assert_eq!(output.len(), 3);
}

#[test]
fn test_send_uncompressed_frame_backpressure() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);
    server.set_max_frames_in_flight(1);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    let surface_id = server.create_surface(64, 64).unwrap();
    server.drain_output();

    let pixel_data = vec![0xFFu8; 64 * 64 * 4];

    // First frame succeeds
    let frame1 = server.send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 0);
    assert!(frame1.is_some());

    // Second frame blocked by backpressure
    let frame2 = server.send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 16);
    assert!(frame2.is_none());
}
