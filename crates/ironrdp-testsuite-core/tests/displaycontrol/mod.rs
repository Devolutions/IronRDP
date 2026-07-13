use std::sync::{Arc, Mutex};

use ironrdp_core::decode;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_testsuite_core::encode_decode_test;

encode_decode_test! {
    capabilities: pdu::DisplayControlPdu::Caps(pdu::DisplayControlCapabilities::new(
        3, 1920, 1080
    ).unwrap()),
    [
        // Header
        0x05, 0x00, 0x00, 0x00,
        0x14, 0x00, 0x00, 0x00,
        // Payload
        0x03, 0x00, 0x00, 0x00,
        0x80, 0x07, 0x00, 0x00,
        0x38, 0x04, 0x00, 0x00,
    ];

    layout: pdu::DisplayControlPdu::MonitorLayout(pdu::DisplayControlMonitorLayout::new(
        &[
            pdu::MonitorLayoutEntry::new_primary(1920, 1080).unwrap()
                .with_orientation(pdu::MonitorOrientation::LandscapeFlipped)
                .with_physical_dimensions(1000, 500).unwrap()
                .with_position(0, 0).unwrap()
                .with_device_scale_factor(pdu::DeviceScaleFactor::Scale140Percent)
                .with_desktop_scale_factor(150).unwrap(),
            pdu::MonitorLayoutEntry::new_secondary(1024, 768).unwrap()
                .with_orientation(pdu::MonitorOrientation::Portrait)
                .with_physical_dimensions(500, 500).unwrap()
                .with_position(-500, 0).unwrap()
                .with_device_scale_factor(pdu::DeviceScaleFactor::Scale100Percent)
                .with_desktop_scale_factor(100).unwrap()
        ]
    ).unwrap()),
    [
        // Header
        0x02, 0x00, 0x00, 0x00,
        0x60, 0x00, 0x00, 0x00,
        // Payload
        0x28, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00,

        // Monitor 1
        0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x80, 0x07, 0x00, 0x00,
        0x38, 0x04, 0x00, 0x00,
        0xE8, 0x03, 0x00, 0x00,
        0xF4, 0x01, 0x00, 0x00,
        0xB4, 0x00, 0x00, 0x00,
        0x96, 0x00, 0x00, 0x00,
        0x8C, 0x00, 0x00, 0x00,

        // Monitor 2
        0x00, 0x00, 0x00, 0x00,
        0x0C, 0xFE, 0xFF, 0xFF,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x04, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x00,
        0xF4, 0x01, 0x00, 0x00,
        0xF4, 0x01, 0x00, 0x00,
        0x5A, 0x00, 0x00, 0x00,
        0x64, 0x00, 0x00, 0x00,
        0x64, 0x00, 0x00, 0x00,
    ];
}

#[test]
fn invalid_caps() {
    pdu::DisplayControlCapabilities::new(2000, 100, 100).expect_err("more than 1024 monitors should not be allowed");

    pdu::DisplayControlCapabilities::new(100, 32 * 1024, 100)
        .expect_err("resolution more than 8k should not be a valid value");
}

#[test]
fn monitor_layout_entry_odd_dimensions_adjustment() {
    let odd_value = 1023;
    let entry = pdu::MonitorLayoutEntry::new_primary(odd_value, odd_value).expect("valid entry should be created");
    let (width, height) = entry.dimensions();
    assert_eq!(width, odd_value - 1);
    assert_eq!(height, odd_value);
}

#[test]
fn invalid_monitor_layout_entry() {
    pdu::MonitorLayoutEntry::new_primary(32 * 1024, 32 * 1024)
        .expect_err("resolution more than 8k should not be allowed");

    pdu::MonitorLayoutEntry::new_primary(1024, 1024)
        .unwrap()
        .with_position(-1, 1)
        .expect_err("primary monitor should always have (0, 0) position");

    pdu::MonitorLayoutEntry::new_primary(1024, 1024)
        .unwrap()
        .with_position(-1, 1)
        .expect_err("primary monitor should always have (0, 0) position");

    pdu::MonitorLayoutEntry::new_primary(1024, 1024)
        .unwrap()
        .with_desktop_scale_factor(999)
        .expect_err("invalid desktop factor should be rejected");

    pdu::MonitorLayoutEntry::new_primary(1024, 1024)
        .unwrap()
        .with_physical_dimensions(1, 9999)
        .expect_err("invalid physical dimensions should be rejected");
}

