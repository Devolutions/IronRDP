//! Connect-time auto-detection demux in the client connector.
//!
//! The continuous (session) auto-detect path is covered in
//! `tests/session/autodetect.rs`. These tests cover the connector's
//! `ConnectTimeAutoDetection` state, which demultiplexes the first PDU received
//! once a message channel has been negotiated: an Auto-Detect Request on the
//! message channel is answered, any other message-channel PDU is ignored, and a
//! PDU on the I/O channel is the first licensing PDU.
//!
//! The phases after it (licensing, multitransport bootstrapping, capabilities
//! exchange, connection finalization) wait for their PDUs on the I/O channel,
//! and a message-channel PDU that reaches one of them is demultiplexed the same
//! way: an Auto-Detect Request is answered, anything else is ignored, and the
//! phase goes on waiting. Windows Server with RDS licensing sends such requests
//! in the middle of the licensing exchange.

use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_connector::connection_activation::ConnectionActivationSequence;
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorResult, LicenseCache, LicenseExchangeSequence,
    LicenseExchangeState, MonotonicInstant, Sequence as _, Written,
};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu};
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags, ShareControlHeader, ShareControlPdu};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, RequestedProtocol};
use ironrdp_pdu::rdp::server_license::{
    LicenseErrorCode, LicenseHeader, LicenseInformation, LicensePdu, LicensingErrorMessage, LicensingStateTransition,
    PreambleFlags, PreambleType, PreambleVersion,
};
use ironrdp_pdu::x224::X224;
use ironrdp_testsuite_core::capsets::SERVER_DEMAND_ACTIVE;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const MESSAGE_CHANNEL_ID: u16 = 1004;
const SHARE_ID: u32 = 0x0001_0000;

/// A client connector parked in `ConnectTimeAutoDetection` with a negotiated
/// message channel, ready to receive the first PDU of that phase.
fn connect_time_autodetect_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(super::test_config(), "127.0.0.1:12345".parse().unwrap());
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
    let mut output = WriteBuf::new();
    connector
        .step(&license_status_valid_client(), None, &mut output)
        .unwrap();

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
    assert_eq!(
        results.1, 1552,
        "every byte in the timed window is still counted, plus the 8-byte header on each of the Payload and Stop"
    );
}

/// A window can open on one driver and close on another, for example when a
/// `Framed` is rebuilt with leftover bytes between the Start and this Stop: the
/// rebuilt `Framed` starts with no arrival time of its own, so the Stop reports
/// `None` even though the Start that opened the window reported `Some`. Nothing
/// upstream of this function stops that from happening, so it is reachable even
/// though it did not use to be exercised by a test.
///
/// The window's accumulated bytes are dropped in that case: reporting them
/// against `timeDelta = UNMEASURABLE_INTERVAL_MS` would pair a byte count that
/// arrived over the window's real duration with a floor timer that understates
/// it, inflating the reported bandwidth the same way an uncounted header would.
#[test]
fn connect_time_bandwidth_stop_with_no_arrival_time_drops_the_open_window() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    let arrival = Some(MonotonicInstant::from_millis(9_000));

    for request in [
        AutoDetectRequest::bw_start_connect_time(0x7777),
        AutoDetectRequest::bw_payload(0x7777, vec![0u8; 2048]),
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
        0x7777,
        vec![0u8; 512],
    )))
    .unwrap();
    connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, stop),
            None,
            &mut output,
        )
        .unwrap();

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 1, "an unmeasured window still floors to 1 ms");
    assert_eq!(
        results.1, 520,
        "the 2048-byte Payload is dropped along with the window; only the Stop's own 512 bytes plus its 8-byte header remain"
    );
}

