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

fn connect_response_announcing(version: RdpVersion) -> Vec<u8> {
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
}

fn server_rdp_version_after_basic_settings(version: RdpVersion) -> Option<RdpVersion> {
    let mut connector = ClientConnector::new(super::test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial {
        selected_protocol: SecurityProtocol::SSL,
    };

    let mut output = WriteBuf::new();
    connector.step(&[], None, &mut output).unwrap();
    assert!(connector.server_rdp_version.is_none());

    output.clear();
    connector
        .step(&connect_response_announcing(version), None, &mut output)
        .unwrap();
    assert!(matches!(
        connector.state,
        ClientConnectorState::ChannelConnection { .. }
    ));

    connector.server_rdp_version
}

#[test]
fn server_rdp_version_comes_from_server_core_data() {
    assert_eq!(
        server_rdp_version_after_basic_settings(RdpVersion::V5_PLUS),
        Some(RdpVersion::V5_PLUS)
    );
    assert_eq!(
        server_rdp_version_after_basic_settings(RdpVersion::V10_7),
        Some(RdpVersion::V10_7)
    );
}
