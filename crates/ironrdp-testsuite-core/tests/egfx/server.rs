use ironrdp_core::{Encode, WriteCursor};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_egfx::pdu::{
    Avc420Region, CapabilitiesAdvertisePdu, CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV8Flags,
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