/// A driver with no clock reports no arrival time for this step, so no window is
/// ever opened and there is nothing to accumulate into. The FFI driver is in this
/// position for the whole connection; the wasm32 driver only for the single
/// x224_connection_response step.
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
        results.1, 520,
        "the 1024-byte Payload is not counted, since no window was open; the Stop's own 512 bytes plus its 8-byte header are"
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
        results.1, 3608,
        "both Payload messages and the Stop payload are counted, each with its 8-byte header"
    );
}

/// Continuous detection is not this phase's business. Its Starts and Stops belong
/// to a different procedure in [MS-RDPBCGR] 3.2.5.14, answered on a multitransport
/// channel, and the 0x0629 Stop needs a sequence-number correlation this phase
/// does not track. A continuous Start must therefore not open a connect-time
/// window, and a continuous Stop must not be answered as though it had.
///
/// The header size follows the same split: 2.2.14.1.4 sets `headerLength` to 0x08
/// only for the 0x002B Stop and 0x06 otherwise, so counting a continuous Stop with
/// the connect-time addend would be wrong on top of replying to it at all.
#[test]
fn continuous_bandwidth_measure_requests_are_not_answered_as_connect_time() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    for request in [
        AutoDetectRequest::bw_start_continuous(0x6666),
        AutoDetectRequest::bw_payload(0x6666, vec![0u8; 1024]),
        AutoDetectRequest::bw_stop_continuous(0x6666),
    ] {
        output.clear();
        let written = connector
            .step(
                &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&AutoDetectReqPdu::new(request)).unwrap()),
                Some(MonotonicInstant::from_millis(3_000)),
                &mut output,
            )
            .unwrap();

        assert_eq!(
            written,
            Written::Nothing,
            "a continuous-detection request produces no connect-time reply"
        );
    }

    assert!(
        matches!(connector.state, ClientConnectorState::ConnectTimeAutoDetection { .. }),
        "the connector keeps listening after ignoring continuous-detection requests"
    );
}

/// A connect-time Stop arriving after a continuous Start has no window to close,
/// because the continuous Start did not open one. It is still answered, since the
/// server blocks without a reply, but with the untimed floor rather than a figure
/// derived from a window that was never opened.
#[test]
fn connect_time_stop_after_a_continuous_start_reports_the_floor() {
    let mut connector = connect_time_autodetect_connector();
    let mut output = WriteBuf::new();

    for (request, at) in [
        (AutoDetectRequest::bw_start_continuous(0x7777), 4_000),
        (AutoDetectRequest::bw_payload(0x7777, vec![0u8; 2048]), 4_100),
        (AutoDetectRequest::bw_stop_connect_time(0x7777, vec![0u8; 512]), 4_250),
    ] {
        output.clear();
        connector
            .step(
                &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&AutoDetectReqPdu::new(request)).unwrap()),
                Some(MonotonicInstant::from_millis(at)),
                &mut output,
            )
            .unwrap();
    }

    let results = decode_bandwidth_results(&output);
    assert_eq!(results.0, 1, "no connect-time window was ever opened");
    assert_eq!(
        results.1, 520,
        "only the Stop is counted, with its 8 header bytes, and the ignored Payload is not"
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
        results.1, 1552,
        "the 4096 bytes counted before the second Start are discarded; only the second window's Payload and Stop, each with an 8-byte header, remain"
    );
}

/// A STATUS_VALID_CLIENT license error on the I/O channel, which completes
/// licensing in a single step ([MS-RDPELE] 3.1.5.3.1).
fn license_status_valid_client() -> Vec<u8> {
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

    server_send_data_indication(IO_CHANNEL_ID, encode_vec(&license).unwrap())
}

/// A connect-time RTT Measure Request on the message channel.
fn rtt_request(sequence_number: u16) -> Vec<u8> {
    let request = AutoDetectReqPdu::new(AutoDetectRequest::rtt_connect_time(sequence_number));

    server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&request).unwrap())
}

