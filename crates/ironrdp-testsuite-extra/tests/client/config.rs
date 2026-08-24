use std::fs;
use std::path::PathBuf;

use std::sync::Arc;

use ironrdp_cfg::PropertySetExt as _;
use ironrdp_client::config::{AudioQualityMode, ClipboardType, ConfigBuilder, Destination, Transport, VmConnectMode};
use ironrdp_pdu::gcc::{ClientMonitorData, Monitor, MonitorFlags};
use ironrdp_pdu::nego::NegoRequestData;
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_viewer::cli::parse_config_from;
use uuid::Uuid;

struct TempRdpFile {
    path: PathBuf,
}

impl TempRdpFile {
    fn new(content: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ironrdp-client-rdp-{}.rdp", Uuid::new_v4()));
        fs::write(&path, content).expect("failed to write temporary .rdp file");
        TempRdpFile { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempRdpFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn parse_config_from_rdp_result(content: &str, extra_args: &[&str]) -> anyhow::Result<ironrdp_client::config::Config> {
    let rdp_file = TempRdpFile::new(content);

    let mut args = vec![
        "ironrdp-client".to_owned(),
        "--rdp-file".to_owned(),
        rdp_file.path().display().to_string(),
    ];

    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));

    parse_config_from(args)
}

fn parse_config_from_rdp(content: &str, extra_args: &[&str]) -> ironrdp_client::config::Config {
    parse_config_from_rdp_result(content, extra_args).expect("failed to parse client config")
}

#[test]
fn gateway_is_disabled_when_gateway_usage_method_is_zero() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:0\n",
        &[],
    );

    assert!(!matches!(config.transport(), Transport::Gateway(_)));
}

#[test]
fn gateway_is_disabled_when_gateway_usage_method_is_four() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:4\n",
        &[],
    );

    assert!(!matches!(config.transport(), Transport::Gateway(_)));
}

#[test]
fn gateway_is_enabled_with_usage_method_one_and_file_credentials() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:1\ngatewayusername:s:gw-user\nGatewayPassword:s:gw-pass\n",
        &[],
    );

    let Transport::Gateway(gw) = config.transport() else {
        panic!("gateway should be configured");
    };
    assert_eq!(gw.endpoint, "gw.example.com:443");
    assert_eq!(gw.username, "gw-user");
    assert_eq!(gw.password, "gw-pass");
    assert!(!gw.prefer_direct);
}

#[test]
fn gateway_detect_prefers_direct_when_hostname_is_set() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:2\ngatewayusername:s:gw-user\nGatewayPassword:s:gw-pass\n",
        &[],
    );

    let Transport::Gateway(gw) = config.transport() else {
        panic!("gateway should be configured");
    };
    assert_eq!(gw.endpoint, "gw.example.com:443");
    assert!(gw.prefer_direct);
}

#[test]
fn no_credssp_cli_flag_overrides_rdp_enable_credssp_property() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nenablecredsspsupport:i:1\n",
        &["--no-credssp"],
    );

    assert!(!config.connector().enable_credssp);
}

#[test]
fn bare_destination_keeps_the_ordinary_rdp_default() {
    let bare = Destination::new("rdp.example.com").expect("valid bare destination");
    let explicit = Destination::new("rdp.example.com:3389").expect("valid explicit destination");

    assert_eq!(bare.port(), 3389);
    assert_eq!(bare, explicit);
}

#[test]
fn vmconnect_uses_enhanced_mode_by_default() {
    let config = parse_config_from_rdp(
        "full address:s:hyperv.example.com:2179\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &["--vmconnect", "efd1efab-c750-4262-b1bb-af0f7733bdd6"],
    );

    assert_eq!(config.vmconnect_mode(), Some(VmConnectMode::Enhanced));
}

#[test]
fn vmconnect_bare_destination_defaults_to_port_2179() {
    let config = parse_config_from([
        "ironrdp-viewer",
        "-u",
        "test-user",
        "-p",
        "test-pass",
        "--vmconnect",
        "efd1efab-c750-4262-b1bb-af0f7733bdd6",
        "hyperv.example.com",
    ])
    .expect("valid vmconnect configuration");

    assert_eq!(config.destination().port(), 2179);
}

