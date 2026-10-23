use std::fs;
use std::path::PathBuf;

use std::sync::Arc;

#[cfg(windows)]
use ironrdp_cfg::GatewayCredentialsSource;
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_client::config::{
    AudioQualityMode, ClipboardType, ConfigBuilder, Destination, DriveRedirection, Transport, UdpVersion, VmConnectMode,
};
#[cfg(windows)]
use ironrdp_client::config::{MissingField, TransportKind};
use ironrdp_client::h264::H264DecoderFactory;
use ironrdp_pdu::gcc::{ClientMonitorData, Monitor, MonitorFlags};
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_viewer::cli::{PrinterOptions, PrinterSelection, ViewerConfig, parse_config_from};
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

fn viewer_args(rdp_file: &TempRdpFile, extra_args: &[&str]) -> Vec<String> {
    let mut args = vec![
        "ironrdp-client".to_owned(),
        "--rdp-file".to_owned(),
        rdp_file.path().display().to_string(),
    ];

    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));
    args
}

fn parse_config_from_rdp_result(content: &str, extra_args: &[&str]) -> anyhow::Result<ironrdp_client::config::Config> {
    let rdp_file = TempRdpFile::new(content);
    parse_config_from(viewer_args(&rdp_file, extra_args))
}

fn parse_viewer_config_from_rdp(content: &str, extra_args: &[&str]) -> ViewerConfig {
    let rdp_file = TempRdpFile::new(content);
    ViewerConfig::parse_from(viewer_args(&rdp_file, extra_args)).expect("failed to parse viewer config")
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
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let config = parse_config_from_rdp(
        "full address:s:hyperv.example.com:2179\nusername:s:test-user\nClearTextPassword:s:test-pass\n",
        &["--vmconnect", VM_ID, "--vmconnect-basic"],
    );

    assert_eq!(config.vmconnect_mode(), Some(VmConnectMode::Basic));
    assert_eq!(config.properties().vmconnect_id(), Some(VM_ID));
    assert_eq!(config.properties().vmconnect_basic(), Some(true));
}

#[cfg(windows)]
#[test]
fn vmconnect_current_user_cli_flag_needs_no_credentials() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let config = parse_config_from([
        "ironrdp-viewer",
        "localhost",
        "--vmconnect",
        VM_ID,
        "--vmconnect-current-user",
    ])
    .expect("valid current-user VMConnect configuration");

    assert!(config.vmconnect_current_user());
    assert_eq!(config.vm_id(), Some(VM_ID));
    assert_eq!(config.properties().vmconnect_current_user(), Some(true));
}

#[test]
fn vmconnect_properties_restore_typed_config() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let config = parse_config_from_rdp(
        &format!(
            "full address:s:hyperv.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\nironrdp_vmconnect:s:{VM_ID}\nironrdp_vmconnect_basic:i:1\n"
        ),
        &[],
    );

    assert_eq!(config.vm_id(), Some(VM_ID));
    assert_eq!(config.vmconnect_mode(), Some(VmConnectMode::Basic));
    assert_eq!(config.destination().port(), 2179);
}

#[cfg(windows)]
#[test]
fn vmconnect_current_user_needs_no_password_and_preserves_explicit_product_id() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    const INSTANCE_ID: &str = "0123456789abcdef0123456789abcde";
    let config = parse_config_from_rdp(
        &format!(
            "full address:s:localhost\nironrdp_vmconnect:s:{VM_ID}\nironrdp_vmconnect_basic:i:1\nironrdp_vmconnect_current_user:i:1\n"
        ),
        &["--dig-product-id", INSTANCE_ID],
    );

    assert!(config.vmconnect_current_user());
    assert_eq!(config.vmconnect_mode(), Some(VmConnectMode::Basic));
    assert!(config.vmconnect_framebuffer_redirection());
    assert_eq!(config.connector().dig_product_id, INSTANCE_ID);
}

#[cfg(windows)]
#[test]
fn vmconnect_current_user_rejects_rdcleanpath() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let error = parse_config_from_rdp_result(
        &format!("full address:s:localhost\nironrdp_vmconnect:s:{VM_ID}\nironrdp_vmconnect_current_user:i:1\n"),
        &[
            "--rdcleanpath-url",
            "wss://rdcleanpath.example.com",
            "--rdcleanpath-token",
            "test-token",
        ],
    )
    .expect_err("current-user VMConnect must reject RDCleanPath");

    assert!(
        error
            .to_string()
            .contains("vmconnect current-user authentication is not supported with RDCleanPath"),
        "unexpected error: {error:#}"
    );
}

#[cfg(windows)]
#[test]
fn current_user_vmconnect_requires_gateway_source_credentials() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let builder = |gateway_username: Option<&str>, gateway_password: Option<&str>| {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.set_gateway_credentials_source(GatewayCredentialsSource::UseServerCredentials);
        let mut builder = ConfigBuilder::new()
            .with_destination(Destination::new("localhost").expect("valid destination"))
            .with_client_build(1)
            .with_client_dir("C:\\")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_client_name("client")
            .with_vmconnect(VM_ID)
            .with_vmconnect_current_user(true)
            .with_transport(TransportKind::Gateway {
                endpoint: "gateway.example.com:443".to_owned(),
                prefer_direct: false,
            })
            .with_property_set(&properties)
            .expect("valid gateway credential source");
        if let Some(username) = gateway_username {
            builder = builder.with_gateway_username(username);
        }
        if let Some(password) = gateway_password {
            builder = builder.with_gateway_password(password);
        }
        builder
    };

    let missing = builder(None, None);
    assert_eq!(missing.missing(), [MissingField::Username, MissingField::Password]);
    assert_eq!(
        missing
            .build()
            .expect_err("gateway source credentials must be required")
            .to_string(),
        "missing required configuration: username, password"
    );
    assert_eq!(builder(Some("gateway-user"), None).missing(), [MissingField::Password]);
    assert_eq!(
        builder(None, Some("gateway-password")).missing(),
        [MissingField::Username]
    );
}

