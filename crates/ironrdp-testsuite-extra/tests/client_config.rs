use std::fs;
use std::path::PathBuf;

use std::sync::Arc;

use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_client::config::{ClipboardType, ConfigBuilder, Destination, Transport, TransportKind};
use ironrdp_viewer::cli::parse_config_from;
use url::Url;
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

fn parse_config_from_rdp(content: &str, extra_args: &[&str]) -> ironrdp_client::config::Config {
    let rdp_file = TempRdpFile::new(content);

    let mut args = vec![
        "ironrdp-client".to_owned(),
        "--rdp-file".to_owned(),
        rdp_file.path().display().to_string(),
    ];

    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));

    parse_config_from(args).expect("failed to parse client config")
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

    let callback: ironrdp_tls::CertificateValidationCallback =
        Arc::new(|certificate, reason| certificate == b"test certificate" && reason == "untrusted issuer");
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("rdp.example.com", 3389))
        .with_username("test-user")
        .with_password("test-pass")
        .with_client_build(1)
        .with_client_dir("C:\\Windows\\System32")
        .with_client_name("ironrdp-tests")
        .with_platform(ironrdp::pdu::rdp::capability_sets::MajorPlatformType::WINDOWS)
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
    assert!(callback(b"test certificate", "untrusted issuer"));
    assert!(!callback(b"other certificate", "untrusted issuer"));
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
        .with_platform(ironrdp::pdu::rdp::capability_sets::MajorPlatformType::WINDOWS)
        .with_keyboard_layout(0x0000_0409)
        .with_connection_type(ConnectionType::BroadbandHigh)
        .with_lossy_compression(false)
        .with_performance_flags(performance_flags)
        .with_alternate_shell("powershell.exe")
        .with_work_dir("C:\\Users\\test-user")
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

#[test]
fn vmconnect_cli_flag_reaches_the_config() {
    let config = parse_config_from_rdp(
        "full address:s:hyperv-host.example.com:2179\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &["--vmconnect", "efd1efab-c750-4262-b1bb-af0f7733bdd6"],
    );

    assert_eq!(config.vm_id(), Some("efd1efab-c750-4262-b1bb-af0f7733bdd6"));
}

/// A builder with every required field filled in, ready for a transport to be selected.
fn vmconnect_builder() -> ConfigBuilder {
    ConfigBuilder::new()
        .with_destination(Destination::new("hyperv-host.example.com:2179").expect("valid destination"))
        .with_username("test-user")
        .with_password("test-pass")
        .with_client_build(0)
        .with_client_dir(String::new())
        .with_client_name("test")
        .with_platform(MajorPlatformType::UNIX)
        .with_vmconnect("efd1efab-c750-4262-b1bb-af0f7733bdd6")
}

#[test]
fn vmconnect_is_rejected_over_rdcleanpath() {
    let err = vmconnect_builder()
        .with_transport(TransportKind::RDCleanPath {
            url: Url::parse("wss://proxy.example.com").expect("valid URL"),
        })
        .with_rdcleanpath_token("token")
        .build()
        .expect_err("RDCleanPath negotiates X.224 for us, which vmconnect cannot accommodate");

    assert!(err.to_string().contains("vmconnect"), "unexpected error: {err}");
}

/// A gateway is only a tunnel, so the connector still drives the Hyper-V ordering itself.
#[test]
fn vmconnect_is_allowed_over_a_gateway() {
    let config = vmconnect_builder()
        .with_transport(TransportKind::Gateway {
            endpoint: "gw.example.com:443".to_owned(),
        })
        .with_gateway_username("gw-user")
        .with_gateway_password("gw-pass")
        .build()
        .expect("gateway vmconnect should build");

    assert_eq!(config.vm_id(), Some("efd1efab-c750-4262-b1bb-af0f7733bdd6"));
    assert!(matches!(config.transport(), Transport::Gateway(_)));
}
