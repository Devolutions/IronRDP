use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_egfx::pdu::{
    Avc420Region, CapabilitiesAdvertisePdu, CapabilitiesV8Flags, CapabilitiesV10Flags, CapabilitiesV81Flags,
    CapabilitySet, Codec1Type, FrameAcknowledgePdu, GfxPdu, PixelFormat, QueueDepth,
};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer, QoeMetrics, Surface};
use ironrdp_graphics::zgfx::Decompressor;

// ============================================================================
// Test Handler
// ============================================================================

struct TestHandler {
    ready_called: bool,
    negotiated: Option<CapabilitySet>,
    frame_acks: Vec<(u32, u32, u32)>,
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

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32, total_frames_decoded: u32) {
        self.frame_acks.push((frame_id, queue_depth, total_frames_decoded));
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
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

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8_1 {
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

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V10 {
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

// ============================================================================
// Planar Frame Tests
// ============================================================================

#[test]
fn test_planar_frame_round_trip() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // V10 client with AVC disabled: Planar remains usable.
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V10 {
        flags: CapabilitiesV10Flags::AVC_DISABLED,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    assert!(server.is_ready());
    assert!(!server.supports_avc420());
    assert!(!server.supports_avc444());

    let surface_id = server
        .create_surface_with_format(1280, 720, PixelFormat::ARgb)
        .expect("surface creation failed");
    server.drain_output();

    let planar_data = [0x20, 0x00, 0x01, 0x02];
    let frame_id = server
        .send_planar_frame(surface_id, &planar_data, 1280, 720, 42)
        .expect("Planar frame should be queued");

    let output = server.drain_output();
    assert_eq!(output.len(), 3);

    let mut decompressor = Decompressor::new();
    let mut pdus = Vec::with_capacity(output.len());
    for message in output {
        let encoded = encode_vec(message.as_ref()).expect("encode should succeed");
        let mut decoded = Vec::new();
        decompressor
            .decompress(&encoded, &mut decoded)
            .expect("ZGFX decode should succeed");
        let mut cursor = ReadCursor::new(&decoded);
        pdus.push(GfxPdu::decode(&mut cursor).expect("PDU decode should succeed"));
    }

    match pdus.as_slice() {
        [
            GfxPdu::StartFrame(start),
            GfxPdu::WireToSurface1(wire),
            GfxPdu::EndFrame(end),
        ] => {
            assert_eq!(start.frame_id, frame_id);
            assert_eq!(wire.surface_id, surface_id);
            assert_eq!(wire.codec_id, Codec1Type::Planar);
            assert_eq!(wire.pixel_format, PixelFormat::ARgb);
            assert_eq!(wire.destination_rectangle.left, 0);
            assert_eq!(wire.destination_rectangle.top, 0);
            assert_eq!(wire.destination_rectangle.right, 1280);
            assert_eq!(wire.destination_rectangle.bottom, 720);
            assert_eq!(wire.bitmap_data, planar_data);
            assert_eq!(end.frame_id, frame_id);
        }
        pdus => panic!("expected StartFrame, WireToSurface1, EndFrame; got {pdus:?}"),
    }
}

#[test]
fn test_planar_frame_rejects_oversized_destination() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V10 {
        flags: CapabilitiesV10Flags::AVC_DISABLED,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    let surface_id = server.create_surface(1280, 720).expect("surface creation failed");
    server.drain_output();

    assert_eq!(server.frames_in_flight(), 0);
    assert!(server.send_planar_frame(surface_id, &[0x20], 1281, 720, 42).is_none());
    assert!(!server.has_pending_output());
    assert_eq!(server.frames_in_flight(), 0);
}

#[test]
fn test_surface_lifecycle() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    // Negotiate capabilities first
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8_1 {
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8_1 {
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8_1 {
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V10 {
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V10 {
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
    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
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

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
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

#[test]
fn test_frames_sent_while_acknowledgement_is_suspended_do_not_hold_backpressure() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);
    server.set_max_frames_in_flight(1);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    let surface_id = server.create_surface(64, 64).unwrap();
    server.drain_output();

    let pixel_data = vec![0xFFu8; 64 * 64 * 4];

    let frame1 = server
        .send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 0)
        .expect("first frame");

    // [MS-RDPEGFX] 2.2.2.13: a queue depth of SUSPEND_FRAME_ACKNOWLEDGEMENT
    // means the client stops sending Frame Acknowledge until it says otherwise.
    let suspend = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
        queue_depth: QueueDepth::Suspend,
        frame_id: frame1,
        total_frames_decoded: 1,
    });
    let _output = server.process(0, &encode_pdu(&suspend)).expect("suspend ack");

    // The pause outlasts the in-flight window by a wide margin, and none of
    // these frames will be acknowledged.
    let mut last_frame = frame1;
    for index in 1..=5 {
        last_frame = server
            .send_uncompressed_frame(surface_id, &pixel_data, 64, 64, index * 16)
            .expect("a suspended client must not throttle the encoder");
    }
    server.drain_output();

    // The client comes back and acknowledges normally.
    let resume = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
        queue_depth: QueueDepth::AvailableBytes(8),
        frame_id: last_frame,
        total_frames_decoded: 6,
    });
    let _output = server.process(0, &encode_pdu(&resume)).expect("resume ack");

    assert!(
        server
            .send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 96)
            .is_some(),
        "frames counted during the pause hold backpressure on for good: with no \
         frame going out there is no End Frame left for the client to acknowledge"
    );
}