/// The whole of `output` is the RTT Measure Response to `sequence_number`, on the
/// message channel.
fn assert_rtt_response(output: &WriteBuf, sequence_number: u16) {
    let X224(McsMessage::SendDataRequest(send_data)) = ironrdp_core::decode(output.filled()).unwrap() else {
        panic!("expected a SendDataRequest in the response frame");
    };
    assert_eq!(
        send_data.channel_id, MESSAGE_CHANNEL_ID,
        "the response goes out on the message channel"
    );

    let response = ironrdp_core::decode::<AutoDetectRspPdu>(&send_data.user_data).unwrap();
    assert_eq!(response.response, AutoDetectResponse::RttResponse { sequence_number });
}

/// A license cache that holds nothing, for a licensing sequence under test.
#[derive(Debug)]
struct NoLicense;

impl LicenseCache for NoLicense {
    fn get_license(&self, _license_info: LicenseInformation) -> ConnectorResult<Option<Vec<u8>>> {
        Ok(None)
    }

    fn store_license(&self, _license_info: LicenseInformation) -> ConnectorResult<()> {
        Ok(())
    }
}

/// A client connector that has just entered the licensing exchange, with a
/// negotiated message channel.
fn licensing_exchange_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(super::test_config(), "127.0.0.1:12345".parse().unwrap());
    connector.state = ClientConnectorState::LicensingExchange {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        license_exchange: LicenseExchangeSequence::new(
            IO_CHANNEL_ID,
            "test".to_owned(),
            None,
            [0; 4],
            Arc::new(NoLicense),
        ),
    };
    connector.message_channel_id = Some(MESSAGE_CHANNEL_ID);
    connector
}

/// A client connector that has completed licensing and is waiting for a
/// multitransport request or the Demand Active.
fn multitransport_bootstrapping_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(super::test_config(), "127.0.0.1:12345".parse().unwrap());
    connector.state = ClientConnectorState::MultitransportBootstrapping {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        message_channel_id: Some(MESSAGE_CHANNEL_ID),
        requests_seen: 0,
    };
    connector.message_channel_id = Some(MESSAGE_CHANNEL_ID);
    connector
}

/// A client connector waiting for the Demand Active.
fn capabilities_exchange_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(super::test_config(), "127.0.0.1:12345".parse().unwrap());
    connector.state = ClientConnectorState::CapabilitiesExchange {
        connection_activation: ConnectionActivationSequence::new(super::test_config(), IO_CHANNEL_ID, USER_CHANNEL_ID),
    };
    connector.message_channel_id = Some(MESSAGE_CHANNEL_ID);
    connector
}

/// The Demand Active, on the I/O channel, that ends the capabilities exchange.
fn server_demand_active() -> Vec<u8> {
    let header = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()),
        pdu_source: USER_CHANNEL_ID,
        share_id: SHARE_ID,
    };

    server_send_data_indication(IO_CHANNEL_ID, encode_vec(&header).unwrap())
}

/// A client connector that has answered the Demand Active and is finalizing the
/// connection.
fn connection_finalization_connector() -> ClientConnector {
    let mut connector = capabilities_exchange_connector();
    let mut output = WriteBuf::new();
    connector.step(&server_demand_active(), None, &mut output).unwrap();
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectionFinalization { .. }),
        "the Demand Active starts connection finalization"
    );
    connector
}

/// Windows Server with RDS licensing sends Auto-Detect Requests in the middle of
/// the licensing exchange, which runs to its full length there (a new license
/// requested, a platform challenge answered, the license upgraded), where a host
/// without licensing ends it with one error message. The licensing sequence
/// decodes licensing PDUs only, and the first such request used to end the
/// connection with "invalid security header flags".
#[test]
fn licensing_exchange_answers_an_autodetect_request_and_keeps_waiting() {
    let mut connector = licensing_exchange_connector();
    let mut output = WriteBuf::new();

    let written = connector.step(&rtt_request(0x0001), None, &mut output).unwrap();

    assert!(written.size().is_some(), "the request is answered");
    assert_rtt_response(&output, 0x0001);
    assert!(
        matches!(
            &connector.state,
            ClientConnectorState::LicensingExchange { license_exchange, .. }
                if matches!(license_exchange.state, LicenseExchangeState::NewLicenseRequest)
        ),
        "licensing goes on waiting for its PDU, where it was"
    );

    // The licensing PDU that follows is handled as before.
    output.clear();
    connector
        .step(&license_status_valid_client(), None, &mut output)
        .unwrap();

    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportBootstrapping { .. }
        ),
        "the licensing PDU completes the exchange"
    );
}