#[test]
fn vmconnect_preserves_explicit_destination_port() {
    for port in [3389, 12_345] {
        let destination = format!("hyperv.example.com:{port}");
        let config = parse_config_from([
            "ironrdp-viewer",
            "-u",
            "test-user",
            "-p",
            "test-pass",
            "--vmconnect",
            "efd1efab-c750-4262-b1bb-af0f7733bdd6",
            &destination,
        ])
        .expect("valid vmconnect configuration");

        assert_eq!(config.destination().port(), port);
    }
}

#[test]
fn vmconnect_basic_flag_selects_basic_mode() {
    let config = parse_config_from_rdp(
        "full address:s:hyperv.example.com:2179\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &[
            "--vmconnect",
            "efd1efab-c750-4262-b1bb-af0f7733bdd6",
            "--vmconnect-basic",
        ],
    );

    assert_eq!(config.vmconnect_mode(), Some(VmConnectMode::Basic));
}

#[test]
fn vmconnect_accepts_rds_gateway() {
    // The gateway channel-create now forwards the destination port, so
    // VMConnect (port 2179) can be tunneled through an RD Gateway.
    let config = parse_config_from_rdp(
        "full address:s:hyperv.example.com:2179\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &[
            "--vmconnect",
            "efd1efab-c750-4262-b1bb-af0f7733bdd6",
            "--gw-endpoint",
            "gw.example.com:443",
            "--gw-user",
            "gw-user",
            "--gw-pass",
            "gw-pass",
        ],
    );

    assert!(matches!(config.transport(), Transport::Gateway(_)));
    assert_eq!(config.destination().port(), 2179);
}

#[test]
fn vmconnect_rejects_disabled_security() {
    for disabled_security in ["--no-tls", "--no-credssp"] {
        let err = parse_config_from_rdp_result(
            "full address:s:hyperv.example.com:2179\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
            &["--vmconnect", "efd1efab-c750-4262-b1bb-af0f7733bdd6", disabled_security],
        )
        .expect_err("vmconnect security requirements must fail during configuration");

        assert!(
            err.to_string().contains("requires"),
            "unexpected error for {disabled_security}: {err:#}"
        );
    }
}

#[test]
fn kdc_proxy_name_is_normalized_to_https_url() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nkdcproxyname:s:kdc.example.com\n",
        &[],
    );

    let kerberos = config.kerberos_config().expect("kerberos config should be present");
    let kdc_proxy_url = kerberos
        .kdc_proxy_url
        .as_ref()
        .expect("kdc proxy url should be present");
    assert_eq!(kdc_proxy_url.as_str(), "https://kdc.example.com/KdcProxy");
}

#[test]
fn redirectclipboard_zero_disables_clipboard_for_default_mode() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nredirectclipboard:i:0\n",
        &[],
    );

    assert!(matches!(config.channels().clipboard, ClipboardType::Disable));
}

#[test]
fn audiomode_two_disables_audio_playback() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\naudiomode:i:2\n",
        &[],
    );

    assert!(!config.connector().enable_audio_playback);
}

#[test]
fn audiomode_one_play_on_server_disables_local_playback() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\naudiomode:i:1\n",
        &[],
    );

    assert!(!config.connector().enable_audio_playback);
}

#[test]
fn audiomode_zero_enables_client_redirection() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\naudiomode:i:0\n",
        &[],
    );

    assert!(config.connector().enable_audio_playback);
}

#[test]
fn invalid_audiomode_falls_back_to_audio_playback_enabled() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\naudiomode:i:99\n",
        &[],
    );

    assert!(config.connector().enable_audio_playback);
}

#[test]
fn certificate_validation_preserves_the_default_and_callbacks_are_explicit() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &[],
    );
    assert_eq!(
        config.certificate_validation(),
        ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate
    );

    let callback: ironrdp_tls::CertificateValidationCallback = Arc::new(|certificate, server_name, reason| {
        certificate == b"test certificate" && server_name == "rdp.example.com" && reason == "untrusted issuer"
    });
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("rdp.example.com", 3389))
        .with_username("test-user")
        .with_password("test-pass")
        .with_client_build(1)
        .with_client_dir("C:\\Windows\\System32")
        .with_client_name("ironrdp-tests")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_certificate_validation_callback(Arc::clone(&callback))
        .build()
        .expect("valid callback configuration");
    assert_eq!(
        config.certificate_validation(),
        ironrdp_tls::CertificateValidation::Strict
    );
    let callback = config
        .certificate_validation_callback()
        .expect("certificate validation callback must be retained");
    assert!(callback(b"test certificate", "rdp.example.com", "untrusted issuer"));
    assert!(!callback(b"other certificate", "rdp.example.com", "untrusted issuer"));
}

