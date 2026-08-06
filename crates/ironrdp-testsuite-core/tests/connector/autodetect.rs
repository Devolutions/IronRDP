//! Connect-time auto-detection demux in the client connector.
//!
//! The continuous (session) auto-detect path is covered in
//! `tests/session/autodetect.rs`. These tests cover the connector's
//! `ConnectTimeAutoDetection` state, which demultiplexes the first PDU received
//! once a message channel has been negotiated: an Auto-Detect Request on the
//! message channel is answered, any other message-channel PDU is ignored, and a
//! PDU on the I/O channel is the first licensing PDU.

use std::borrow::Cow;

use ironrdp_connector::MonotonicInstant;
use ironrdp_connector::{ClientConnector, ClientConnectorState, Credentials, DesktopSize, Sequence as _, Written};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::gcc;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::server_license::{
    LicenseErrorCode, LicenseHeader, LicensePdu, LicensingErrorMessage, LicensingStateTransition, PreambleFlags,
    PreambleType, PreambleVersion,
};
use ironrdp_pdu::x224::X224;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const MESSAGE_CHANNEL_ID: u16 = 1004;

fn test_config() -> ironrdp_connector::Config {
    ironrdp_connector::Config {
        desktop_size: DesktopSize {
            width: 1024,
            height: 768,
        },
        desktop_scale_factor: 0,
        enable_tls: true,
        enable_credssp: false,
        credentials: Credentials::UsernamePassword {
            username: "test".into(),
            password: "test".into(),
        },
        domain: None,
        client_build: 0,
        client_name: "test".into(),
        keyboard_type: gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

/// A client connector parked in `ConnectTimeAutoDetection` with a negotiated
/// message channel, ready to receive the first PDU of that phase.
fn connect_time_autodetect_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:12345".parse().unwrap());
    connector.state = ClientConnectorState::ConnectTimeAutoDetection {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
    };
    connector.message_channel_id = Some(MESSAGE_CHANNEL_ID);
    connector
}

/// Frame a server-to-client SendDataIndication on the given MCS channel.
fn server_send_data_indication(channel_id: u16, user_data: Vec<u8>) -> Vec<u8> {
    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id,
        user_data: Cow::Owned(user_data),
    });

    encode_vec(&X224(indication)).unwrap()
}

#[test]
fn connect_time_autodetect_request_is_answered_and_phase_continues() {
    let mut connector = connect_time_autodetect_connector();

    let user_data = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::rtt_connect_time(0x1234))).unwrap();
    let frame = server_send_data_indication(MESSAGE_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    let written = connector.step(&frame, None, &mut output).unwrap();

    assert!(written.size().is_some(), "an RTT request must produce a response frame");
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectTimeAutoDetection { .. }),
        "the connector keeps listening after answering an auto-detect request"
    );
}

#[test]
fn unrelated_message_channel_pdu_is_ignored_and_phase_continues() {
    let mut connector = connect_time_autodetect_connector();

    // A message-channel PDU that is not an auto-detect request: a bare security
    // header without the SEC_AUTODETECT_REQ flag. It must be ignored, not handed
    // to the licensing sequence (which would try to decode it as a license PDU).
    let user_data = encode_vec(&BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::HEARTBEAT,
    })
    .unwrap();
    let frame = server_send_data_indication(MESSAGE_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    let written = connector.step(&frame, None, &mut output).unwrap();

    assert_eq!(
        written,
        Written::Nothing,
        "an unrelated message-channel PDU produces no response"
    );
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectTimeAutoDetection { .. }),
        "the connector keeps listening on the message channel"
    );
}

#[test]
fn first_licensing_pdu_leaves_autodetect_for_the_licensing_path() {
    let mut connector = connect_time_autodetect_connector();

    // The first PDU that is not on the message channel is the licensing PDU on
    // the I/O channel. A STATUS_VALID_CLIENT license error completes licensing in
    // a single step ([MS-RDPELE] 3.1.5.3.1), so the connector advances out of
    // auto-detection into multitransport bootstrapping.
    let license = LicensePdu::LicensingErrorMessage(LicensingErrorMessage {
        license_header: LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::ErrorAlert,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: 0x10,
        },
        error_code: LicenseErrorCode::StatusValidClient,
        state_transition: LicensingStateTransition::NoTransition,
        error_info: Vec::new(),
    });
    let user_data = encode_vec(&license).unwrap();
    let frame = server_send_data_indication(IO_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    connector.step(&frame, None, &mut output).unwrap();

    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportBootstrapping { .. }
        ),
        "a completed licensing exchange advances the connector out of auto-detection"
    );
}