#[test]
fn only_non_optional_layout_fields_required_to_be_valid() {
    let encoded = [
        0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x38, 0x04,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00,
    ];

    let decoded = decode::<pdu::MonitorLayoutEntry>(&encoded).unwrap();

    assert!(decoded.desktop_scale_factor().is_none());
    assert!(decoded.device_scale_factor().is_none());
    assert!(decoded.orientation().is_none());
    assert!(decoded.physical_dimensions().is_none());
    assert!(decoded.position().is_none())
}

#[test]
fn client_process_decodes_headered_caps() {
    let received = Arc::new(Mutex::new(None));
    let received_clone = Arc::clone(&received);
    let mut client = DisplayControlClient::new(move |caps| {
        *received_clone.lock().unwrap() = Some(caps);
        Ok(Vec::new())
    });

    // DISPLAYCONTROL_HEADER (Type + Length) followed by the DISPLAYCONTROL_CAPS_PDU body,
    // per MS-RDPEDISP 2.2.2.1 -- the same bytes as the `capabilities` case above.
    let headered_caps = [
        // Header
        0x05, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, // Payload
        0x03, 0x00, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x38, 0x04, 0x00, 0x00,
    ];

    assert!(!client.ready());
    client
        .process(0, &headered_caps)
        .expect("headered caps PDU should be accepted");
    assert!(client.ready());
    assert_eq!(
        received.lock().unwrap().as_ref(),
        Some(&pdu::DisplayControlCapabilities::new(3, 1920, 1080).unwrap())
    );
}

#[test]
fn client_process_rejects_monitor_layout_from_server() {
    let mut client = DisplayControlClient::new(|_caps| Ok(Vec::new()));

    // A DISPLAYCONTROL_MONITOR_LAYOUT_PDU is a client-to-server message; the server should
    // never send one, and process() must reject it rather than silently accepting it as caps.
    let layout = [
        // Header
        0x02, 0x00, 0x00, 0x00, 0x60, 0x00, 0x00, 0x00, // Payload
        0x28, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // Monitor 1
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x38, 0x04,
        0x00, 0x00, 0xE8, 0x03, 0x00, 0x00, 0xF4, 0x01, 0x00, 0x00, 0xB4, 0x00, 0x00, 0x00, 0x96, 0x00, 0x00, 0x00,
        0x8C, 0x00, 0x00, 0x00, // Monitor 2
        0x00, 0x00, 0x00, 0x00, 0x0C, 0xFE, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x03,
        0x00, 0x00, 0xF4, 0x01, 0x00, 0x00, 0xF4, 0x01, 0x00, 0x00, 0x5A, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00,
        0x64, 0x00, 0x00, 0x00,
    ];

    assert!(
        client.process(0, &layout).is_err(),
        "a MONITOR_LAYOUT_PDU from the server should be rejected"
    );
    assert!(!client.ready());
}

#[test]
fn client_process_rejects_caps_with_mismatched_length() {
    let mut client = DisplayControlClient::new(|_caps| Ok(Vec::new()));

    let mismatched_length = [
        // Header: Type = Caps, Length = 0xffff_ffff
        0x05, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, // Payload
        0x03, 0x00, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x38, 0x04, 0x00, 0x00,
    ];

    assert!(
        client.process(0, &mismatched_length).is_err(),
        "a Length that doesn't match the actual payload size should be rejected"
    );
    assert!(!client.ready());
}

#[test]
fn client_process_rejects_trailing_bytes_after_caps() {
    let mut client = DisplayControlClient::new(|_caps| Ok(Vec::new()));

    let trailing_bytes = [
        // Header: Type = Caps, Length = 24 (matches the 16-byte payload below)
        0x05, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, // Payload: valid 12-byte caps body
        0x03, 0x00, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x38, 0x04, 0x00, 0x00, // + 4 trailing bytes
        0xAA, 0xAA, 0xAA, 0xAA,
    ];

    assert!(
        client.process(0, &trailing_bytes).is_err(),
        "bytes left over after decoding the caps body should be rejected"
    );
    assert!(!client.ready());
}