#[test]
fn desktop_dimensions_are_parsed_from_rdp_file() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ndesktopwidth:i:1024\ndesktopheight:i:768\ndesktopscalefactor:i:125\n",
        &[],
    );

    assert_eq!(config.connector().desktop_size.width, 1024);
    assert_eq!(config.connector().desktop_size.height, 768);
    assert_eq!(config.connector().desktop_scale_factor, 125);
}

#[test]
fn generic_builder_options_reach_connector_configuration() {
    use ironrdp::pdu::gcc::ConnectionType;
    use ironrdp::pdu::rdp::client_info::PerformanceFlags;

    let performance_flags = PerformanceFlags::DISABLE_WALLPAPER | PerformanceFlags::DISABLE_THEMING;
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("rdp.example.com", 3389))
        .with_username("test-user")
        .with_password("test-pass")
        .with_client_build(1)
        .with_client_dir("C:\\Windows\\System32")
        .with_client_name("ironrdp-tests")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_keyboard_layout(0x0000_0409)
        .with_connection_type(ConnectionType::BroadbandHigh)
        .with_lossy_compression(false)
        .with_performance_flags(performance_flags)
        .with_alternate_shell("powershell.exe")
        .with_work_dir("C:\\Users\\test-user")
        .with_load_balance_info("tsv://MS Terminal Services Plugin.1.collection")
        .with_administrative_session(true)
        .with_audio_quality_mode(AudioQualityMode::Dynamic)
        .build()
        .expect("valid generic configuration");

    assert_eq!(config.connector().keyboard_layout, 0x0000_0409);
    assert_eq!(config.connector().connection_type, ConnectionType::BroadbandHigh);
    assert!(
        !config
            .connector()
            .bitmap
            .as_ref()
            .expect("bitmap config")
            .lossy_compression
    );
    assert_eq!(config.connector().performance_flags, performance_flags);
    assert_eq!(config.connector().alternate_shell, "powershell.exe");
    assert_eq!(config.connector().work_dir, "C:\\Users\\test-user");
    assert!(matches!(
        config.connector().request_data.as_ref(),
        Some(NegoRequestData::OpaqueRoutingToken(token))
            if token.0 == "tsv://MS Terminal Services Plugin.1.collection"
    ));
    assert!(config.administrative_session());
    assert_eq!(config.audio_quality_mode(), AudioQualityMode::Dynamic);
    assert_eq!(
        config.properties().get::<&str>("loadbalanceinfo"),
        Some("tsv://MS Terminal Services Plugin.1.collection")
    );
    assert_eq!(config.properties().get::<bool>("administrative session"), Some(true));
    assert_eq!(config.properties().get::<u32>("audioqualitymode"), Some(0));
}

#[test]
fn rdp_file_maps_routing_admin_and_audio_quality_settings() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\n\
         username:s:test-user\n\
         ClearTextPassword:s:test-pass\n\
         loadbalanceinfo:s:tsv://MS Terminal Services Plugin.1.collection\n\
         administrative session:i:1\n\
         audioqualitymode:i:1\n",
        &[],
    );

    assert!(matches!(
        config.connector().request_data.as_ref(),
        Some(NegoRequestData::OpaqueRoutingToken(token))
            if token.0 == "tsv://MS Terminal Services Plugin.1.collection"
    ));
    assert!(config.administrative_session());
    assert_eq!(config.audio_quality_mode(), AudioQualityMode::Medium);
}

