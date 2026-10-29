use std::sync::{Arc, Mutex};

use ironrdp_core::decode;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu;
use ironrdp_displaycontrol::server::{DisplayControlHandler, DisplayControlServer};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_pdu::{PduResult, decode_err};
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
                .with_position(-1024, 0).unwrap()
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
        0x00, 0xFC, 0xFF, 0xFF,
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

fn primary(width: u32, height: u32) -> pdu::MonitorLayoutEntry {
    pdu::MonitorLayoutEntry::new_primary(width, height).unwrap()
}

fn secondary(width: u32, height: u32, left: i32, top: i32) -> pdu::MonitorLayoutEntry {
    pdu::MonitorLayoutEntry::new_secondary(width, height)
        .unwrap()
        .with_position(left, top)
        .unwrap()
}

#[test]
fn monitor_layout_rejects_overlapping_monitors() {
    pdu::DisplayControlMonitorLayout::new(&[primary(1920, 1080), secondary(1920, 1080, 1919, 0)])
        .expect_err("monitors sharing a column of pixels overlap");
    pdu::DisplayControlMonitorLayout::new(&[
        primary(1920, 1080),
        pdu::MonitorLayoutEntry::new_secondary(800, 600).unwrap(),
    ])
    .expect_err("a secondary monitor left at the origin overlaps the primary");
}

#[test]
fn monitor_layout_requires_adjacent_monitors() {
    pdu::DisplayControlMonitorLayout::new(&[primary(1920, 1080), secondary(1920, 1080, 1922, 0)])
        .expect_err("a monitor separated by a gap is not adjacent");

    // Monitors touching on an edge, or only at a corner, are adjacent per MS-RDPEDISP 3.1.5.2.
    pdu::DisplayControlMonitorLayout::new(&[
        primary(1920, 1080),
        secondary(1920, 1080, -1920, 0),
        secondary(1080, 1920, 1920, -1920),
        secondary(1280, 1024, 0, 1080),
    ])
    .expect("edge- and corner-adjacent monitors form a valid layout");
}

#[test]
fn monitor_layout_desktop_size() {
    let layout =
        pdu::DisplayControlMonitorLayout::new(&[primary(1920, 1080), secondary(1080, 1920, 1920, -420)]).unwrap();
    assert_eq!(layout.desktop_size(), (3000, 1920));
}

#[test]
fn monitor_layout_fit_drops_monitors_beyond_the_server_maximum() {
    let layout = pdu::DisplayControlMonitorLayout::new(&[
        secondary(1920, 1080, -1920, 0),
        primary(1920, 1080),
        secondary(1920, 1080, 1920, 0),
    ])
    .unwrap();

    let fitted = layout
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(2, 3840, 2400).unwrap())
        .unwrap();
    let monitors = fitted.monitors();
    assert_eq!(monitors.len(), 2);
    assert_eq!(monitors[0].position(), Some((-1920, 0)));
    assert!(monitors[1].is_primary());

    let fitted = layout
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(1, 3840, 2400).unwrap())
        .unwrap();
    assert_eq!(fitted.monitors().len(), 1);
    assert!(fitted.monitors()[0].is_primary());

    layout
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(0, 3840, 2400).unwrap())
        .expect_err("a server accepting no monitor cannot receive a layout");

    let unchanged = layout
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(16, 3840, 2400).unwrap())
        .unwrap();
    assert_eq!(unchanged, layout);
}

#[test]
fn monitor_layout_fit_keeps_the_reduced_layout_connected() {
    // The first secondary monitor listed only touches the second one, which bridges it to the primary.
    let layout = pdu::DisplayControlMonitorLayout::new(&[
        primary(1920, 1080),
        secondary(1920, 1080, 3840, 0),
        secondary(1920, 1080, 1920, 0),
    ])
    .unwrap();

    let fitted = layout
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(2, 3840, 2400).unwrap())
        .unwrap();
    let monitors = fitted.monitors();
    assert_eq!(monitors.len(), 2);
    assert!(monitors[0].is_primary());
    assert_eq!(monitors[1].position(), Some((1920, 0)));
}