#[cfg(windows)]
#[test]
fn loopback_vmconnect_hosts_offer_framebuffer_redirection() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    const INSTANCE_ID: &str = "0123456789abcdef0123456789abcde";
    for host in ["localhost", "localhost.", ".", "127.0.0.1", "[::1]"] {
        let config = parse_config_from_rdp(
            &format!("full address:s:{host}\nironrdp_vmconnect:s:{VM_ID}\nironrdp_vmconnect_current_user:i:1\n"),
            &["--dig-product-id", INSTANCE_ID],
        );

        assert!(
            config.vmconnect_framebuffer_redirection(),
            "{host} should be eligible for frame-buffer redirection"
        );
        if host == "." {
            assert_eq!(config.destination().name(), "localhost");
            assert_eq!(
                config
                    .properties()
                    .full_address()
                    .expect("valid full address")
                    .expect("normalized full address")
                    .to_string(),
                "localhost"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn gateway_fallback_vmconnect_does_not_offer_local_framebuffer_redirection() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let config = parse_config_from_rdp(
        &format!(
            "full address:s:localhost\nironrdp_vmconnect:s:{VM_ID}\nironrdp_vmconnect_current_user:i:1\ngatewayhostname:s:gateway.example.com\ngatewayusagemethod:i:2\ngatewayusername:s:gateway-user\nGatewayPassword:s:gateway-password\n"
        ),
        &[],
    );

    let Transport::Gateway(gateway) = config.transport() else {
        panic!("gateway fallback transport expected");
    };
    assert!(gateway.prefer_direct);
    assert!(!config.vmconnect_framebuffer_redirection());
    assert!(config.connector().dig_product_id.is_empty());
}

#[cfg(windows)]
#[test]
fn rdcleanpath_vmconnect_does_not_offer_local_framebuffer_redirection() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let config = parse_config_from_rdp(
        &format!(
            "full address:s:localhost\nusername:s:test-user\nClearTextPassword:s:test-pass\nironrdp_vmconnect:s:{VM_ID}\n"
        ),
        &[
            "--rdcleanpath-url",
            "wss://rdcleanpath.example.com",
            "--rdcleanpath-token",
            "test-token",
        ],
    );

    assert!(matches!(config.transport(), Transport::RDCleanPath(_)));
    assert!(!config.vmconnect_framebuffer_redirection());
    assert!(config.connector().dig_product_id.is_empty());
}

#[cfg(windows)]
#[test]
fn remote_vmconnect_does_not_offer_local_framebuffer_redirection() {
    const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
    let config = parse_config_from_rdp(
        &format!(
            "full address:s:hyperv.example.com\nironrdp_vmconnect:s:{VM_ID}\nironrdp_vmconnect_current_user:i:1\n"
        ),
        &[],
    );

    assert!(config.vmconnect_current_user());
    assert!(!config.vmconnect_framebuffer_redirection());
    assert!(config.connector().dig_product_id.is_empty());
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
fn printer_redirection_follows_redirectprinters_like_the_daemon() {
    let rdp = "full address:s:rdp.example.com
username:s:test-user
ClearTextPassword:s:test-pass
";

    assert!(!parse_viewer_config_from_rdp(rdp, &[]).redirect_printers());

    // The daemon honors `redirectprinters:i:1`; the viewer does the same for the default printer.
    let stock = parse_viewer_config_from_rdp(
        &format!(
            "{rdp}redirectprinters:i:1
"
        ),
        &[],
    );
    assert_eq!(
        stock.printers().map(|printers| &printers.selection),
        Some(&PrinterSelection::Default)
    );
    let channels = stock.into_config().expect("failed to build client config");
    assert!(channels.channels().rdpdr.enabled);
    assert!(channels.channels().rdpdr.printers);

    // `ironrdp_rdpdr:i:0` takes precedence over the file property, as in the daemon.
    assert!(
        !parse_viewer_config_from_rdp(
            &format!(
                "{rdp}redirectprinters:i:1
ironrdp_rdpdr:i:0
"
            ),
            &[],
        )
        .redirect_printers()
    );

    // `redirectprinters:i:0` keeps printers off in the client configuration too.
    let disabled = parse_config_from_rdp(
        &format!(
            "{rdp}redirectprinters:i:0
"
        ),
        &[],
    );
    assert!(!disabled.channels().rdpdr.printers);

    let viewer = parse_viewer_config_from_rdp(
        &format!(
            "{rdp}ironrdp_rdpdr:i:0
redirectprinters:i:0
"
        ),
        &["--printer"],
    );
    assert!(viewer.redirect_printers());
    let config = viewer.into_config().expect("failed to build client config");
    assert!(config.channels().rdpdr.enabled);
    assert!(config.channels().rdpdr.printers);
}

#[test]
fn printer_queue_options_select_named_printers_and_drivers() {
    let rdp = "full address:s:rdp.example.com
username:s:test-user
ClearTextPassword:s:test-pass
";

    let viewer = parse_viewer_config_from_rdp(
        rdp,
        &[
            "--printer-queue",
            "Office,HP Universal Printing PCL 6",
            "--printer-queue",
            r"\\print\Lobby",
            "--printer-default",
            "Office",
        ],
    );
    assert_eq!(
        viewer.printers(),
        Some(&PrinterOptions {
            selection: PrinterSelection::Named(vec!["Office".to_owned(), r"\\print\Lobby".to_owned()]),
            drivers: vec![("Office".to_owned(), "HP Universal Printing PCL 6".to_owned())],
            default_queue: Some("Office".to_owned()),
        })
    );

    let all = parse_viewer_config_from_rdp(rdp, &["--printer-all", "--printer-queue", "Office,Generic / Text Only"]);
    assert_eq!(
        all.printers()
            .map(|printers| (&printers.selection, printers.drivers.len())),
        Some((&PrinterSelection::All, 1))
    );

    // `--printer` keeps the default selection; entries only override drivers.
    let default = parse_viewer_config_from_rdp(rdp, &["--printer", "--printer-queue", "Office,Generic / Text Only"]);
    assert_eq!(
        default
            .printers()
            .map(|printers| (&printers.selection, printers.drivers.len())),
        Some((&PrinterSelection::Default, 1))
    );

    // RPC hosts take printer selection from connect properties, so printer flags are rejected.
    for flags in [
        &["--printer"][..],
        &["--printer-all"],
        &["--printer-queue", "Office"],
        &["--printer-default", "Office"],
    ] {
        let args = ["ironrdp-viewer", "--rpc"].into_iter().chain(flags.iter().copied());
        assert!(ViewerConfig::parse_from(args).is_err(), "{flags:?}");
    }

    let rdp_file = TempRdpFile::new(rdp);
    for invalid in [
        &["--printer-default", "Office"][..],
        &["--printer-queue", "Office,"],
        &["--printer-all", "--printer-queue", "Office"],
        &["--printer", "--printer-queue", "Office"],
        &["--printer", "--printer-all"],
    ] {
        assert!(
            ViewerConfig::parse_from(viewer_args(&rdp_file, invalid)).is_err(),
            "{invalid:?} must be rejected"
        );
    }
}

const DRIVE_TEST_RDP: &str = "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n";

#[test]
fn rdp_file_selects_drives_to_redirect() {
    let config = parse_config_from_rdp(
        &format!("{DRIVE_TEST_RDP}drivestoredirect:s:C:\\;e:\\;DynamicDrives\nredirectdrives:i:1\n"),
        &[],
    );

    let drives = config.channels().rdpdr.drives;
    assert!(!drives.redirects_all());
    assert!(drives.redirects_dynamic());
    assert_eq!(drives.letters().collect::<String>(), "CE");
}

#[test]
fn legacy_redirect_drives_selects_every_drive() {
    let config = parse_config_from_rdp(
        &format!("{DRIVE_TEST_RDP}drivestoredirect:s:\nredirectdrives:i:1\n"),
        &[],
    );
    assert_eq!(config.channels().rdpdr.drives, DriveRedirection::all());

    let config = parse_config_from_rdp(&format!("{DRIVE_TEST_RDP}drivestoredirect:s:\n"), &[]);
    assert!(config.channels().rdpdr.drives.is_empty());
}

#[test]
fn drive_cli_flags_override_the_rdp_file() {
    let rdp = format!("{DRIVE_TEST_RDP}drivestoredirect:s:C:\\\n");

    let config = parse_config_from_rdp(&rdp, &["--drives"]);
    assert_eq!(config.channels().rdpdr.drives, DriveRedirection::all());
    assert_eq!(config.properties().drives_to_redirect(), Some("*"));

    let config = parse_config_from_rdp(&rdp, &["--dynamic-drives"]);
    let drives = config.channels().rdpdr.drives;
    assert!(drives.redirects_dynamic());
    assert_eq!(drives.letters().collect::<String>(), "C");
    assert_eq!(config.properties().drives_to_redirect(), Some(r"C:\;DynamicDrives"));

    let config = parse_config_from_rdp(&format!("{DRIVE_TEST_RDP}ironrdp_rdpdr:i:0\n"), &["--dynamic-drives"]);
    assert!(config.channels().rdpdr.enabled);
}

#[test]
fn named_drive_cli_options_are_collected() {
    let viewer = parse_viewer_config_from_rdp(
        DRIVE_TEST_RDP,
        &[
            "--drive",
            r"Data,D:\data,2026",
            "--drive",
            r"tmp,C:\Temp",
            "--home-drive",
        ],
    );

    let drives = viewer.named_drives();
    assert_eq!(drives.len(), 3);
    assert_eq!(drives[0].name, "Data");
    assert_eq!(drives[0].path, PathBuf::from(r"D:\data,2026"));
    assert_eq!(drives[1].name, "tmp");
    assert_eq!(drives[2].name, "home");
    assert!(!drives[2].path.as_os_str().is_empty());
    assert!(viewer.into_config().expect("valid config").channels().rdpdr.enabled);

    for invalid in ["Data", r",C:\", "Data,"] {
        assert!(
            ViewerConfig::parse_from(["ironrdp-viewer", "--drive", invalid]).is_err(),
            "{invalid}"
        );
    }

    // RPC hosts take drive selection from connect properties, so drive flags are rejected.
    for flags in [
        &["--drives"][..],
        &["--dynamic-drives"],
        &["--home-drive"],
        &["--drive", r"Data,C:\"],
    ] {
        let args = ["ironrdp-viewer", "--rpc"].into_iter().chain(flags.iter().copied());
        assert!(ViewerConfig::parse_from(args).is_err(), "{flags:?}");
    }
}

#[cfg(windows)]
#[test]
fn named_drives_are_validated_before_connecting() {
    use ironrdp_client::rdp::RdpInputSender;
    use ironrdp_viewer::cli::NamedDrive;
    use ironrdp_viewer::drives::ViewerDrives;

    let (input_sender, _input_receiver) = RdpInputSender::channel(1);
    let windows_directory = PathBuf::from(std::env::var("SystemRoot").expect("SystemRoot is set on Windows"));
    let drive = |name: &str, path: &std::path::Path| NamedDrive {
        name: name.to_owned(),
        path: path.to_owned(),
    };
    let factory_with = |selection: DriveRedirection, drives: Vec<NamedDrive>| {
        ViewerDrives::new(selection, drives).backend_factory(&input_sender)
    };
    let factory = |drives: Vec<NamedDrive>| factory_with(DriveRedirection::default(), drives);

    let (valid, watcher) = factory(vec![drive("Data", &windows_directory)]).expect("valid named drive");
    assert_eq!(valid.initial_drives(), vec![(27, "Data".to_owned())]);
    assert!(watcher.is_none());

    let rejected = [
        // Names are case-insensitive.
        vec![
            drive("Data", &windows_directory),
            drive("data", &windows_directory.join("System32")),
        ],
        // A root may be redirected only once.
        vec![drive("One", &windows_directory), drive("Two", &windows_directory)],
        // Missing directories, files, and network shares cannot be drive roots.
        vec![drive("Missing", &windows_directory.join("ironrdp-missing-directory"))],
        vec![drive("File", &windows_directory.join("notepad.exe"))],
        vec![drive(
            "Share",
            std::path::Path::new(r"\\localhost\ironrdp-missing-share"),
        )],
    ];
    for drives in rejected {
        let names = drives.iter().map(|drive| drive.name.clone()).collect::<Vec<_>>();
        assert!(factory(drives).is_err(), "{names:?}");
    }

    // Drive-letter names belong to logical drives once drive letters are redirected.
    assert!(factory(vec![drive("C:", &windows_directory)]).is_ok());
    assert!(factory_with(DriveRedirection::parse("D"), vec![drive("C:", &windows_directory)]).is_err());
}

#[cfg(windows)]
#[test]
fn rdpdr_disabled_in_the_rdp_file_drops_drive_letters() {
    let config = parse_config_from_rdp(
        &format!("{DRIVE_TEST_RDP}ironrdp_rdpdr:i:0\ndrivestoredirect:s:*\n"),
        &[],
    );
    let rdpdr = &config.channels().rdpdr;
    assert!(!rdpdr.enabled);
    assert!(ironrdp_viewer::drives::ViewerDrives::from_rdpdr_config(rdpdr, Vec::new()).is_empty());
}

#[test]
fn port_redirection_follows_the_cli_and_redirectcomports() {
    use ironrdp_viewer::cli::{PortChoice, PortOptions};

    let rdp = "full address:s:rdp.example.com
username:s:test-user
ClearTextPassword:s:test-pass
";

    assert!(!parse_viewer_config_from_rdp(rdp, &[]).ports().is_enabled());
    assert!(
        !parse_viewer_config_from_rdp(
            &format!(
                "{rdp}redirectcomports:i:0
"
            ),
            &[]
        )
        .ports()
        .is_enabled()
    );

    let all = parse_viewer_config_from_rdp(
        &format!(
            "{rdp}redirectcomports:i:1
"
        ),
        &[],
    );
    assert_eq!(
        all.ports(),
        &PortOptions {
            serial: PortChoice::All,
            parallel: PortChoice::All,
        }
    );
    let config = all.into_config().expect("failed to build client config");
    assert!(config.channels().rdpdr.enabled);
    assert!(config.channels().rdpdr.ports);

    // The file's channel disable wins over its own port selection.
    let disabled = parse_viewer_config_from_rdp(
        &format!(
            "{rdp}redirectcomports:i:1
ironrdp_rdpdr:i:0
"
        ),
        &[],
    );
    assert!(!disabled.ports().is_enabled());
    assert!(
        !disabled
            .into_config()
            .expect("failed to build client config")
            .channels()
            .rdpdr
            .enabled
    );

    // Explicit CLI ports replace the file's all-ports selection.
    let named = parse_viewer_config_from_rdp(
        &format!(
            "{rdp}redirectcomports:i:0
"
        ),
        &["--serial", "COM3", "--serial", "com4", "--parallel", "all"],
    );
    assert_eq!(
        named.ports(),
        &PortOptions {
            serial: PortChoice::Named(vec!["COM3".to_owned(), "com4".to_owned()]),
            parallel: PortChoice::All,
        }
    );
    let config = named.into_config().expect("failed to build client config");
    assert!(config.channels().rdpdr.ports);
    assert_eq!(config.properties().redirect_com_ports(), Some(true));

    // RPC hosts take port selection from connect properties, so port flags are rejected.
    for flags in [&["--serial", "COM3"][..], &["--parallel", "all"]] {
        let args = ["ironrdp-viewer", "--rpc"].into_iter().chain(flags.iter().copied());
        assert!(ViewerConfig::parse_from(args).is_err(), "{flags:?}");
    }
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
    assert_eq!(
        config.load_balance_info(),
        Some("tsv://MS Terminal Services Plugin.1.collection")
    );
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

    assert_eq!(
        config.load_balance_info(),
        Some("tsv://MS Terminal Services Plugin.1.collection")
    );
    assert!(config.administrative_session());
    assert_eq!(config.audio_quality_mode(), AudioQualityMode::Medium);
}

#[test]
fn empty_rdp_load_balance_info_clears_an_existing_token() {
    for value in ["", "\r\n"] {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.insert("loadbalanceinfo", value);

        let config = ConfigBuilder::new()
            .with_destination(Destination::from_parts("rdp.example.com", 3389))
            .with_username("test-user")
            .with_password("test-pass")
            .with_client_build(1)
            .with_client_dir("C:\\Windows\\System32")
            .with_client_name("ironrdp-tests")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_load_balance_info("tsv://existing")
            .with_property_set(&properties)
            .expect("valid empty load-balance property")
            .build()
            .expect("valid configuration without a routing token");

        assert!(config.load_balance_info().is_none());
    }
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
fn reliable_udp_is_opt_in() {
    let config = complete_builder().build().expect("default config");
    assert!(!config.udp_transport_enabled());

    let config = complete_builder()
        .with_udp_transport(true)
        .build()
        .expect("UDP-enabled config");
    assert!(config.udp_transport_enabled());
}

#[test]
fn udp_offer_defaults_to_version_3_and_can_be_lowered() {
    let config = complete_builder().build().expect("default config");
    assert_eq!(config.udp_offer_version(), UdpVersion::V3);

    for version in [UdpVersion::V1, UdpVersion::V2, UdpVersion::V3] {
        let config = complete_builder()
            .with_udp_offer_version(version)
            .build()
            .expect("config with a UDP offer version");
        assert_eq!(config.udp_offer_version(), version);
    }
}

fn null_decoder() -> Option<H264DecoderFactory> {
    Some(H264DecoderFactory::new(|| None))
}

// oracle: mutant the graphics pipeline default ignoring the decoder or the opt-outs
#[test]
fn graphics_pipeline_follows_the_h264_decoder() {
    let config = complete_builder().build().expect("default config");
    assert_eq!(
        config.connector().support_dyn_vc_gfx_protocol,
        H264DecoderFactory::platform_default().is_some()
    );

    for (decoder, explicit, enabled) in [
        (null_decoder(), None, true),
        (None, None, false),
        (null_decoder(), Some(false), false),
        (None, Some(true), true),
    ] {
        let mut builder = complete_builder().with_h264_decoder(decoder);
        if let Some(explicit) = explicit {
            builder = builder.with_graphics_pipeline(explicit);
        }
        let config = builder.build().expect("graphics pipeline config");
        assert_eq!(config.connector().support_dyn_vc_gfx_protocol, enabled);
    }

    for (value, enabled) in [(1i64, true), (0, false)] {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.insert("ironrdp_egfx", value);
        for decoder in [null_decoder(), None] {
            let config = complete_builder()
                .with_h264_decoder(decoder)
                .with_property_set(&properties)
                .expect("valid graphics pipeline property")
                .build()
                .expect("graphics pipeline property config");
            assert_eq!(config.connector().support_dyn_vc_gfx_protocol, enabled);
        }
    }
}

// oracle: mutant the configured factory dropped or not called
#[test]
fn the_h264_decoder_factory_is_shared_and_replaceable() {
    let config = complete_builder().build().expect("default config");
    assert_eq!(
        config.channels().h264_decoder.is_some(),
        H264DecoderFactory::platform_default().is_some()
    );
    #[cfg(windows)]
    assert!(
        config.channels().h264_decoder.is_some(),
        "Media Foundation is the Windows default"
    );

    let created = Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let factory = {
        let created = Arc::clone(&created);
        H264DecoderFactory::new(move || {
            created.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            None
        })
    };
    let config = complete_builder()
        .with_h264_decoder(Some(factory))
        .build()
        .expect("config with a decoder");
    let decoder = config.channels().h264_decoder.as_ref().expect("the decoder is kept");
    assert!(decoder.create().is_none());
    assert_eq!(created.load(core::sync::atomic::Ordering::Relaxed), 1);

    let config = complete_builder()
        .with_h264_decoder(None)
        .build()
        .expect("config without a decoder");
    assert!(config.channels().h264_decoder.is_none());
}

/// A decoder that only reports YUV support, for checking what the background start forwards.
struct YuvDecoder;

impl ironrdp_egfx::decode::H264Decoder for YuvDecoder {
    fn decode(&mut self, _data: &[u8]) -> ironrdp_egfx::decode::DecoderResult<ironrdp_egfx::decode::DecodedFrame> {
        Err(ironrdp_egfx::decode::DecoderError::msg("not a real decoder"))
    }

    fn supports_yuv420(&self) -> bool {
        true
    }
}

// oracle: mutant the background start blocking its caller or losing the created decoder
#[test]
fn a_decoder_started_in_background_is_ready_later() {
    let slow = H264DecoderFactory::new(|| {
        std::thread::sleep(core::time::Duration::from_secs(2));
        Some(Box::new(YuvDecoder))
    });
    let started = std::time::Instant::now();
    let mut decoder = slow.create_in_background();
    assert!(
        started.elapsed() < core::time::Duration::from_secs(1),
        "creating the decoder returned at once"
    );
    assert!(
        !decoder.supports_yuv420(),
        "nothing is advertised before the decoder is ready"
    );

    assert!(decoder.wait_until_ready());
    assert!(decoder.supports_yuv420());
}

// oracle: mutant an unavailable backend reported as ready
#[test]
fn an_unavailable_decoder_is_not_ready() {
    let mut decoder = null_decoder().expect("factory").create_in_background();
    assert!(!decoder.wait_until_ready());
    assert!(!decoder.supports_yuv420());
    assert!(decoder.decode(&[0, 0, 0, 1]).is_err());
}

// oracle: reference mstsc videoplaybackmode:i:0
#[test]
fn video_playback_mode_zero_declines_video() {
    let config = complete_builder()
        .with_h264_decoder(null_decoder())
        .build()
        .expect("video config");
    assert!(config.channels().video.enabled);

    let mut properties = ironrdp_propertyset::PropertySet::new();
    properties.insert("videoplaybackmode", 0i64);
    let config = complete_builder()
        .with_h264_decoder(null_decoder())
        .with_property_set(&properties)
        .expect("valid video playback property")
        .build()
        .expect("video playback property config");
    assert!(!config.channels().video.enabled);
    // The graphics pipeline keeps the decoder.
    assert!(config.channels().h264_decoder.is_some());
    assert!(config.connector().support_dyn_vc_gfx_protocol);
}

#[test]
fn udp_and_graphics_pipeline_cli_flags_reach_the_config() {
    let rdp = "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n";

    let config = parse_config_from_rdp(rdp, &[]);
    assert!(!config.udp_transport_enabled());
    assert_eq!(config.udp_offer_version(), UdpVersion::V3);
    assert_eq!(
        config.connector().support_dyn_vc_gfx_protocol,
        H264DecoderFactory::platform_default().is_some()
    );

    let config = parse_config_from_rdp(rdp, &["--udp", "--udp-offer", "2", "--egfx"]);
    assert!(config.udp_transport_enabled());
    assert_eq!(config.udp_offer_version(), UdpVersion::V2);
    assert!(config.connector().support_dyn_vc_gfx_protocol);

    let config = parse_config_from_rdp(rdp, &["--egfx=false"]);
    assert!(!config.connector().support_dyn_vc_gfx_protocol);

    let config = parse_config_from_rdp(rdp, &["--udp", "--udp-offer", "1"]);
    assert_eq!(config.udp_offer_version(), UdpVersion::V1);
}

/// Where `bitmapcachepersistenable:i:1` puts the cache, derived from the documented location
/// rather than from `default_cache_path`.
fn documented_default_cache_path() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };

    #[cfg(windows)]
    {
        absolute("LOCALAPPDATA").map(|dir| dir.join("IronRDP").join("egfx-cache.bin"))
    }

    #[cfg(not(windows))]
    {
        absolute("XDG_CACHE_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".cache")))
            .map(|dir| dir.join("ironrdp").join("egfx-cache.bin"))
    }
}

#[test]
fn bitmap_cache_persistence_reaches_the_graphics_cache() {
    // oracle: reference Microsoft RDP file settings (bitmapcachepersistenable 1 caches bitmaps on
    // the local computer, 0 does not); off without the key is mutant (desktop pixels written to disk
    // without being asked), unlike mstsc; the default location is mutant (a path other than the one
    // default_cache_path documents)
    let rdp = "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n";
    assert_eq!(parse_config_from_rdp(rdp, &[]).channels().egfx_persistent_cache, None);

    let enabled = format!("{rdp}bitmapcachepersistenable:i:1\n");
    assert_eq!(
        parse_config_from_rdp(&enabled, &[]).channels().egfx_persistent_cache,
        documented_default_cache_path()
    );
    assert_eq!(
        parse_config_from_rdp(&enabled, &["--egfx-cache", "explicit.bin"])
            .channels()
            .egfx_persistent_cache,
        Some(PathBuf::from("explicit.bin"))
    );

    let disabled = format!("{rdp}bitmapcachepersistenable:i:0\n");
    assert_eq!(
        parse_config_from_rdp(&disabled, &[]).channels().egfx_persistent_cache,
        None
    );
    // The command line is applied after the .rdp file, so an explicit path still wins.
    assert_eq!(
        parse_config_from_rdp(&disabled, &["--egfx-cache", "explicit.bin"])
            .channels()
            .egfx_persistent_cache,
        Some(PathBuf::from("explicit.bin"))
    );
}

#[test]
fn udp_offer_without_udp_is_rejected() {
    let rdp = "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n";

    let error = parse_config_from_rdp_result(rdp, &["--udp-offer", "2"])
        .expect_err("--udp-offer is only used with --udp, so it must not be accepted alone");
    assert!(error.to_string().contains("required arguments"), "{error:?}");
    assert!(error.to_string().contains("--udp"), "{error:?}");

    // Turning UDP off explicitly leaves nothing to be surprised by.
    let config = parse_config_from_rdp(rdp, &["--udp=false", "--udp-offer", "2"]);
    assert!(!config.udp_transport_enabled());
}

#[test]
fn udp_offer_requires_udp_in_rpc_mode() {
    let error = ViewerConfig::parse_from(["ironrdp-viewer", "--rpc", "--udp-offer", "2"])
        .err()
        .expect("clap must validate UDP arguments before entering RPC mode");
    assert!(error.to_string().contains("required arguments"), "{error:?}");
    assert!(error.to_string().contains("--udp"), "{error:?}");

    assert!(ViewerConfig::parse_from(["ironrdp-viewer", "--rpc", "--udp=false", "--udp-offer", "2",]).is_ok());
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

// oracle: spec crates/ironrdp-viewer/README.md §`.rdp` file support (ironrdp_audio* keys)
#[test]
fn rdp_audio_properties_select_devices_and_latency() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com
username:s:test-user
ClearTextPassword:s:test-pass
ironrdp_audiooutputdevice:s:Headphones
ironrdp_audioinputdevice:s:wasapi:{0.0.1.00000000}.{mic}
ironrdp_audiolatency:i:250
",
        &[],
    );

    let audio = &config.channels().audio;
    assert_eq!(audio.output_device.as_deref(), Some("Headphones"));
    assert_eq!(audio.input_device.as_deref(), Some("wasapi:{0.0.1.00000000}.{mic}"));
    assert_eq!(audio.latency, Some(core::time::Duration::from_millis(250)));
}

// oracle: spec crates/ironrdp-viewer/README.md §`.rdp` file support (ironrdp_audiolatency is a duration)
#[test]
fn negative_rdp_audio_latency_is_rejected() {
    let result = parse_config_from_rdp_result(
        "full address:s:rdp.example.com
username:s:test-user
ClearTextPassword:s:test-pass
ironrdp_audiolatency:i:-5
",
        &[],
    );

    assert!(result.is_err());
}

// oracle: property round-trip
#[test]
fn audio_builder_settings_round_trip_through_the_property_set() {
    let config = complete_builder()
        .with_audio_output_device(Some("Speakers".to_owned()))
        .with_audio_input_device(Some("Array".to_owned()))
        .with_audio_latency(core::time::Duration::from_millis(150))
        .build()
        .expect("valid configuration");

    let reparsed = complete_builder()
        .with_property_set(config.properties())
        .expect("valid properties")
        .build()
        .expect("valid configuration");

    assert_eq!(reparsed.channels().audio, config.channels().audio);
    assert_eq!(config.channels().audio.output_device.as_deref(), Some("Speakers"));
}

// oracle: spec crates/ironrdp-viewer/README.md §`.rdp` file support (property precedence)
#[test]
fn viewer_audio_flags_override_rdp_properties() {
    let config = parse_config_from_rdp(
        "full address:s:rdp.example.com
username:s:test-user
ClearTextPassword:s:test-pass
ironrdp_audiooutputdevice:s:Headphones
ironrdp_audiolatency:i:250
",
        &[
            "--audio-output-device",
            "Speakers",
            "--audio-input-device",
            "Array",
            "--audio-latency",
            "60",
        ],
    );

    let audio = &config.channels().audio;
    assert_eq!(audio.output_device.as_deref(), Some("Speakers"));
    assert_eq!(audio.input_device.as_deref(), Some("Array"));
    assert_eq!(audio.latency, Some(core::time::Duration::from_millis(60)));
}

#[test]
fn camerastoredirect_selects_redirected_cameras() {
    let mut properties = ironrdp_propertyset::PropertySet::new();
    properties.insert("camerastoredirect", r"*; -\\?\usb#vid_0001 ;Front Camera;");

    let config = complete_builder()
        .with_property_set(&properties)
        .expect("valid camera selection")
        .build()
        .expect("valid configuration");

    let cameras = &config.channels().cameras;
    assert!(cameras.is_enabled());
    assert!(cameras.redirect_all);
    assert_eq!(cameras.include, ["Front Camera"]);
    assert_eq!(cameras.exclude, [r"\\?\usb#vid_0001"]);
    assert!(cameras.encode_video);
    assert!(cameras.selects(r"\\?\usb#vid_0002", "Rear Camera"));
    assert!(!cameras.selects(r"\\?\USB#VID_0001", "Excluded Camera"));

    let mut explicit = ironrdp_client::camera::CameraConfig::default();
    explicit.apply_rdp_value("front camera");
    assert!(!explicit.redirect_all);
    assert!(explicit.selects(r"\\?\usb#vid_0003", "Front Camera"));
    assert!(!explicit.selects(r"\\?\usb#vid_0002", "Rear Camera"));
    assert!(!ironrdp_client::camera::CameraConfig::default().is_enabled());
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

const RDP_TARGET: &str = "full address:s:rdp.example.com\nusername:s:test-user\nClearTextPassword:s:test-pass\n";

// oracle: mutant a session bpp outside 16 and 32 must map to the nearest depth IronRDP renders (no document decides
// the mapping; it is this client's choice), and ironrdp_colordepth must keep precedence
#[test]
fn session_bpp_selects_the_closest_supported_color_depth() {
    let color_depth = |rdp: &str| {
        parse_config_from_rdp(&format!("{RDP_TARGET}{rdp}"), &[])
            .connector()
            .bitmap
            .as_ref()
            .expect("bitmap config")
            .color_depth
    };

    assert_eq!(color_depth("session bpp:i:15\n"), 16);
    assert_eq!(color_depth("session bpp:i:24\n"), 32);
    assert_eq!(color_depth("session bpp:i:16\n"), 16);
    // The IronRDP-specific color depth takes precedence.
    assert_eq!(color_depth("session bpp:i:16\nironrdp_colordepth:i:32\n"), 32);

    assert!(parse_config_from_rdp_result(&format!("{RDP_TARGET}session bpp:i:12\n"), &[]).is_err());
}

#[test]
fn rdp_file_maps_device_scale_factor_and_connection_type() {
    use ironrdp_pdu::gcc::ConnectionType;

    let config = parse_config_from_rdp(
        &format!("{RDP_TARGET}desktopscalefactor:i:150\ndevicescalefactor:i:140\nconnection type:i:2\n"),
        &[],
    );
    assert_eq!(config.connector().desktop_scale_factor, 150);
    assert_eq!(config.connector().device_scale_factor, Some(140));
    assert_eq!(config.connector().connection_type, ConnectionType::BroadbandLow);

    for invalid in [
        "devicescalefactor:i:120\n",
        "connection type:i:0\n",
        "connection type:i:8\n",
    ] {
        assert!(
            parse_config_from_rdp_result(&format!("{RDP_TARGET}{invalid}"), &[]).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn builder_display_attributes_reach_connector_configuration() {
    use ironrdp_pdu::gcc::{ClientMonitorExtendedData, ExtendedMonitorInfo, MonitorOrientation};

    let monitor_extended = ClientMonitorExtendedData {
        extended_monitors_info: vec![ExtendedMonitorInfo {
            physical_width: 0,
            physical_height: 0,
            orientation: MonitorOrientation::Portrait,
            desktop_scale_factor: 125,
            device_scale_factor: 100,
        }],
    };
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("rdp.example.com", 3389))
        .with_username("test-user")
        .with_password("test-pass")
        .with_client_build(1)
        .with_client_dir("C:\\Windows\\System32")
        .with_client_name("ironrdp-tests")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_desktop_scale_factor(125)
        .with_device_scale_factor(140)
        .with_desktop_physical_size(600, 340)
        .with_desktop_orientation(MonitorOrientation::LandscapeFlipped)
        .with_monitor_layout(ClientMonitorData {
            monitors: vec![Monitor {
                left: 0,
                top: 0,
                right: 1_023,
                bottom: 767,
                flags: MonitorFlags::PRIMARY,
            }],
        })
        .with_monitor_extended_layout(monitor_extended.clone())
        .with_max_touch_contacts(20)
        .build()
        .expect("valid display configuration");

    let connector = config.connector();
    assert_eq!(connector.device_scale_factor, Some(140));
    assert_eq!(connector.desktop_physical_size, Some((600, 340)));
    assert_eq!(
        connector.desktop_orientation,
        Some(MonitorOrientation::LandscapeFlipped)
    );
    assert_eq!(connector.monitor_extended_layout, Some(monitor_extended));
    assert_eq!(config.channels().max_touch_contacts, 20);
    assert_eq!(config.properties().get::<u32>("devicescalefactor"), Some(140));
}

// oracle: spec crates/ironrdp-rdpeusb-native/README.md §Device selection
#[test]
fn property_set_selects_usb_devices() {
    use ironrdp_client::usb::UsbDeviceMatch;

    let mut properties = ironrdp_propertyset::PropertySet::new();
    properties.set_usb_devices_to_redirect(r"USB\VID_2E8A&PID_000A\E66038B7135F2B2E;-USB\VID_046D&PID_C52B");

    let config = complete_builder()
        .with_property_set(&properties)
        .expect("valid USB property")
        .build()
        .expect("valid configuration");
    let usb = config.channels().usb.as_ref().expect("USB redirection is enabled");
    assert_eq!(
        usb.devices,
        [UsbDeviceMatch::InstanceId(
            r"USB\VID_2E8A&PID_000A\E66038B7135F2B2E".to_owned()
        )]
    );
    assert_eq!(
        usb.exclude,
        [UsbDeviceMatch::VidPid {
            vendor_id: 0x046d,
            product_id: 0xc52b,
        }]
    );
}

// oracle: spec crates/ironrdp-rdpeusb-native/README.md §Device selection
#[test]
fn property_set_without_usb_devices_disables_usb() {
    let mut properties = ironrdp_propertyset::PropertySet::new();
    properties.set_usb_devices_to_redirect("{36fc9e60-c465-11cf-8056-444553540000}");

    let config = complete_builder()
        .with_property_set(&properties)
        .expect("valid USB property")
        .build()
        .expect("valid configuration");
    assert!(config.channels().usb.is_none());
}

// oracle: spec crates/ironrdp-rdpeusb-native/README.md §Device selection
#[test]
fn property_set_rejects_invalid_usb_devices() {
    let mut properties = ironrdp_propertyset::PropertySet::new();
    properties.set_usb_devices_to_redirect(r"USB\VID_XYZ&PID_0001");

    assert!(ConfigBuilder::from_property_set(&properties).is_err());
}