#[test]
fn out_of_range_desktop_dimensions_fall_back_to_defaults() {
    let default_config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &[],
    );
    let invalid_config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ndesktopwidth:i:-1\ndesktopheight:i:-1\n",
        &[],
    );

    assert_eq!(
        invalid_config.connector().desktop_size.width,
        default_config.connector().desktop_size.width
    );
    assert_eq!(
        invalid_config.connector().desktop_size.height,
        default_config.connector().desktop_size.height
    );
}

fn complete_builder() -> ConfigBuilder {
    ConfigBuilder::new()
        .with_destination(Destination::new("server.example:3389").unwrap())
        .with_username("user")
        .with_password("password")
        .with_client_build(1)
        .with_client_dir("C:\\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("client")
}

#[test]
fn remote_application_mode_requires_remote_programs_support() {
    let error = complete_builder()
        .with_remote_application_mode(true)
        .with_rail_support_level(RailSupportLevel::empty())
        .build()
        .expect_err("RemoteApp must require remote programs support");

    assert!(error.to_string().contains("RAIL support level"), "{error:?}");
}

#[test]
fn remote_application_mode_is_preserved_in_properties() {
    let config = complete_builder()
        .with_remote_application_mode(true)
        .build()
        .expect("valid RemoteApp configuration");

    assert!(config.connector().remote_application_mode);
    assert_eq!(config.properties().remote_application_mode(), Some(true));
}

#[test]
fn monitor_layout_is_preserved_in_connector_configuration() {
    let monitor_layout = ClientMonitorData {
        monitors: vec![Monitor {
            left: 0,
            top: 0,
            right: 1_919,
            bottom: 1_079,
            flags: MonitorFlags::PRIMARY,
        }],
    };
    let config = complete_builder()
        .with_desktop_width(1_920)
        .with_desktop_height(1_080)
        .with_monitor_layout(monitor_layout.clone())
        .build()
        .expect("valid monitor layout configuration");

    assert_eq!(config.connector().monitor_layout.as_ref(), Some(&monitor_layout));
}

#[test]
fn with_audio_mode_redirect_enables_playback_channel() {
    use ironrdp_cfg::AudioMode;

    let config = complete_builder()
        .with_audio_mode(AudioMode::RedirectToClient)
        .build()
        .expect("valid configuration");

    assert!(config.connector().enable_audio_playback);
    assert!(config.channels().sound);
    assert_eq!(
        config.properties().audio_mode().unwrap(),
        Some(AudioMode::RedirectToClient)
    );
}

#[test]
fn with_audio_mode_play_on_server_disables_local_playback() {
    use ironrdp_cfg::AudioMode;

    let config = complete_builder()
        .with_audio_mode(AudioMode::PlayOnServer)
        .build()
        .expect("valid configuration");

    assert!(!config.connector().enable_audio_playback);
    assert!(!config.channels().sound);
    assert_eq!(config.properties().audio_mode().unwrap(), Some(AudioMode::PlayOnServer));
}

#[test]
fn with_audio_mode_disabled_suppresses_sound_channel() {
    use ironrdp_cfg::AudioMode;

    let config = complete_builder()
        .with_audio_mode(AudioMode::Disabled)
        .build()
        .expect("valid configuration");

    assert!(!config.connector().enable_audio_playback);
    assert!(!config.channels().sound);
    assert_eq!(config.properties().audio_mode().unwrap(), Some(AudioMode::Disabled));
}

#[test]
fn with_audio_capture_enables_client_info_flag_and_channel() {
    use ironrdp_cfg::AudioCaptureMode;

    let config = complete_builder()
        .with_audio_capture(true)
        .build()
        .expect("valid configuration");

    assert!(config.connector().enable_audio_capture);
    assert!(config.channels().audio_capture);
    assert_eq!(
        config.properties().audio_capture_mode().unwrap(),
        Some(AudioCaptureMode::CaptureFromClient)
    );
}

#[test]
fn with_audio_capture_disabled_clears_channel() {
    use ironrdp_cfg::AudioCaptureMode;

    let config = complete_builder()
        .with_audio_capture(true)
        .with_audio_capture(false)
        .build()
        .expect("valid configuration");

    assert!(!config.connector().enable_audio_capture);
    assert!(!config.channels().audio_capture);
    assert_eq!(
        config.properties().audio_capture_mode().unwrap(),
        Some(AudioCaptureMode::Disabled)
    );
}
