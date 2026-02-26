#![allow(unused_crate_dependencies)]
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

use std::fs;
use std::path::PathBuf;

use ironrdp_client::config::{ClipboardType, Config};
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
}

impl Drop for TempRdpFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn parse_config_from_rdp(content: &str, extra_args: &[&str]) -> Config {
    let rdp_file = TempRdpFile::new(content);

    let mut args = vec![
        "ironrdp-client".to_owned(),
        "--rdp-file".to_owned(),
        rdp_file.path.display().to_string(),
    ];

    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));

    Config::parse_from(args).expect("failed to parse client config")
}

#[test]
fn gateway_is_disabled_when_gateway_usage_method_is_zero() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:0\n",
        &[],
    );

    assert!(config.gw.is_none());
}

#[test]
fn gateway_is_disabled_when_gateway_usage_method_is_four() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:4\n",
        &[],
    );

    assert!(config.gw.is_none());
}

#[test]
fn gateway_is_enabled_with_usage_method_one_and_file_credentials() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:1\ngatewayusername:s:gw-user\nGatewayPassword:s:gw-pass\n",
        &[],
    );

    let gw = config.gw.expect("gateway should be configured");
    assert_eq!(gw.gw_endpoint, "gw.example.com:443");
    assert_eq!(gw.gw_user, "gw-user");
    assert_eq!(gw.gw_pass, "gw-pass");
}

#[test]
fn unsupported_gateway_credential_source_falls_back_to_username_password() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ngatewayhostname:s:gw.example.com:443\ngatewayusagemethod:i:1\ngatewaycredentialssource:i:2\ngatewayusername:s:gw-user\nGatewayPassword:s:gw-pass\n",
        &[],
    );

    let gw = config.gw.expect("gateway should be configured");
    assert_eq!(gw.gw_user, "gw-user");
    assert_eq!(gw.gw_pass, "gw-pass");
}

#[test]
fn no_credssp_cli_flag_overrides_rdp_enable_credssp_property() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nenablecredsspsupport:i:1\n",
        &["--no-credssp"],
    );

    assert!(!config.connector.enable_credssp);
}

#[test]
fn kdc_proxy_name_is_normalized_to_https_url() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nkdcproxyname:s:kdc.example.com\n",
        &[],
    );

    let kerberos = config.kerberos_config.expect("kerberos config should be present");
    let kdc_proxy_url = kerberos.kdc_proxy_url.expect("kdc proxy url should be present");

    assert_eq!(kdc_proxy_url.as_str(), "https://kdc.example.com/KdcProxy");
}

#[test]
fn redirectclipboard_zero_disables_clipboard_for_default_mode() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nredirectclipboard:i:0\n",
        &[],
    );

    assert!(matches!(config.clipboard_type, ClipboardType::None));
}

#[test]
fn audiomode_two_disables_audio_playback() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\naudiomode:i:2\n",
        &[],
    );

    assert!(!config.connector.enable_audio_playback);
}

#[test]
fn invalid_audiomode_falls_back_to_audio_playback_enabled() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\naudiomode:i:99\n",
        &[],
    );

    assert!(config.connector.enable_audio_playback);
}

#[test]
fn desktop_dimensions_are_parsed_from_rdp_file() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ndesktopwidth:i:1024\ndesktopheight:i:768\ndesktopscalefactor:i:125\n",
        &[],
    );

    assert_eq!(config.connector.desktop_size.width, 1024);
    assert_eq!(config.connector.desktop_size.height, 768);
    assert_eq!(config.connector.desktop_scale_factor, 125);
}

#[test]
fn out_of_range_desktop_dimensions_fall_back_to_defaults() {
    let default_config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &[],
    );

    let invalid_config = parse_config_from_rdp(
        "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\ndesktopwidth:i:-1\ndesktopheight:i:-1\ndesktopscalefactor:i:99999999999\n",
        &[],
    );

    assert_eq!(
        invalid_config.connector.desktop_size.width,
        default_config.connector.desktop_size.width
    );
    assert_eq!(
        invalid_config.connector.desktop_size.height,
        default_config.connector.desktop_size.height
    );
    assert_eq!(
        invalid_config.connector.desktop_scale_factor,
        default_config.connector.desktop_scale_factor
    );
}