#[test]
fn connect_time_bandwidth_measure_stop_is_answered_and_phase_continues() {
    let mut connector = connect_time_autodetect_connector();

    // A connect-time Bandwidth Measure Stop ([MS-RDPBCGR] 2.2.14.1.4) must be
    // answered with a Bandwidth Measure Results reply. FreeRDP-based servers (for
    // example GNOME Remote Desktop) block in their AWAIT_BW_RESULT state until they
    // receive it, so no response stalls the whole connection.
    let user_data = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::bw_stop_connect_time(
        0x5678,
        vec![0u8; 1024],
    )))
    .unwrap();
    let frame = server_send_data_indication(MESSAGE_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    let written = connector.step(&frame, None, &mut output).unwrap();

    assert!(
        written.size().is_some(),
        "a connect-time Bandwidth Measure Stop must produce a Bandwidth Measure Results response frame"
    );
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectTimeAutoDetection { .. }),
        "the connector keeps listening after answering the bandwidth measurement"
    );
}

/// Unwrap a Bandwidth Measure Results response and return `(time_delta_ms, byte_count)`.
///
/// The response frame is X224 > MCS SendDataRequest > Auto-Detect Response PDU.
fn decode_bandwidth_results(output: &WriteBuf) -> (u32, u32) {
    let X224(McsMessage::SendDataRequest(send_data)) = ironrdp_core::decode(output.filled()).unwrap() else {
        panic!("expected a SendDataRequest in the response frame");
    };

    let response = ironrdp_core::decode::<AutoDetectRspPdu>(&send_data.user_data).unwrap();
    match response.response {
        AutoDetectResponse::BandwidthMeasureResults {
            time_delta_ms,
            byte_count,
            ..
        } => (time_delta_ms, byte_count),
        other => panic!("expected BandwidthMeasureResults, got {other:?}"),
    }
}

/// The reported interval is the time between the Start that opened the window and
/// the Stop that closed it, taken from the arrival times the driver observed.
#[test]
fn connect_time_bandwidth_reports_the_measured_interval_and_total_bytes() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    let start = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::bw_start_connect_time(0x1111))).unwrap();
    connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, start),
            Some(MonotonicInstant::from_millis(1_000)),
            &mut output,
        )
        .unwrap();

    output.clear();
    let stop = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::bw_stop_connect_time(
        0x1111,
        vec![0u8; 4096],
    )))
    .unwrap();
    connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, stop),
            Some(MonotonicInstant::from_millis(1_250)),
            &mut output,
        )
        .unwrap();

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 250, "interval is Stop arrival minus Start arrival");
    assert_eq!(results.1, 4096, "byte count is what the server sent in the window");
}

/// A Stop with no preceding Start has nothing to have measured. It is still
/// answered, because the server blocks without a reply, but the interval reported
/// is the unmeasurable floor rather than an invented figure.
#[test]
fn connect_time_bandwidth_stop_without_start_reports_the_floor() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    let stop = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::bw_stop_connect_time(
        0x2222,
        vec![0u8; 512],
    )))
    .unwrap();
    let written = connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, stop),
            Some(MonotonicInstant::from_millis(9_999)),
            &mut output,
        )
        .unwrap();

    assert!(written.size().is_some(), "the server still needs its reply");
    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 1, "no window was open, so no interval was measured");
}

/// Start, Payload and Stop delivered by one socket read carry the same arrival
/// time, so the elapsed time rounds down to nothing. TCP coalescing makes this the
/// common case on a fast link, since the connect-time payload is small.
///
/// Reporting `timeDelta` of 0 would divide out to an unbounded bandwidth for a
/// server computing `byteCount * 8 / timeDelta`, so the floor is reported. Every
/// byte is still counted, per [MS-RDPBCGR] 3.2.5.14: the window was timed, and the
/// bytes really did arrive inside that millisecond, so the floor bounds a real
/// measurement instead of standing in for a missing one.
#[test]
fn connect_time_bandwidth_coalesced_into_one_read_reports_the_floor() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    // One instant for all three, which is what `Framed` hands down when a single
    // read filled the buffer that all three PDUs were then extracted from.
    let arrival = Some(MonotonicInstant::from_millis(7_000));

    for request in [
        AutoDetectRequest::bw_start_connect_time(0x3333),
        AutoDetectRequest::bw_payload(0x3333, vec![0u8; 1024]),
    ] {
        output.clear();
        connector
            .step(
                &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&AutoDetectReqPdu::new(request)).unwrap()),
                arrival,
                &mut output,
            )
            .unwrap();
    }

    output.clear();
    let stop = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::bw_stop_connect_time(
        0x3333,
        vec![0u8; 512],
    )))
    .unwrap();
    connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, stop),
            arrival,
            &mut output,
        )
        .unwrap();

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 1, "a window that arrived in one read floors to 1 ms");
    assert_eq!(results.1, 1536, "every byte in the timed window is still counted");
}