#[test]
fn monitor_layout_fit_scales_a_single_monitor_down_to_the_maximum_area() {
    let layout = pdu::DisplayControlMonitorLayout::new_single_primary_monitor(7680, 4320, Some(200), None).unwrap();
    let capabilities = pdu::DisplayControlCapabilities::new(1, 3840, 2400).unwrap();

    let fitted = layout.fit_to_capabilities(&capabilities).unwrap();
    let [monitor] = fitted.monitors() else {
        panic!("expected a single monitor");
    };
    let (width, height) = monitor.dimensions();
    assert!(u64::from(width) * u64::from(height) <= capabilities.max_monitor_area());
    assert_eq!(width % 2, 0);
    // The 16:9 aspect ratio is preserved, give or take rounding.
    assert_eq!((width, height), (4046, 2276));
    assert_eq!(monitor.desktop_scale_factor(), Some(200));

    pdu::DisplayControlMonitorLayout::new_single_primary_monitor(1024, 768, None, None)
        .unwrap()
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(1, 100, 100).unwrap())
        .expect_err("the minimum monitor size exceeds a 100x100 maximum area");
}

#[test]
fn monitor_layout_fit_rejects_multiple_monitors_beyond_the_maximum_area() {
    let layout = pdu::DisplayControlMonitorLayout::new(&[primary(3840, 2160), secondary(3840, 2160, 3840, 0)]).unwrap();

    layout
        .fit_to_capabilities(&pdu::DisplayControlCapabilities::new(2, 1920, 1080).unwrap())
        .expect_err("shrinking several monitors would move them relative to each other");
}

#[test]
fn single_primary_monitor_derives_orientation_and_clamps_scale() {
    let layout = pdu::DisplayControlMonitorLayout::new_single_primary_monitor(1024, 1024, Some(50), None).unwrap();
    let monitor = &layout.monitors()[0];
    assert_eq!(monitor.orientation(), Some(pdu::MonitorOrientation::Landscape));
    assert_eq!(monitor.desktop_scale_factor(), Some(100));
    assert_eq!(
        monitor.device_scale_factor(),
        Some(pdu::DeviceScaleFactor::Scale100Percent)
    );

    let layout = pdu::DisplayControlMonitorLayout::new_single_primary_monitor(1080, 1920, Some(150), None).unwrap();
    let monitor = &layout.monitors()[0];
    assert_eq!(monitor.orientation(), Some(pdu::MonitorOrientation::Portrait));
    assert_eq!(monitor.desktop_scale_factor(), Some(150));
    assert_eq!(
        monitor.device_scale_factor(),
        Some(pdu::DeviceScaleFactor::Scale140Percent)
    );

    let layout = pdu::DisplayControlMonitorLayout::new_single_primary_monitor(1920, 1080, Some(900), None).unwrap();
    let monitor = &layout.monitors()[0];
    assert_eq!(monitor.desktop_scale_factor(), Some(500));
    assert_eq!(
        monitor.device_scale_factor(),
        Some(pdu::DeviceScaleFactor::Scale180Percent)
    );
}

#[test]
fn device_scale_factor_values() {
    assert_eq!(
        pdu::DeviceScaleFactor::from_value(140),
        Some(pdu::DeviceScaleFactor::Scale140Percent)
    );
    assert_eq!(pdu::DeviceScaleFactor::from_value(125), None);
    assert_eq!(
        pdu::DeviceScaleFactor::from_desktop_scale_factor(139),
        pdu::DeviceScaleFactor::Scale100Percent
    );
    assert_eq!(
        pdu::DeviceScaleFactor::from_desktop_scale_factor(179),
        pdu::DeviceScaleFactor::Scale140Percent
    );
    assert_eq!(
        pdu::DeviceScaleFactor::from_desktop_scale_factor(180),
        pdu::DeviceScaleFactor::Scale180Percent
    );
    assert_eq!(pdu::MonitorLayoutEntry::adjust_desktop_scale_factor(0), 100);
    assert_eq!(pdu::MonitorLayoutEntry::adjust_desktop_scale_factor(250), 250);
    assert_eq!(pdu::MonitorLayoutEntry::adjust_desktop_scale_factor(501), 500);
}