/// The PDU an RDS host mixed into licensing, byte for byte (the trace in upstream
/// IronRDP issue #1629): a Bandwidth Measure Start of the reliable UDP kind
/// (0x0014), which the connect-time responder leaves alone. It is still no
/// licensing PDU, so it must not end the connection.
#[test]
fn licensing_exchange_tolerates_the_autodetect_request_an_rds_host_mixes_in() {
    const SERVER_PDU: [u8; 10] = [
        0x00, 0x10, // flags: SEC_AUTODETECT_REQ
        0x00, 0x00, // flagsHi
        0x06, // headerLength
        0x00, // headerTypeId: TYPE_ID_AUTODETECT_REQUEST
        0x00, 0x00, // sequenceNumber
        0x14, 0x00, // requestType: Bandwidth Measure Start (reliable UDP)
    ];

    let mut connector = licensing_exchange_connector();
    let mut output = WriteBuf::new();

    let written = connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, SERVER_PDU.to_vec()),
            Some(MonotonicInstant::from_millis(1_000)),
            &mut output,
        )
        .unwrap();

    assert_eq!(
        written,
        Written::Nothing,
        "a reliable UDP Start opens no connect-time window"
    );
    assert!(
        matches!(connector.state, ClientConnectorState::LicensingExchange { .. }),
        "licensing goes on waiting for its PDU"
    );
}

#[test]
fn licensing_exchange_ignores_other_message_channel_pdus() {
    let mut connector = licensing_exchange_connector();
    let mut output = WriteBuf::new();

    let user_data = encode_vec(&BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::HEARTBEAT,
    })
    .unwrap();
    let written = connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, user_data),
            None,
            &mut output,
        )
        .unwrap();

    assert_eq!(
        written,
        Written::Nothing,
        "an unrelated message-channel PDU produces no response"
    );
    assert!(
        matches!(connector.state, ClientConnectorState::LicensingExchange { .. }),
        "licensing goes on waiting for its PDU"
    );
}

#[test]
fn licensing_exchange_ignores_a_malformed_autodetect_request() {
    let mut connector = licensing_exchange_connector();
    let mut output = WriteBuf::new();

    // SEC_AUTODETECT_REQ with no Auto-Detect Request after it. Auto-detection is
    // advisory, so this is ignored as during the connect-time phase rather than
    // ending the connection.
    let user_data = encode_vec(&BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::AUTODETECT_REQ,
    })
    .unwrap();
    let written = connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, user_data),
            None,
            &mut output,
        )
        .unwrap();

    assert_eq!(written, Written::Nothing, "a malformed request produces no response");
    assert!(
        matches!(connector.state, ClientConnectorState::LicensingExchange { .. }),
        "licensing goes on waiting for its PDU"
    );
}

#[test]
fn multitransport_bootstrapping_ignores_other_message_channel_pdus() {
    let mut connector = multitransport_bootstrapping_connector();
    let mut output = WriteBuf::new();

    let user_data = encode_vec(&BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::HEARTBEAT,
    })
    .unwrap();
    let written = connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, user_data),
            None,
            &mut output,
        )
        .unwrap();

    assert_eq!(
        written,
        Written::Nothing,
        "an unrelated message-channel PDU produces no response"
    );
    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportBootstrapping { requests_seen: 0, .. }
        ),
        "the phase goes on waiting, and the PDU is not counted as a multitransport request"
    );
}

