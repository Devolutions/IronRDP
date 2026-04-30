use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use ironrdp_web_replay::{PduBuffer, PduSource, PointerState, ProcessResult, ReplayProcessor, ReplayProcessorConfig};

/// Captured FastPath client input message (44 bytes, 6 mouse events).
const FASTPATH_CLIENT_INPUT: [u8; 44] = [
    0x18, 0x2c, 0x20, 0x0, 0x90, 0x1a, 0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1b, 0x0, 0x26, 0x4, 0x20, 0x0, 0x10, 0x1b,
    0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1a, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19,
    0x0, 0x28, 0x4,
];

fn test_processor() -> ReplayProcessor {
    ReplayProcessor::new(&ReplayProcessorConfig::default())
}

fn test_image() -> DecodedImage {
    DecodedImage::new(PixelFormat::RgbA32, 64, 64)
}

// process_pdu

#[test]
fn process_pdu_returns_error_for_empty_input() {
    let mut proc = test_processor();
    let mut image = test_image();
    let result = proc.process_pdu(&mut image, PduSource::Server, &[]);
    assert!(result.is_err(), "empty input should return an error");
}

#[test]
fn process_pdu_returns_error_for_truncated_pdu() {
    let mut proc = test_processor();
    let mut image = test_image();
    // Single byte — find_size cannot determine PDU length
    let result = proc.process_pdu(&mut image, PduSource::Server, &[0x00]);
    assert!(result.is_err(), "truncated PDU should return an error");
}

#[test]
fn process_pdu_returns_error_for_garbage_bytes() {
    let mut proc = test_processor();
    let mut image = test_image();
    let result = proc.process_pdu(&mut image, PduSource::Server, &[0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(result.is_err(), "garbage bytes should return an error");
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

    // The captured PDU contains 6 mouse events
    assert_eq!(positions.len(), 6);
    assert_eq!(positions[0], (26, 1062));
    assert_eq!(positions[5], (25, 1064));
}

#[test]
fn process_client_pdu_suppressed_during_seek() {
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

// Seek suppression / update_canvas

#[test]
fn update_canvas_defaults_to_true() {
    let proc = test_processor();
    assert!(proc.update_canvas());
}

#[test]
fn set_update_canvas_toggles_flag() {
    let mut proc = test_processor();
    proc.set_update_canvas(false);
    assert!(!proc.update_canvas());
    proc.set_update_canvas(true);
    assert!(proc.update_canvas());
}

#[test]
fn pointer_state_defaults_to_default() {
    let proc = test_processor();
    assert!(
        matches!(proc.current_pointer_state(), PointerState::Default),
        "initial pointer state should be Default"
    );
}

// process_till

#[test]
fn process_till_empty_buffer_returns_zero_pdus() {
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    let result = proc.process_till(&mut buffer, &mut image, 1000.0);

    assert_eq!(result.pdus_processed, 0);
    assert!(!result.resolution_changed);
    assert!(!result.session_ended);
    assert!(!result.frame_dirty);
    assert!(result.errors.is_empty());
}

#[test]
fn process_till_consumes_pdus_up_to_target() {
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    // Push 3 client PDUs at different timestamps
    buffer.push_pdu(100.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);
    buffer.push_pdu(200.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);
    buffer.push_pdu(300.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    // Process up to 250ms — should consume 2 PDUs (100, 200) but not 300
    let result = proc.process_till(&mut buffer, &mut image, 250.0);

    assert_eq!(result.pdus_processed, 2);
    assert_eq!(buffer.count(), 1, "one PDU should remain in buffer");
    assert_eq!(buffer.peek_timestamp(), Some(300.0));
}

#[test]
fn process_till_tracks_mouse_position() {
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    buffer.push_pdu(100.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    let result = proc.process_till(&mut buffer, &mut image, 200.0);

    assert_eq!(result.pdus_processed, 1);
    // Last mouse event in FASTPATH_CLIENT_INPUT is (25, 1064)
    assert_eq!(result.last_mouse_position, Some((25, 1064)));
}

#[test]
fn process_till_with_seek_suppression() {
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
    let mut proc = test_processor();
    let mut buffer = PduBuffer::new();
    let mut image = test_image();

    // Push a garbage PDU followed by a valid client PDU.
    // The garbage bytes will fail in process_pdu (find_size returns
    // Err or Ok(None) → ReplayError). process_till captures it in
    // errors and continues to the next PDU.
    buffer.push_pdu(100.0, PduSource::Server, &[0xFF, 0xFF]);
    buffer.push_pdu(200.0, PduSource::Client, &FASTPATH_CLIENT_INPUT);

    let result = proc.process_till(&mut buffer, &mut image, 300.0);

    // The garbage PDU should be captured as an error.
    assert_eq!(result.errors.len(), 1, "malformed PDU should produce one error");
    // The valid PDU should be processed successfully.
    assert_eq!(result.pdus_processed, 1, "only the valid PDU counts as processed");
    // Both PDUs should be drained from the buffer.
    assert_eq!(buffer.count(), 0, "buffer should be empty");
    // The valid client PDU's last mouse event should be tracked.
    assert_eq!(result.last_mouse_position, Some((25, 1064)));
}

// X224 session control — resolution changes

/// Captured Server Demand Active X224 PDU (472 bytes).
/// Contains bitmap capability with desktopWidth=1470, desktopHeight=802.
const X224_SERVER_DEMAND_ACTIVE: &[u8] = include_bytes!("../../test_data/pdu/web_replay/x224_server_demand_active.bin");

#[test]
fn process_x224_demand_active_returns_resolution() {
    let mut proc = test_processor();
    let mut image = test_image();

    let results = proc
        .process_pdu(&mut image, PduSource::Server, X224_SERVER_DEMAND_ACTIVE)
        .unwrap();

    let resolution = results.iter().find_map(|r| match r {
        ProcessResult::ResolutionChanged { width, height } => Some((*width, *height)),
        _ => None,
    });

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
    let mut image = test_image(); // starts at 64x64

    buffer.push_pdu(100.0, PduSource::Server, X224_SERVER_DEMAND_ACTIVE);

    let result = proc.process_till(&mut buffer, &mut image, 200.0);

    assert!(result.resolution_changed);
    assert_eq!(result.new_resolution, Some((1470, 802)));
    // process_till reallocates the image internally
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

    assert!(
        result.resolution_changed,
        "resolution changes should be emitted even during seek"
    );
    assert_eq!(result.new_resolution, Some((1470, 802)));
}