#[test]
fn client_encodes_monitor_layouts_only_after_capabilities() {
    let mut client = DisplayControlClient::new(|_caps| Ok(Vec::new()));
    let layout = pdu::DisplayControlMonitorLayout::new(&[primary(1920, 1080), secondary(1920, 1080, 1920, 0)]).unwrap();

    client
        .encode_monitor_layout(1, &layout)
        .expect_err("the server capabilities bound every layout");

    let caps = ironrdp_core::encode_vec(&pdu::DisplayControlPdu::Caps(
        pdu::DisplayControlCapabilities::new(2, 3840, 2400).unwrap(),
    ))
    .unwrap();
    client.process(1, &caps).unwrap();
    assert_eq!(client.capabilities().map(|caps| caps.max_num_monitors()), Some(2));
    assert!(!client.encode_monitor_layout(1, &layout).unwrap().is_empty());
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

struct OutOfRangeCapsHandler;

impl DisplayControlHandler for OutOfRangeCapsHandler {
    fn capabilities(&self) -> PduResult<pdu::DisplayControlCapabilities> {
        // More than 1024 monitors is rejected by DisplayControlCapabilities::new, per
        // invalid_caps above.
        pdu::DisplayControlCapabilities::new(2000, 100, 100).map_err(|e| decode_err!(e))
    }
}

#[test]
fn server_start_propagates_an_invalid_capabilities_override_as_an_error() {
    // A handler deriving capabilities from real runtime state can hit the wire-encoding bound
    // DisplayControlCapabilities::new enforces; start() must surface that as a PduResult::Err
    // rather than let a handler's own expect()/panic tear down the connection.
    let mut server = DisplayControlServer::new(Box::new(OutOfRangeCapsHandler));
    assert!(server.start(0).is_err());
}

/// Decode a `DvcMessage` produced by `DisplayControlServer::start()` back into the
/// `DisplayControlPdu` it encodes, the same round trip a real client would perform.
fn decode_dvc(msg: &ironrdp_dvc::DvcMessage) -> pdu::DisplayControlPdu {
    let bytes = ironrdp_core::encode_vec(msg.as_ref()).expect("encode dvc message");
    decode(&bytes).expect("decode dvc message")
}

struct MultiMonitorCapsHandler;

impl DisplayControlHandler for MultiMonitorCapsHandler {
    fn capabilities(&self) -> PduResult<pdu::DisplayControlCapabilities> {
        pdu::DisplayControlCapabilities::new(2, 3840, 2400).map_err(|e| decode_err!(e))
    }
}

#[test]
fn server_start_encodes_a_multi_monitor_override() {
    // The motivating case for a fallible, overridable capabilities(): a handler
    // advertising more than the hardcoded single-monitor default. Verify the
    // override's values actually reach the encoded DISPLAYCONTROL_CAPS_PDU, not
    // just that start() succeeds.
    let mut server = DisplayControlServer::new(Box::new(MultiMonitorCapsHandler));
    let messages = server.start(0).expect("valid override should not error");
    assert_eq!(messages.len(), 1);
    match decode_dvc(&messages[0]) {
        pdu::DisplayControlPdu::Caps(caps) => {
            assert_eq!(caps, pdu::DisplayControlCapabilities::new(2, 3840, 2400).unwrap());
        }
        other => panic!("expected DisplayControlPdu::Caps, got {other:?}"),
    }
}

struct DefaultCapsHandler;

impl DisplayControlHandler for DefaultCapsHandler {}

#[test]
fn server_start_encodes_the_default_capabilities_when_not_overridden() {
    // The default capabilities() impl (single monitor, 3840x2400) was the
    // hardcoded start() literal before this PR and remains untested; pin it so a
    // regression in the default constants or the .into() encoding step is caught.
    let mut server = DisplayControlServer::new(Box::new(DefaultCapsHandler));
    let messages = server.start(0).expect("default capabilities should not error");
    assert_eq!(messages.len(), 1);
    match decode_dvc(&messages[0]) {
        pdu::DisplayControlPdu::Caps(caps) => {
            assert_eq!(caps, pdu::DisplayControlCapabilities::new(1, 3840, 2400).unwrap());
        }
        other => panic!("expected DisplayControlPdu::Caps, got {other:?}"),
    }
}