#[test]
fn multitransport_bootstrapping_answers_an_autodetect_request_and_keeps_waiting() {
    let mut connector = multitransport_bootstrapping_connector();
    let mut output = WriteBuf::new();

    let written = connector.step(&rtt_request(0x0002), None, &mut output).unwrap();

    assert!(written.size().is_some(), "the request is answered");
    assert_rtt_response(&output, 0x0002);
    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportBootstrapping { requests_seen: 0, .. }
        ),
        "the phase goes on waiting, and the request is not counted as a multitransport request"
    );

    // A multitransport request that follows is surfaced as before.
    let request = MultitransportRequestPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
        },
        request_id: 1,
        requested_protocol: RequestedProtocol::UdpFecR,
        security_cookie: [0u8; 16],
    };
    output.clear();
    connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, encode_vec(&request).unwrap()),
            None,
            &mut output,
        )
        .unwrap();

    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportPending { requests_seen: 1, .. }
        ),
        "the multitransport request is surfaced and counted"
    );
}

#[test]
fn capabilities_exchange_answers_an_autodetect_request_and_keeps_waiting() {
    let mut connector = capabilities_exchange_connector();
    let mut output = WriteBuf::new();

    let written = connector.step(&rtt_request(0x0003), None, &mut output).unwrap();

    assert!(written.size().is_some(), "the request is answered");
    assert_rtt_response(&output, 0x0003);
    assert!(
        matches!(connector.state, ClientConnectorState::CapabilitiesExchange { .. }),
        "the phase goes on waiting for the Demand Active"
    );

    // The Demand Active that follows is handled as before.
    output.clear();
    connector.step(&server_demand_active(), None, &mut output).unwrap();

    assert!(
        matches!(connector.state, ClientConnectorState::ConnectionFinalization { .. }),
        "the Demand Active starts connection finalization"
    );
}

#[test]
fn connection_finalization_answers_an_autodetect_request_and_keeps_waiting() {
    let mut connector = connection_finalization_connector();
    let mut output = WriteBuf::new();

    let written = connector.step(&rtt_request(0x0004), None, &mut output).unwrap();

    assert!(written.size().is_some(), "the request is answered");
    assert_rtt_response(&output, 0x0004);
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectionFinalization { .. }),
        "the phase goes on waiting for its PDUs"
    );
}

/// A connect-time bandwidth measurement that straddles phases is still one
/// measurement: the window opened during licensing is the one the Stop closes
/// after it.
#[test]
fn connect_time_bandwidth_measurement_spans_licensing_and_what_follows() {
    let mut connector = licensing_exchange_connector();
    let mut output = WriteBuf::new();

    for (request, arrival) in [
        (AutoDetectRequest::bw_start_connect_time(0x8888), 1_000),
        (AutoDetectRequest::bw_payload(0x8888, vec![0u8; 1024]), 1_100),
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

    output.clear();
    connector
        .step(
            &license_status_valid_client(),
            Some(MonotonicInstant::from_millis(1_200)),
            &mut output,
        )
        .unwrap();
    assert!(matches!(
        connector.state,
        ClientConnectorState::MultitransportBootstrapping { .. }
    ));

    output.clear();
    let stop = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::bw_stop_connect_time(
        0x8888,
        vec![0u8; 512],
    )))
    .unwrap();
    connector
        .step(
            &server_send_data_indication(MESSAGE_CHANNEL_ID, stop),
            Some(MonotonicInstant::from_millis(1_500)),
            &mut output,
        )
        .unwrap();

    let results = decode_bandwidth_results(&output);
    assert_eq!(
        results.0, 500,
        "the interval runs from the Start received during licensing"
    );
    assert_eq!(
        results.1, 1552,
        "the Payload received during licensing and the Stop are counted, each with its 8-byte header"
    );
}
