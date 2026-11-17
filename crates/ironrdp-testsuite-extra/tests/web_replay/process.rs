use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use ironrdp_web_replay::{PduBuffer, PduSource, PointerState, ProcessResult, ReplayProcessor, ReplayProcessorConfig};

/// Fast-Path client input PDU fixture (44 bytes, 6 mouse events).
// The fixture's capture provenance is unknown; expected mouse positions are decoded from its event fields.
// oracle: spec MS-RDPBCGR §Section_2.2.8.1.2 and §Section_2.2.8.1.2.2.3.
const FASTPATH_CLIENT_INPUT: [u8; 44] = [
    0x18, 0x2c, 0x20, 0x0, 0x90, 0x1a, 0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1b, 0x0, 0x26, 0x4, 0x20, 0x0, 0x10, 0x1b,
    0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1a, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19,
    0x0, 0x28, 0x4,
];

// oracle: spec MS-RDPBCGR §Section_2.2.9.1.2 and §Section_2.2.9.1.2.1.6.
const FASTPATH_SERVER_POINTER_DEFAULT: [u8; 5] = [0x00, 0x05, 0x06, 0x00, 0x00];

fn test_processor() -> ReplayProcessor {
    ReplayProcessor::new(&ReplayProcessorConfig::default())
}

fn test_image() -> DecodedImage {
    DecodedImage::new(PixelFormat::RgbA32, 64, 64)
}

// Tests for process_pdu.

#[test]
fn process_pdu_returns_error_for_empty_input() {
    // oracle: mutant accept an empty PDU as a no-op.
    let mut proc = test_processor();
    let mut image = test_image();
    let result = proc.process_pdu(&mut image, PduSource::Server, &[]);
    assert!(result.is_err(), "empty input should return an error");
}

#[test]
fn process_pdu_returns_error_for_truncated_pdu() {
    // oracle: mutant accept a truncated PDU as an incomplete no-op.
    let mut proc = test_processor();
    let mut image = test_image();
    // A single byte is insufficient for find_size to determine the PDU length.
    let result = proc.process_pdu(&mut image, PduSource::Server, &[0x00]);
    assert!(result.is_err(), "truncated PDU should return an error");
}

