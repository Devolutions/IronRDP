use ironrdp_client::__test_build_connector;
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_connector::NetworkCharacteristics;
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, NETCHAR_RESULT_ALL, NETCHAR_RESULT_BW_RTT, NETCHAR_RESULT_RTT};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;

/// Feeds `results` to a connector during connection setup and returns the measurement it keeps.
fn connect_time_measurement(results: Vec<AutoDetectRequest>) -> Option<NetworkCharacteristics> {
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("network-characteristics.invalid", 3389))
        .with_username("user")
        .with_password(String::new())
        .with_client_build(1)
        .with_client_dir(r"C:\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("client")
        .build()
        .expect("client configuration");
    let mut connector = __test_build_connector(&config, None).expect("build connector");

    let mut network_characteristics = None;
    for result in results {
        network_characteristics = connector
            .__test_respond_to_connect_time_autodetect(result)
            .expect("retain the server result");
    }
    network_characteristics
}

// oracle: mutant a connector that keeps the first connect-time result rather than the latest
#[test]
fn connect_time_network_characteristics_keep_the_latest_result() {
    let network_characteristics = connect_time_measurement(vec![
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 1,
            request_type: NETCHAR_RESULT_ALL,
            base_rtt_ms: Some(3),
            bandwidth_kbps: Some(10_000),
            average_rtt_ms: 4,
        },
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 2,
            request_type: NETCHAR_RESULT_ALL,
            base_rtt_ms: Some(5),
            bandwidth_kbps: Some(8_000),
            average_rtt_ms: 6,
        },
    ]);

    assert_eq!(
        network_characteristics,
        Some(NetworkCharacteristics {
            base_rtt_ms: Some(5),
            bandwidth_kbps: Some(8_000),
            average_rtt_ms: 6,
        })
    );
}

// oracle: mutant a measurement that a result without baseRTT or bandwidth replaces whole
#[test]
fn connect_time_network_characteristics_keep_what_a_partial_result_leaves_out() {
    let network_characteristics = connect_time_measurement(vec![
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 1,
            request_type: NETCHAR_RESULT_RTT,
            base_rtt_ms: Some(3),
            bandwidth_kbps: None,
            average_rtt_ms: 4,
        },
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 2,
            request_type: NETCHAR_RESULT_BW_RTT,
            base_rtt_ms: None,
            bandwidth_kbps: Some(8_000),
            average_rtt_ms: 6,
        },
    ]);

    assert_eq!(
        network_characteristics,
        Some(NetworkCharacteristics {
            base_rtt_ms: Some(3),
            bandwidth_kbps: Some(8_000),
            average_rtt_ms: 6,
        })
    );
}

// oracle: mutant a measurement that drops the bandwidth a later result without one leaves out
#[test]
fn connect_time_network_characteristics_keep_the_bandwidth_a_later_rtt_only_result_leaves_out() {
    let network_characteristics = connect_time_measurement(vec![
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 1,
            request_type: NETCHAR_RESULT_ALL,
            base_rtt_ms: Some(3),
            bandwidth_kbps: Some(10_000),
            average_rtt_ms: 4,
        },
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 2,
            request_type: NETCHAR_RESULT_RTT,
            base_rtt_ms: Some(2),
            bandwidth_kbps: None,
            average_rtt_ms: 5,
        },
    ]);

    assert_eq!(
        network_characteristics,
        Some(NetworkCharacteristics {
            base_rtt_ms: Some(2),
            bandwidth_kbps: Some(10_000),
            average_rtt_ms: 5,
        })
    );
}
