use ironrdp_client::__test_build_connector;
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_connector::NetworkCharacteristics;
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, NETCHAR_RESULT_ALL};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;

// oracle: regression Devolutions/IronRDP#2437
#[test]
fn connect_time_network_characteristics_keep_the_latest_result() {
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
    let results = [
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
    ];

    let mut network_characteristics = None;
    for result in results {
        network_characteristics = connector
            .__test_respond_to_connect_time_autodetect(result)
            .expect("retain the server result");
    }

    assert_eq!(
        network_characteristics,
        Some(NetworkCharacteristics {
            base_rtt_ms: Some(5),
            bandwidth_kbps: Some(8_000),
            average_rtt_ms: 6,
        })
    );
}