#[test]
fn process_pdu_returns_error_for_garbage_bytes() {
    // oracle: mutant ignore unrecognized PDU headers instead of returning a decode error.
    let mut proc = test_processor();
    let mut image = test_image();
    let result = proc.process_pdu(&mut image, PduSource::Server, &[0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(result.is_err(), "garbage bytes should return an error");
}

#[test]
fn process_client_fastpath_rejects_length_that_cannot_contain_extended_event_count() {
    // oracle: spec MS-RDPBCGR §2.2.8.1.2 defines the overall PDU length and optional numEvents field.
    let mut proc = test_processor();
    let mut image = test_image();
    let result = proc.process_pdu(&mut image, PduSource::Client, &[0x00, 0x02, 0x00]);
    assert!(result.is_err(), "PDU length must include the extended event-count byte");
}

#[test]
fn process_client_fastpath_extracts_mouse_positions() {
    let mut proc = test_processor();
    let mut image = test_image();
    let results = proc
        .process_pdu(&mut image, PduSource::Client, &FASTPATH_CLIENT_INPUT)
        .unwrap();

    let positions: Vec<(u16, u16)> = results
        .iter()
        .filter_map(|r| match r {
            ProcessResult::ClientPointerPosition { x, y } => Some((*x, *y)),
            _ => None,
        })
        .collect();

    // oracle: spec MS-RDPBCGR §Section_2.2.8.1.2 for the event count.
    assert_eq!(positions.len(), 6);
    // oracle: spec MS-RDPBCGR §Section_2.2.8.1.2.2.3 for the mouse coordinates.
    // Decode the first and last coordinate pairs independently from the fixture's event fields.
    assert_eq!(positions[0], (26, 1062));
    assert_eq!(positions[5], (25, 1064));
}

#[test]
fn process_server_fastpath_routes_as_output() {
    // oracle: spec MS-RDPBCGR §Section_2.2.9.1.2.1.6.
    let mut proc = test_processor();
    let mut image = test_image();

    let results = proc
        .process_pdu(&mut image, PduSource::Server, &FASTPATH_SERVER_POINTER_DEFAULT)
        .unwrap();

    assert!(matches!(
        results.as_slice(),
        [ProcessResult::FastPath(ironrdp_web_replay::UpdateKind::PointerDefault)]
    ));
}

#[test]
fn process_client_pdu_suppressed_during_seek() {
    // oracle: mutant emit client pointer events while the canvas is suppressed.
    let mut proc = test_processor();
    let mut image = test_image();
    proc.set_update_canvas(false);

    let results = proc
        .process_pdu(&mut image, PduSource::Client, &FASTPATH_CLIENT_INPUT)
        .unwrap();

    assert!(
        results.is_empty(),
        "client PDUs should be suppressed when update_canvas is false"
    );
}

// Tests for seek suppression and update_canvas.

#[test]
fn update_canvas_defaults_to_true() {
    // oracle: mutant initialize canvas updates as disabled.
    let proc = test_processor();
    assert!(proc.update_canvas());
}

#[test]
fn set_update_canvas_toggles_flag() {
    // oracle: mutant ignore requested update_canvas state changes.
    let mut proc = test_processor();
    proc.set_update_canvas(false);
    assert!(!proc.update_canvas());
    proc.set_update_canvas(true);
    assert!(proc.update_canvas());
}

#[test]
fn pointer_state_defaults_to_default() {
    // oracle: mutant initialize the pointer state as Hidden.
    let proc = test_processor();
    assert!(
        matches!(proc.current_pointer_state(), PointerState::Default),
        "initial pointer state should be Default"
    );
}

// Tests for process_till.

#[test]
fn process_till_empty_buffer_returns_zero_pdus() {
    // oracle: mutant count an empty buffer as processed.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    let result = proc.process_till(&mut buffer, &mut image, 1000.0);

    assert_eq!(result.pdus_processed, 0);
    assert!(result.new_resolution.is_none());
    assert!(!result.session_ended);
    assert!(!result.frame_dirty);
    assert!(result.errors.is_empty());
}

#[test]
fn process_till_consumes_pdus_up_to_target() {
    // oracle: mutant process a PDU whose timestamp is after the target.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    // Push three client PDUs at different timestamps.
    buffer.push_pdu(100.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);
    buffer.push_pdu(200.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);
    buffer.push_pdu(300.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    // Processing up to 250 ms consumes the PDUs at 100 and 200 ms but not the one at 300 ms.
    let result = proc.process_till(&mut buffer, &mut image, 250.0);

    assert_eq!(result.pdus_processed, 2);
    assert_eq!(buffer.count(), 1, "one PDU should remain in buffer");
    assert_eq!(buffer.peek_timestamp(), Some(300.0));
}

#[test]
fn process_till_preserves_insertion_order_for_out_of_order_timestamps() {
    // oracle: mutant reorder buffered PDUs by timestamp instead of preserving insertion order.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();
    buffer.push_pdu(100.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);
    buffer.push_pdu(200.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);
    buffer.push_pdu(50.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    let first = proc.process_till(&mut buffer, &mut image, 100.0);

    assert_eq!(first.pdus_processed, 1);
    assert_eq!(buffer.peek_timestamp(), Some(200.0));
    assert_eq!(buffer.count(), 2);

    let second = proc.process_till(&mut buffer, &mut image, 200.0);
    assert_eq!(second.pdus_processed, 2);
    assert_eq!(buffer.peek_timestamp(), None);
    assert_eq!(buffer.count(), 0);
}

#[test]
fn process_till_tracks_mouse_position() {
    // oracle: spec MS-RDPBCGR §Section_2.2.8.1.2.2.3.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    buffer.push_pdu(100.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    let result = proc.process_till(&mut buffer, &mut image, 200.0);

    assert_eq!(result.pdus_processed, 1);
    assert_eq!(result.last_mouse_position, Some((25, 1064)));
}

#[test]
fn process_till_with_seek_suppression() {
    // oracle: mutant emit visual output from client PDUs during seek.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    proc.set_update_canvas(false);

    buffer.push_pdu(100.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    let result = proc.process_till(&mut buffer, &mut image, 200.0);

    assert_eq!(result.pdus_processed, 1, "PDU should still be consumed");
    assert!(!result.frame_dirty, "no visual results when canvas suppressed");
    assert!(
        result.last_mouse_position.is_none(),
        "client PDUs suppressed during seek"
    );
}

#[test]
fn process_till_collects_malformed_pdu_errors_and_continues() {
    // oracle: mutant discard a malformed-PDU error instead of continuing with later PDUs.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    // Push a garbage PDU followed by a valid client PDU.
    // The garbage bytes fail in process_pdu and produce a ReplayError.
    // process_till captures the error and continues to the next PDU.
    buffer.push_pdu(100.0, PduSource::Server, &[0xFF, 0xFF]);
    buffer.push_pdu(200.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    let result = proc.process_till(&mut buffer, &mut image, 300.0);

    // The garbage PDU is captured as an error.
    assert_eq!(result.errors.len(), 1, "malformed PDU should produce one error");
    // The valid PDU is processed successfully.
    assert_eq!(result.pdus_processed, 1, "only the valid PDU counts as processed");
    // Both PDUs are drained from the buffer.
    assert_eq!(buffer.count(), 0, "buffer should be empty");
    // The valid client PDU's last mouse event is tracked.
    assert_eq!(result.last_mouse_position, Some((25, 1064)));
}

// Tests for X.224 session control and resolution changes.

/// Server Demand Active X.224 PDU fixture with a bitmap capability for 1470x802.
// The fixture's capture provenance is unknown; expected dimensions are decoded from its capability fields.
// oracle: spec MS-RDPBCGR §Section_2.2.1.13.1.1 and §Section_2.2.7.1.2.
const X224_SERVER_DEMAND_ACTIVE: &[u8] = include_bytes!("../../test_data/pdu/web_replay/x224_server_demand_active.bin");

#[test]
fn client_direction_demand_active_does_not_change_resolution() {
    // oracle: spec MS-RDPBCGR §Section_2.2.1.13.1.1.
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();
    buffer.push_pdu(0.0, PduSource::Client, X224_SERVER_DEMAND_ACTIVE);

    let result = proc.process_till(&mut buffer, &mut image, 0.0);
    assert!(result.new_resolution.is_none());
    assert_eq!((image.width(), image.height()), (64, 64));
}

#[test]
fn oversized_demand_active_is_rejected_without_reallocating() {
    // oracle: mutant accept desktop dimensions above the replay limit.
    let mut pdu = X224_SERVER_DEMAND_ACTIVE.to_vec();
    assert_eq!(&pdu[125..129], &[0xbe, 0x05, 0x22, 0x03]);
    pdu[125..127].copy_from_slice(&8193u16.to_le_bytes());
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();
    buffer.push_pdu(0.0, PduSource::Server, &pdu);

    let result = proc.process_till(&mut buffer, &mut image, 0.0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.new_resolution.is_none());
    assert_eq!((image.width(), image.height()), (64, 64));
}

#[test]
fn zero_desktop_width_is_rejected_without_reallocating() {
    // oracle: mutant accept zero desktop width in the Bitmap Capability Set.
    let mut pdu = X224_SERVER_DEMAND_ACTIVE.to_vec();
    pdu[125..127].copy_from_slice(&0u16.to_le_bytes());
    let mut proc = test_processor();
    let mut image = test_image();
    assert!(proc.process_pdu(&mut image, PduSource::Server, &pdu).is_err());
    assert_eq!((image.width(), image.height()), (64, 64));
}

#[test]
fn zero_desktop_height_is_rejected_without_reallocating() {
    // oracle: mutant accept zero desktop height in the Bitmap Capability Set.
    let mut pdu = X224_SERVER_DEMAND_ACTIVE.to_vec();
    pdu[127..129].copy_from_slice(&0u16.to_le_bytes());
    let mut proc = test_processor();
    let mut image = test_image();
    assert!(proc.process_pdu(&mut image, PduSource::Server, &pdu).is_err());
    assert_eq!((image.width(), image.height()), (64, 64));
}

#[test]
fn maximum_desktop_dimensions_are_accepted() {
    // oracle: mutant reject the largest desktop dimensions accepted by the replay limit.
    let mut pdu = X224_SERVER_DEMAND_ACTIVE.to_vec();
    pdu[125..127].copy_from_slice(&8192u16.to_le_bytes());
    pdu[127..129].copy_from_slice(&8192u16.to_le_bytes());
    let mut proc = test_processor();
    let mut image = test_image();
    let results = proc.process_pdu(&mut image, PduSource::Server, &pdu).unwrap();
    assert!(results.iter().any(|result| matches!(
        result,
        ProcessResult::ResolutionChanged {
            width: 8192,
            height: 8192
        }
    )));
}

#[test]
fn encrypted_fastpath_flag_is_rejected_for_client_during_seek() {
    // oracle: mutant accept an encrypted client Fast-Path PDU.
    assert_fastpath_security_flags_rejected(PduSource::Client, 0x80);
}

#[test]
fn encrypted_fastpath_flag_is_rejected_for_server_during_seek() {
    // oracle: mutant accept an encrypted server Fast-Path PDU.
    assert_fastpath_security_flags_rejected(PduSource::Server, 0x80);
}

#[test]
fn secure_checksum_fastpath_flag_is_rejected_for_client_during_seek() {
    // oracle: mutant accept a client Fast-Path PDU with the secure-checksum flag.
    assert_fastpath_security_flags_rejected(PduSource::Client, 0x40);
}

#[test]
fn secure_checksum_fastpath_flag_is_rejected_for_server_during_seek() {
    // oracle: mutant accept a server Fast-Path PDU with the secure-checksum flag.
    assert_fastpath_security_flags_rejected(PduSource::Server, 0x40);
}

fn assert_fastpath_security_flags_rejected(source: PduSource, flag: u8) {
    let mut proc = test_processor();
    let mut image = test_image();
    proc.set_update_canvas(false);
    let mut pdu = match source {
        PduSource::Client => FASTPATH_CLIENT_INPUT.to_vec(),
        PduSource::Server => FASTPATH_SERVER_POINTER_DEFAULT.to_vec(),
    };
    pdu[0] |= flag;
    assert!(proc.process_pdu(&mut image, source, &pdu).is_err());
}

#[test]
fn process_x224_demand_active_returns_resolution() {
    // oracle: spec MS-RDPBCGR §Section_2.2.1.13.1.1 and §Section_2.2.7.1.2.
    let mut proc = test_processor();
    let mut image = test_image();

    let results = proc
        .process_pdu(&mut image, PduSource::Server, X224_SERVER_DEMAND_ACTIVE)
        .unwrap();

    let resolution = results.iter().find_map(|r| match r {
        ProcessResult::ResolutionChanged { width, height } => Some((*width, *height)),
        _ => None,
    });

    // The expected dimensions are decoded from those fields in the fixed fixture.
    assert_eq!(
        resolution,
        Some((1470, 802)),
        "Server Demand Active should report resolution 1470x802"
    );
}

#[test]
fn process_till_x224_resolution_change_reallocates_image() {
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image(); // The image starts at 64x64.

    buffer.push_pdu(100.0, PduSource::Server, X224_SERVER_DEMAND_ACTIVE);

    let result = proc.process_till(&mut buffer, &mut image, 200.0);

    // oracle: spec MS-RDPBCGR §Section_2.2.1.13.1.1 and §Section_2.2.7.1.2.
    assert_eq!(result.new_resolution, Some((1470, 802)));
    // oracle: mutant report the new resolution without reallocating the framebuffer.
    assert_eq!(image.width(), 1470);
    assert_eq!(image.height(), 802);
}

#[test]
fn process_till_x224_resolution_emitted_regardless_of_update_canvas() {
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    proc.set_update_canvas(false);

    buffer.push_pdu(100.0, PduSource::Server, X224_SERVER_DEMAND_ACTIVE);

    let result = proc.process_till(&mut buffer, &mut image, 200.0);

    // oracle: mutant suppress session-state resolution changes during seek.
    // oracle: spec MS-RDPBCGR §Section_2.2.1.13.1.1 and §Section_2.2.7.1.2.
    assert_eq!(
        result.new_resolution,
        Some((1470, 802)),
        "resolution changes are emitted during seek"
    );
}
