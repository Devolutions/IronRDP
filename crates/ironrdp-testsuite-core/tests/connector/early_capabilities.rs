use ironrdp_connector::{ClientConnector, ClientConnectorState, Sequence as _};
use ironrdp_core::{WriteBuf, decode};
use ironrdp_pdu::gcc::ClientEarlyCapabilityFlags;
use ironrdp_pdu::mcs::ConnectInitial;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::x224::{X224, X224Data};

#[test]
fn dyn_vc_gfx_protocol_flag_matches_config() {
    let early_capability_flags = |enabled| {
        let mut config = super::test_config();
        config.support_dyn_vc_gfx_protocol = enabled;

        let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap());
        connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial {
            selected_protocol: SecurityProtocol::SSL,
        };

        let mut output = WriteBuf::new();
        connector.step(&[], None, &mut output).unwrap();

        let x224 = decode::<X224<X224Data<'_>>>(output.filled()).unwrap().0;
        let connect_initial = decode::<ConnectInitial>(x224.data.as_ref()).unwrap();

        connect_initial
            .conference_create_request
            .into_gcc_blocks()
            .core
            .optional_data
            .early_capability_flags
            .expect("the connector emits early capability flags")
    };

    let flag = ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL;
    let without = early_capability_flags(false);
    let with = early_capability_flags(true);

    assert!(!without.contains(flag));
    assert_eq!(with, without | flag);
}