/// A driver with no clock reports no arrival times, so no window is ever opened and
/// there is nothing to accumulate into. The wasm32 and FFI drivers are in this
/// position for the whole connection.
///
/// The server still blocks without a reply, so one is sent, but it claims no more
/// than this Stop's own payload. Counting the Payload messages here would pair a
/// full byte count with a `timeDelta` the client never measured, which yields a
/// bandwidth figure that grows with however much the server chose to send.
#[test]
fn connect_time_bandwidth_without_a_clock_reports_the_stop_payload_alone() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    for request in [
        AutoDetectRequest::bw_start_connect_time(0x5555),
        AutoDetectRequest::bw_payload(0x5555, vec![0u8; 1024]),
        AutoDetectRequest::bw_stop_connect_time(0x5555, vec![0u8; 512]),
    ] {
        output.clear();
        connector
            .step(
                &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&AutoDetectReqPdu::new(request)).unwrap()),
                None,
                &mut output,
            )
            .unwrap();
    }

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 1, "a driver with no clock measured no interval");
    assert_eq!(
        results.1, 512,
        "the 1024-byte Payload is not counted, since no window was open to count it"
    );
}

/// Payload messages accumulate across a timed window, and the total they reach is
/// what the Stop reports. This is the path the floor case deliberately skips, so it
/// needs its own coverage: without it, nothing would catch an accumulator that had
/// stopped adding.
#[test]
fn connect_time_bandwidth_measured_window_reports_every_payload() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    // Each message lands in its own read, a millisecond apart, so the window is
    // timed and the accumulator is what decides the reported total.
    for (request, arrival) in [
        (AutoDetectRequest::bw_start_connect_time(0x4444), 2_000),
        (AutoDetectRequest::bw_payload(0x4444, vec![0u8; 2048]), 2_100),
        (AutoDetectRequest::bw_payload(0x4444, vec![0u8; 1024]), 2_200),
        (AutoDetectRequest::bw_stop_connect_time(0x4444, vec![0u8; 512]), 2_250),
    ] {
        output.clear();
        connector
            .step(
                &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&AutoDetectReqPdu::new(request)).unwrap()),
                Some(MonotonicInstant::from_millis(arrival)),
                &mut output,
            )
            .unwrap();
    }

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 250, "interval is Stop arrival minus Start arrival");
    assert_eq!(
        results.1, 3584,
        "both Payload messages and the Stop payload are counted"
    );
}

#[test]
fn connect_time_bandwidth_second_start_discards_the_first_window() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    // [MS-RDPBCGR] 3.2.5.14 has the client clear both stores and restart the
    // timer on each Bandwidth Measure Start, so the 4096 bytes counted into the
    // abandoned window must not survive into the reported total, and the
    // interval must be measured from the second Start rather than the first.
    for (request, arrival) in [
        (AutoDetectRequest::bw_start_connect_time(0x5555), 1_000),
        (AutoDetectRequest::bw_payload(0x5555, vec![0u8; 4096]), 1_100),
        (AutoDetectRequest::bw_start_connect_time(0x5555), 2_000),
        (AutoDetectRequest::bw_payload(0x5555, vec![0u8; 1024]), 2_100),
        (AutoDetectRequest::bw_stop_connect_time(0x5555, vec![0u8; 512]), 2_500),
    ] {
        output.clear();
        connector
            .step(
                &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&AutoDetectReqPdu::new(request)).unwrap()),
                Some(MonotonicInstant::from_millis(arrival)),
                &mut output,
            )
            .unwrap();
    }

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 500, "interval runs from the second Start, not the first");
    assert_eq!(
        results.1, 1536,
        "the 4096 bytes counted before the second Start are discarded"
    );
}