#[test]
fn test_suspension_does_not_shrink_the_window_for_the_rest_of_the_connection() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);
    server.set_max_frames_in_flight(3);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    let surface_id = server.create_surface(64, 64).unwrap();
    server.drain_output();
    let pixel_data = vec![0xFFu8; 64 * 64 * 4];

    // Fill the window, then suspend. The frames already in flight will not be
    // acknowledged either -- the suspension covers them as much as the ones
    // that follow.
    let mut first = None;
    for index in 0..3 {
        let id = server
            .send_uncompressed_frame(surface_id, &pixel_data, 64, 64, index * 16)
            .expect("window not yet full");
        first.get_or_insert(id);
    }

    let suspend = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
        queue_depth: QueueDepth::Suspend,
        frame_id: first.expect("a frame went out"),
        total_frames_decoded: 1,
    });
    let _output = server.process(0, &encode_pdu(&suspend)).expect("suspend ack");

    let resume = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
        queue_depth: QueueDepth::AvailableBytes(8),
        frame_id: u32::MAX,
        total_frames_decoded: 2,
    });
    let _output = server.process(0, &encode_pdu(&resume)).expect("resume ack");
    server.drain_output();

    // The whole window has to be available again. Frames stranded by the
    // suspension are never acknowledged, so leaving them tracked shrinks the
    // window permanently -- here to one frame instead of three.
    for index in 0..3 {
        assert!(
            server
                .send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 100 + index * 16)
                .is_some(),
            "frame {index} after resume was held back by a frame stranded in the pause"
        );
    }
}

#[test]
fn test_a_suspending_acknowledgement_still_acknowledges_its_frame() {
    let handler = Box::new(TestHandler::new());
    let mut server = GraphicsPipelineServer::new(handler);

    let client_caps_pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::SMALL_CACHE,
    }]));
    let payload = encode_pdu(&client_caps_pdu);
    let _output = server.process(0, &payload).expect("process failed");

    let surface_id = server.create_surface(64, 64).unwrap();
    server.drain_output();
    let pixel_data = vec![0xFFu8; 64 * 64 * 4];

    let frame_id = server
        .send_uncompressed_frame(surface_id, &pixel_data, 64, 64, 0)
        .expect("first frame");

    // The PDU that suspends also acknowledges a frame, and that frame carries
    // the round-trip sample the QoE report is built from. Dropping it because
    // the same PDU happens to suspend loses the measurement.
    let suspend = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
        queue_depth: QueueDepth::Suspend,
        frame_id,
        total_frames_decoded: 1,
    });
    let _output = server.process(0, &encode_pdu(&suspend)).expect("suspend ack");

    assert!(
        server.qoe_snapshot().is_some(),
        "the suspending acknowledgement was not counted, so no round-trip sample exists"
    );
}
