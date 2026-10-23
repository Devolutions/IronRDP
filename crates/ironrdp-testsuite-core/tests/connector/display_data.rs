use ironrdp_connector::{ClientConnector, ClientConnectorState, Sequence as _};
use ironrdp_core::{WriteBuf, decode};
use ironrdp_pdu::gcc::{ClientGccBlocks, MonitorOrientation};
use ironrdp_pdu::mcs::ConnectInitial;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::x224::{X224, X224Data};

fn gcc_blocks(config: ironrdp_connector::Config) -> ClientGccBlocks {
    let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial {
        selected_protocol: SecurityProtocol::SSL,
    };

    let mut output = WriteBuf::new();
    connector.step(&[], None, &mut output).unwrap();

    let x224 = decode::<X224<X224Data<'_>>>(output.filled()).unwrap().0;
    decode::<ConnectInitial>(x224.data.as_ref())
        .unwrap()
        .conference_create_request
        .into_gcc_blocks()
}

#[test]
fn core_data_defaults_derive_the_display_attributes() {
    let mut config = super::test_config();
    config.desktop_scale_factor = 125;

    let core = gcc_blocks(config).core.optional_data;
    assert_eq!(core.desktop_physical_width, Some(0));
    assert_eq!(core.desktop_physical_height, Some(0));
    assert_eq!(core.desktop_orientation, Some(MonitorOrientation::Landscape.as_u16()));
    assert_eq!(core.desktop_scale_factor, Some(125));
    assert_eq!(core.device_scale_factor, Some(100));
}

#[test]
fn core_data_carries_the_configured_display_attributes() {
    let mut config = super::test_config();
    config.desktop_scale_factor = 175;
    config.desktop_physical_size = Some((520, 290));
    config.desktop_orientation = Some(MonitorOrientation::PortraitFlipped);
    config.device_scale_factor = Some(180);

    let core = gcc_blocks(config).core.optional_data;
    assert_eq!(core.desktop_physical_width, Some(520));
    assert_eq!(core.desktop_physical_height, Some(290));
    assert_eq!(
        core.desktop_orientation,
        Some(MonitorOrientation::PortraitFlipped.as_u16())
    );
    assert_eq!(core.device_scale_factor, Some(180));
}

// oracle: spec MS-RDPBCGR 2.2.1.3.2
/// [MS-RDPBCGR] 2.2.1.3.2 says servers ignore `deviceScaleFactor` unless `desktopScaleFactor` is 100
/// through 500. In that case the connector keeps sending its existing placeholder of 0 instead of the
/// configured value.
#[test]
fn device_scale_factor_is_zero_without_a_valid_desktop_scale_factor() {
    let mut config = super::test_config();
    config.desktop_scale_factor = 0;
    config.device_scale_factor = Some(140);

    assert_eq!(gcc_blocks(config).core.optional_data.device_scale_factor, Some(0));
}

#[test]
fn relative_mouse_input_is_advertised_only_when_enabled() {
    use ironrdp_pdu::gcc::{ClientEarlyCapabilityFlags, RdpVersion};

    // oracle: spec MS-RDPBCGR 2.2.1.3.2 (servers ignore the flag below version 0x00080011)
    let advertised = |config: ironrdp_connector::Config| {
        let core = gcc_blocks(config).core;
        let flag = core
            .optional_data
            .early_capability_flags
            .expect("early capability flags are present")
            .contains(ClientEarlyCapabilityFlags::RELATIVE_MOUSE_INPUT);
        (flag, core.version)
    };

    assert_eq!(advertised(super::test_config()), (false, RdpVersion::V5_PLUS));
    let mut config = super::test_config();
    config.relative_mouse_input = true;
    assert_eq!(advertised(config), (true, RdpVersion::V10_12));
}
