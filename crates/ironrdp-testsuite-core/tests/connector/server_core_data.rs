use std::borrow::Cow;

use ironrdp_connector::{ClientConnector, ClientConnectorState, Sequence as _};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::gcc::{ConferenceCreateResponse, RdpVersion};
use ironrdp_pdu::mcs::ConnectResponse;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_testsuite_core::gcc::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS;
use ironrdp_testsuite_core::mcs::CONNECT_RESPONSE;
use ironrdp_testsuite_core::security_data::SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS;
use rstest::rstest;

/// A connector that has sent its MCS Connect Initial and waits for the server's answer.
fn connector_awaiting_connect_response() -> ClientConnector {
    let mut connector = ClientConnector::new(super::test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial {
        selected_protocol: SecurityProtocol::SSL,
    };

    let mut output = WriteBuf::new();
    connector.step(&[], None, &mut output).unwrap();
    connector
}

// oracle: spec MS-RDPBCGR 2.2.1.4.2 (the version is only known from the Server Core Data)
#[test]
fn server_rdp_version_is_none_before_server_core_data() {
    let connector = connector_awaiting_connect_response();

    assert!(connector.server_rdp_version.is_none());
}

// oracle: spec MS-RDPBCGR 2.2.1.4.2 (the version the test writes into the Server Core Data)
#[rstest]
#[case::v5_plus(RdpVersion::V5_PLUS)]
#[case::v10_7(RdpVersion::V10_7)]
fn server_rdp_version_comes_from_server_core_data(#[case] version: RdpVersion) {
    let mut connector = connector_awaiting_connect_response();

    let connect_response = {
        let mut gcc_blocks = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
        gcc_blocks.core.version = version;
        gcc_blocks.security = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS.clone();

        let response = ConnectResponse {
            conference_create_response: ConferenceCreateResponse::new(0x79f3, gcc_blocks).unwrap(),
            ..CONNECT_RESPONSE.clone()
        };

        encode_vec(&X224(X224Data {
            data: Cow::Owned(encode_vec(&response).unwrap()),
        }))
        .unwrap()
    };

    let mut output = WriteBuf::new();
    connector.step(&connect_response, None, &mut output).unwrap();

    assert!(matches!(
        connector.state,
        ClientConnectorState::ChannelConnection { .. }
    ));
    assert_eq!(connector.server_rdp_version, Some(version));
}
