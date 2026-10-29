#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::time::Duration;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use clap::clap_derive::ValueEnum;
use ironrdp::client::config::{
    ClipboardType as ResolvedClipboardType, Config, ConfigBuilder, Destination, DriveRedirection, DvcProxyInfo,
    MissingField, TransportKind, VmConnectMode,
};
use ironrdp::client::usb::UsbRedirectionConfig;
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_propertyset::Value;
use tap::prelude::*;
use url::Url;

/// CLI selection for the clipboard backend.
///
/// Maps directly into the library's [`ResolvedClipboardType`] when the typed [`Config`] is built.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum ClipboardType {
    /// Enable clipboard redirection (use the best available backend).
    Enable,
    /// Disable clipboard redirection entirely.
    Disable,
    /// Use a stub clipboard backend (for testing or headless usage).
    Stub,
}

/// A single `--prop KEY:TYPE:VALUE` override, parsed with the same grammar as one `.rdp` file line
/// (see `ironrdp_rdpfile::load`): `TYPE` is `i` for integer or `s` for string.
#[derive(Clone, Debug)]
struct PropOverride {
    key: String,
    value: Value,
}

impl core::str::FromStr for PropOverride {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut parts = input.splitn(3, ':');
        let (Some(key), Some(ty), Some(value)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("malformed --prop '{input}', expected KEY:TYPE:VALUE"));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("empty key in --prop '{input}', expected KEY:TYPE:VALUE"));
        }
        let value = match ty {
            "i" => value
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| format!("invalid integer value in --prop '{input}'"))?,
            "s" => Value::from(value),
            other => {
                return Err(format!(
                    "unknown type '{other}' in --prop '{input}', expected 'i' or 's'"
                ));
            }
        };
        Ok(Self {
            key: key.to_owned(),
            value,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum KeyboardType {
    IbmPcXt,
    OlivettiIco,
    IbmPcAt,
    IbmEnhanced,
    Nokia1050,
    Nokia9140,
    Japanese,
}

impl KeyboardType {
    fn into_pdu(self) -> ironrdp::pdu::gcc::KeyboardType {
        match self {
            KeyboardType::IbmEnhanced => ironrdp::pdu::gcc::KeyboardType::IBM_ENHANCED,
            KeyboardType::IbmPcAt => ironrdp::pdu::gcc::KeyboardType::IBM_PC_AT,
            KeyboardType::IbmPcXt => ironrdp::pdu::gcc::KeyboardType::IBM_PC_XT,
            KeyboardType::OlivettiIco => ironrdp::pdu::gcc::KeyboardType::OLIVETTI_ICO,
            KeyboardType::Nokia1050 => ironrdp::pdu::gcc::KeyboardType::NOKIA_1050,
            KeyboardType::Nokia9140 => ironrdp::pdu::gcc::KeyboardType::NOKIA_9140,
            KeyboardType::Japanese => ironrdp::pdu::gcc::KeyboardType::JAPANESE,
        }
    }
}

/// Devolutions IronRDP viewer
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP viewer")]
#[clap(version, long_about = None)]
struct Args {
    /// A file with IronRDP viewer logs
    #[clap(short, long, value_parser)]
    log_file: Option<String>,

    /// Run as a visible GUI host for the IronRDP local RPC protocol.
    #[clap(long)]
    rpc: bool,

    /// Override the agent-compatible RPC endpoint (defaults to the per-user agent endpoint).
    #[clap(long, requires = "rpc")]
    rpc_endpoint: Option<String>,

    #[clap(long, value_parser)]
    gw_endpoint: Option<String>,
    #[clap(long, value_parser)]
    gw_user: Option<String>,
    #[clap(long, value_parser)]
    gw_pass: Option<String>,

    /// An address on which the client will connect.
    #[clap(env = "RDP_HOSTNAME")]
    destination: Option<Destination>,

    /// Path to a .rdp file to read the configuration from.
    #[clap(long)]
    rdp_file: Option<PathBuf>,

    /// Arbitrary property override (repeatable): `KEY:TYPE:VALUE`, the same grammar as one `.rdp`
    /// file line (`TYPE` is `i` for integer or `s` for string), e.g. `--prop
    /// enablerdsaadauth:i:1 --prop username:s:admin`. Applied on top of `--rdp-file` but under the
    /// named flags below (e.g. `--username`), which always win for the same key.
    #[clap(long = "prop", value_name = "KEY:TYPE:VALUE")]
    prop: Vec<PropOverride>,

    /// A target RDP server user name
    #[clap(short, long, env = "RDP_USERNAME")]
    username: Option<String>,

    /// An optional target RDP server domain name
    #[clap(short, long)]
    domain: Option<String>,

    /// A target RDP server user password
    #[clap(short, long, env = "RDP_PASSWORD", hide_env_values = true)]
    password: Option<String>,

    /// Proxy URL to connect to for the RDCleanPath
    ///
    /// The accompanying token may be supplied via `--rdcleanpath-token` or entered interactively.
    #[clap(long)]
    rdcleanpath_url: Option<Url>,

    /// Authentication token to insert in the RDCleanPath packet
    #[clap(long, requires("rdcleanpath_url"))]
    rdcleanpath_token: Option<String>,

    /// Connect to a Hyper-V VM console instead of an RDP host, by VM ID (`Get-VM | Select Id`).
    ///
    /// The destination is the Hyper-V host, which listens on port 2179. TLS and CredSSP are
    /// required (do not combine with `--no-tls` / `--no-credssp`). Not supported with
    /// `--gw-endpoint` / RDCleanPath until those transports can target port 2179.
    #[clap(long, value_name = "VM_ID")]
    vmconnect: Option<String>,

    /// Use the Hyper-V basic console instead of Enhanced Session mode.
    #[clap(long, requires = "vmconnect")]
    vmconnect_basic: bool,

    /// Authenticate the Hyper-V host with the current Windows logon token.
    #[cfg(windows)]
    #[clap(long, requires = "vmconnect")]
    vmconnect_current_user: bool,

    /// The keyboard type
    #[clap(long, value_enum, default_value_t = KeyboardType::IbmEnhanced)]
    keyboard_type: KeyboardType,

    /// Enable reliable RDP-UDP for direct connections, falling back to TCP
    #[clap(long, env = "IRONRDP_UDP", value_parser = clap::builder::BoolishValueParser::new(), num_args = 0..=1, default_missing_value = "true")]
    udp: Option<bool>,

    /// Highest RDP-UDP protocol version to offer (1, 2 or 3). Requires --udp
    #[clap(long, env = "IRONRDP_UDP_OFFER", requires = "udp", value_parser = clap::value_parser!(u16).range(1..=3))]
    udp_offer: Option<u16>,

    /// Advertise the graphics pipeline (MS-RDPEGFX); on by default when an H.264 decoder is available
    #[clap(long, env = "IRONRDP_EGFX", value_parser = clap::builder::BoolishValueParser::new(), num_args = 0..=1, default_missing_value = "true")]
    egfx: Option<bool>,

    /// Keep the graphics pipeline's bitmap cache in this file across sessions, so reconnects paint
    /// from cached tiles. Off by default, unlike mstsc; `.rdp` `bitmapcachepersistenable:i:1`
    /// uses a per-user default file.
    #[clap(long, value_name = "PATH")]
    egfx_cache: Option<PathBuf>,

    /// The keyboard subtype (an original equipment manufacturer-dependent value)
    #[clap(long, default_value_t = 0)]
    keyboard_subtype: u32,

    /// The number of function keys on the keyboard
    #[clap(long, default_value_t = 12)]
    keyboard_functional_keys_count: u32,

    /// The input method editor (IME) file name associated with the active input locale
    #[clap(long, default_value_t = String::from(""))]
    ime_file_name: String,

    /// Contains a value that uniquely identifies the client
    #[clap(long, default_value_t = String::from(""))]
    dig_product_id: String,

    /// Scaling factor for desktop applications, percentage (value between 100 and 500)
    #[clap(long, value_parser = clap::value_parser!(u32).range(100..=500))]
    scale_desktop: Option<u32>,

    /// Desired desktop width for the RDP session
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    desktop_width: Option<u16>,

    /// Desired desktop height for the RDP session
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    desktop_height: Option<u16>,

    /// Set required color depth. Currently only 32 and 16 bit color depths are supported
    #[clap(long)]
    color_depth: Option<u32>,

    /// Ignore mouse pointer messages sent by the server. Increases performance when enabled, as the
    /// client could skip costly software rendering of the pointer with alpha blending
    #[clap(long)]
    no_server_pointer: bool,

    /// Automatically logon to the server by passing the INFO_AUTOLOGON flag
    ///
    /// This flag is ignored if CredSSP authentication is used.
    /// You can use `--no-credssp` to ensure it's not.
    #[clap(long)]
    autologon: bool,

    /// Disable TLS + Graphical login (legacy authentication method)
    ///
    /// Disabling this in order to enforce usage of CredSSP (NLA) is recommended.
    #[clap(long)]
    no_tls: bool,

    /// Disable TLS + Network Level Authentication (NLA) using CredSSP
    ///
    /// NLA is used to authenticates RDP clients and servers before sending credentials over the network.
    /// It's not recommended to disable this.
    #[clap(long, alias = "no-nla")]
    no_credssp: bool,

    /// The clipboard type
    #[clap(long, value_enum, default_value_t = ClipboardType::Enable)]
    clipboard_type: ClipboardType,

    /// Enable native MS-RDPEWA WebAuthn redirection (Windows Hello / security keys).
    ///
    /// Defaults to the `.rdp` `redirectwebauthn` value when present, otherwise enabled on
    /// Windows builds that include the client `webauthn` feature.
    #[clap(long, action = clap::ArgAction::SetTrue, overrides_with = "no_webauthn")]
    webauthn: bool,

    /// Disable native MS-RDPEWA WebAuthn redirection.
    #[clap(long = "no-webauthn", action = clap::ArgAction::SetTrue, overrides_with = "webauthn")]
    no_webauthn: bool,

    /// Audio input (microphone)
    ///
    /// Enables the AUDIO_INPUT (MS-RDPEAI) dynamic virtual channel so the local microphone is
    /// redirected to the remote session. Disabled by default.
    #[clap(long)]
    microphone: bool,

    /// Audio output device, by ID or by (part of its) name
    ///
    /// Defaults to the system default device, followed when it changes. `--list-audio-devices`
    /// shows the choices. Overrides the `.rdp` `ironrdp_audiooutputdevice` property.
    #[clap(long, value_name = "DEVICE")]
    audio_output_device: Option<String>,

    /// Audio input (microphone) device, by ID or by (part of its) name
    ///
    /// Defaults to the system default device, followed when it changes. Overrides the `.rdp`
    /// `ironrdp_audioinputdevice` property.
    #[clap(long, value_name = "DEVICE")]
    audio_input_device: Option<String>,

    /// Audio kept buffered ahead of the speakers, in milliseconds (default 100)
    ///
    /// Raise it on networks with high jitter. Overrides the `.rdp` `ironrdp_audiolatency` property.
    #[clap(long, value_name = "MS")]
    audio_latency: Option<u64>,

    /// List the audio output and input devices, then exit
    #[clap(long)]
    list_audio_devices: bool,

    /// Camera to redirect (MS-RDPECAM), by symbolic link or friendly name
    ///
    /// Repeat to redirect several cameras. `*` redirects every camera, including cameras attached
    /// during the session, and a `-` prefix excludes one (`--camera '*' --camera -Integrated`).
    /// Overrides the `.rdp` `camerastoredirect` property, which uses the same entries separated
    /// by `;`. Capture is only available on Windows.
    #[clap(long = "camera", value_name = "CAMERA", allow_hyphen_values = true)]
    cameras: Vec<String>,

    /// Send raw camera frames instead of encoding them to H.264
    ///
    /// Encoding only applies to cameras without a compressed (H.264 or MJPEG) format.
    #[clap(long)]
    camera_raw: bool,

    /// The bitmap codecs to use (remotefx:on, ...)
    #[clap(long, num_args = 1.., value_delimiter = ',')]
    codecs: Vec<String>,

    /// Disable bulk compression support.
    ///
    /// By default the client advertises support for bulk compression and the
    /// server may send compressed PDUs. Pass `--no-compression` to disable it.
    /// When not specified, the value from the `.rdp` file is used (if present),
    /// otherwise compression is enabled by default.
    #[clap(long)]
    no_compression: bool,

    /// Bulk compression level to negotiate with the server.
    ///
    /// Valid values:
    ///   0 — MPPC with 8 KB history (RDP 4.0)
    ///   1 — MPPC with 64 KB history (RDP 5.0)
    ///   2 — NCRUSH (RDP 6.0)
    ///   3 — XCRUSH (RDP 6.1)
    #[clap(long, value_parser = clap::value_parser!(u32).range(0..=3))]
    compression_level: Option<u32>,

    /// Prevents session locking by injecting fake mouse movement events when
    /// the connection is idle (interval in minutes)
    #[clap(long)]
    prevent_session_lock: Option<u32>,

    /// Add DVC channel named pipe proxy
    ///
    /// The format is `<name>=<pipe>`, e.g., `ChannelName=PipeName` where `ChannelName` is the name of the channel,
    /// and `PipeName` is the name of the named pipe to connect to (without OS-specific prefix).
    /// `<pipe>` will automatically be prefixed with `\\.\pipe\` on Windows.
    #[clap(long)]
    dvc_proxy: Vec<DvcProxyInfo>,
    /// Load a DVC client plugin DLL (Windows only).
    ///
    /// Path to a DVC plugin DLL that exports VirtualChannelGetInstance.
    /// Example: C:\Windows\System32\webauthn.dll
    #[cfg(windows)]
    #[clap(long)]
    dvc_plugin: Vec<PathBuf>,

    /// Write the effective PropertySet (merged .rdp file and CLI overrides) to the given path and exit.
    ///
    /// The output is a standard `.rdp` file that can be used as a starting point for customisation
    /// or passed back via `--rdp-file` on the next invocation.
    #[clap(long)]
    dump_rdp: Option<PathBuf>,

    /// Enable Windows WinSCard smartcard redirection (sets `ironrdp_smartcard`).
    ///
    /// On Windows this attaches the native RDPDR backend so smartcard IRPs complete via WinSCard.
    /// Ignored on other platforms except for the property value itself.
    #[clap(long)]
    smartcard: bool,

    /// Redirect the local default Windows printer (same as `redirectprinters:i:1`).
    ///
    /// The remote host needs a printer driver with the announced name, which is the local
    /// queue's driver unless a `--printer-queue <queue>,<driver>` entry overrides it. Ignored on
    /// other platforms.
    #[clap(long, conflicts_with = "printer_all")]
    printer: bool,

    /// Redirect every local Windows printer queue and printer connection.
    ///
    /// `--printer-queue` entries then only override drivers. Ignored on other platforms.
    #[clap(long)]
    printer_all: bool,

    /// Redirect the named local printer queue, optionally announcing another driver.
    ///
    /// The format is `<queue>[,<driver>]`, like FreeRDP `/printer`. Windows printer names cannot
    /// contain commas. Repeat the option for several queues. With `--printer` or `--printer-all`,
    /// entries only override drivers and must name one. Ignored on other platforms.
    #[clap(long, value_name = "QUEUE[,DRIVER]")]
    printer_queue: Vec<String>,

    /// Mark the named redirected queue as the remote default printer.
    ///
    /// Defaults to the local default queue when it is redirected.
    #[clap(long, value_name = "QUEUE")]
    printer_default: Option<String>,

    /// Redirect a local directory as a drive named NAME (repeatable).
    ///
    /// PATH may be a volume root (`E:\`) or a directory on one. Windows only.
    #[clap(long = "drive", value_name = "NAME,PATH")]
    drives: Vec<NamedDrive>,

    /// Redirect every local drive, including drives connected later (`drivestoredirect:s:*`).
    ///
    /// Windows only.
    #[clap(long = "drives")]
    all_drives: bool,

    /// Redirect the user profile directory as a drive named `home`.
    ///
    /// Windows only.
    #[clap(long)]
    home_drive: bool,

    /// Redirect drives connected during the session, such as USB sticks (`DynamicDrives`).
    ///
    /// Windows only.
    #[clap(long)]
    dynamic_drives: bool,

    /// Redirect a local serial port, such as `COM3` (repeatable); `all` redirects every serial port.
    ///
    /// Without this or `--parallel`, the `.rdp` `redirectcomports:i:1` property redirects every
    /// serial and parallel port. Ignored on other platforms.
    #[clap(long, value_name = "PORT")]
    serial: Vec<String>,

    /// Redirect a local parallel port, such as `LPT1` (repeatable); `all` redirects every parallel port.
    ///
    /// Ignored on other platforms.
    #[clap(long, value_name = "PORT")]
    parallel: Vec<String>,

    /// Redirect local USB devices over MS-RDPEUSB, using the FreeRDP `/usb` rule syntax.
    ///
    /// Comma-separated rules, with hexadecimal numbers: `id:VID:PID[#VID:PID...]` and
    /// `addr:BUS:ADDR[#BUS:ADDR...]` select devices present at connection (VID/PID rules also follow
    /// replugs), `auto` redirects devices plugged in during the session, and `filter:CLASS[#CLASS...]`
    /// or `filter:none` replaces the device classes `auto` keeps local (audio, HID, mass storage, hub
    /// and smart card by default). Repeatable; overrides the `.rdp` `usbdevicestoredirect` property.
    /// `--usb list` prints the local devices and exits.
    ///
    /// On Windows, only devices bound to the WinUSB driver can be redirected.
    #[clap(long, value_name = "RULES")]
    usb: Vec<String>,

    /// Launch a RemoteApp program instead of a full desktop (`remoteapplicationprogram`).
    ///
    /// PROGRAM is a published alias such as `||notepad` or a path on the server. Each server
    /// window opens as its own local window.
    #[clap(long, value_name = "PROGRAM")]
    remote_app: Option<String>,

    /// Working directory of the RemoteApp program (`shell working directory`).
    #[clap(long, value_name = "DIR", requires = "remote_app")]
    remote_app_dir: Option<String>,
}

/// A local directory redirected as a named drive (`--drive NAME,PATH`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedDrive {
    /// Drive name shown in the session.
    pub name: String,
    /// Local directory exposed as the drive root.
    pub path: PathBuf,
}

impl core::str::FromStr for NamedDrive {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, path) = s
            .split_once(',')
            .context("missing ',' delimiter in drive specification (expected NAME,PATH)")?;
        let name = name.trim();
        if name.is_empty() || name.contains('\0') {
            anyhow::bail!("drive name must be nonempty and must not contain NUL");
        }
        if path.is_empty() {
            anyhow::bail!("drive path must not be empty");
        }
        Ok(Self {
            name: name.to_owned(),
            path: PathBuf::from(path),
        })
    }
}

/// Which local ports of one kind to redirect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PortChoice {
    /// No port of this kind.
    #[default]
    None,
    /// Every port of this kind present when the session starts.
    All,
    /// The named ports, such as `COM3` or `LPT1`.
    Named(Vec<String>),
}

impl PortChoice {
    fn from_args(values: Vec<String>) -> Self {
        if values.is_empty() {
            Self::None
        } else if values.iter().any(|value| value.eq_ignore_ascii_case("all")) {
            Self::All
        } else {
            Self::Named(values)
        }
    }

    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Serial and parallel ports to redirect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortOptions {
    pub serial: PortChoice,
    pub parallel: PortChoice,
}

impl PortOptions {
    /// Whether any port is requested.
    pub fn is_enabled(&self) -> bool {
        !self.serial.is_none() || !self.parallel.is_none()
    }
}

/// Local printers selected on the command line or through `redirectprinters`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrinterOptions {
    /// Queues to redirect.
    pub selection: PrinterSelection,
    /// `(queue, driver)` overrides, in command-line order.
    pub drivers: Vec<(String, String)>,
    /// Queue marked as the remote default printer.
    pub default_queue: Option<String>,
}

/// Which local printer queues are redirected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PrinterSelection {
    /// The local default queue.
    #[default]
    Default,
    /// Every local queue and printer connection.
    All,
    /// The listed queues.
    Named(Vec<String>),
}

impl PrinterOptions {
    fn from_args(args: &Args, redirect_from_file: bool) -> anyhow::Result<Option<Self>> {
        // `--printer` and `--printer-all` select the queues; `--printer-queue` entries then only
        // override drivers.
        let overrides_only = args.printer || args.printer_all;
        let mut queues = Vec::with_capacity(args.printer_queue.len());
        let mut drivers = Vec::new();
        for spec in &args.printer_queue {
            let (queue, driver) = match spec.split_once(',') {
                Some((queue, driver)) => (queue.trim(), Some(driver.trim())),
                None => (spec.trim(), None),
            };
            anyhow::ensure!(!queue.is_empty(), "--printer-queue needs a queue name: {spec}");
            anyhow::ensure!(
                driver.is_some() || !overrides_only,
                "--printer-queue needs <queue>,<driver> with --printer or --printer-all: {spec}"
            );
            if let Some(driver) = driver {
                anyhow::ensure!(
                    !driver.is_empty() && !driver.contains(','),
                    "--printer-queue expects <queue>[,<driver>]: {spec}"
                );
                drivers.push((queue.to_owned(), driver.to_owned()));
            }
            queues.push(queue.to_owned());
        }

        let selection = if args.printer_all {
            PrinterSelection::All
        } else if args.printer {
            PrinterSelection::Default
        } else if !queues.is_empty() {
            PrinterSelection::Named(queues)
        } else if redirect_from_file {
            PrinterSelection::Default
        } else {
            anyhow::ensure!(
                args.printer_default.is_none(),
                "--printer-default needs --printer, --printer-all, or --printer-queue"
            );
            return Ok(None);
        };

        Ok(Some(Self {
            selection,
            drivers,
            default_queue: args.printer_default.clone(),
        }))
    }
}

/// Result of parsing CLI args + loading the `.rdp` file: a configured [`ConfigBuilder`] plus the
/// CLI-only settings that cannot live on the builder.
///
/// Call [`ViewerConfig::into_config`] to resolve the remaining required fields (interactive prompts
/// + frontend-derived client identity) and build the strongly-typed [`Config`].
pub struct ViewerConfig {
    builder: ConfigBuilder,

    // CLI-only settings that are not representable as `.rdp` file properties.
    log_file: Option<String>,
    dump_rdp: Option<PathBuf>,
    rpc: bool,
    rpc_endpoint: Option<String>,
    printers: Option<PrinterOptions>,
    named_drives: Vec<NamedDrive>,
    ports: PortOptions,
    list_usb_devices: bool,
    list_audio_devices: bool,
}

impl ViewerConfig {
    pub fn parse_args() -> anyhow::Result<Self> {
        Self::from_args(Args::parse())
    }

    /// Parses arguments without exiting the process on a command-line error.
    pub fn parse_from<I, T>(args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::from_args(Args::try_parse_from(args)?)
    }

    fn from_args(mut args: Args) -> anyhow::Result<Self> {
        if args.usb.iter().any(|rules| rules == "list") {
            return Ok(Self {
                builder: ConfigBuilder::new(),
                log_file: args.log_file,
                dump_rdp: None,
                rpc: false,
                rpc_endpoint: None,
                printers: None,
                named_drives: Vec::new(),
                ports: PortOptions::default(),
                list_usb_devices: true,
                list_audio_devices: false,
            });
        }

        let mut properties = ironrdp_propertyset::PropertySet::new();

        if let Some(rdp_file) = &args.rdp_file {
            let input =
                std::fs::read_to_string(rdp_file).with_context(|| format!("failed to read {}", rdp_file.display()))?;

            if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &input) {
                for error in &errors {
                    eprintln!("Warning: skipped entry in {}: {error}", rdp_file.display());
                }
            }
            ConfigBuilder::apply_rdp_launch_companion(rdp_file, &mut properties);
        }

        for over in core::mem::take(&mut args.prop) {
            properties.insert(over.key, over.value);
        }

        let log_file = args.log_file.clone();
        let dump_rdp = args.dump_rdp.clone();
        let list_audio_devices = args.list_audio_devices;

        if args.rpc {
            if args.rdp_file.is_some() || dump_rdp.is_some() {
                anyhow::bail!("--rpc cannot be combined with --rdp-file or --dump-rdp");
            }
            if args.remote_app.is_some() {
                anyhow::bail!("--rpc cannot be combined with --remote-app");
            }
            if !args.drives.is_empty() || args.all_drives || args.home_drive || args.dynamic_drives {
                anyhow::bail!(
                    "--rpc cannot be combined with drive options; pass drivestoredirect in the connect properties"
                );
            }
            if args.printer || args.printer_all || !args.printer_queue.is_empty() || args.printer_default.is_some() {
                anyhow::bail!(
                    "--rpc cannot be combined with printer options; pass redirectprinters in the connect properties"
                );
            }
            if !args.serial.is_empty() || !args.parallel.is_empty() {
                anyhow::bail!(
                    "--rpc cannot be combined with port options; pass redirectcomports in the connect properties"
                );
            }
            return Ok(Self {
                builder: ConfigBuilder::new(),
                log_file,
                dump_rdp: None,
                rpc: true,
                rpc_endpoint: args.rpc_endpoint,
                printers: None,
                named_drives: Vec::new(),
                ports: PortOptions::default(),
                list_usb_devices: false,
                list_audio_devices: false,
            });
        }

        // Same rule as the daemon: `redirectprinters:i:1` redirects the default printer unless
        // `ironrdp_rdpdr:i:0` disables device redirection. CLI printer options always apply.
        let redirect_printers_from_file =
            properties.redirect_printers().unwrap_or(false) && properties.enable_rdpdr() != Some(false);
        let printers = PrinterOptions::from_args(&args, redirect_printers_from_file)?;
        // Unlike printers, mstsc only writes `redirectcomports:i:1` when the user selected ports.
        let mut ports = PortOptions {
            serial: PortChoice::from_args(core::mem::take(&mut args.serial)),
            parallel: PortChoice::from_args(core::mem::take(&mut args.parallel)),
        };
        // Explicit CLI ports take precedence; the file-derived selection honors `ironrdp_rdpdr:i:0`.
        if !ports.is_enabled()
            && properties.redirect_com_ports() == Some(true)
            && properties.enable_rdpdr() != Some(false)
        {
            ports = PortOptions {
                serial: PortChoice::All,
                parallel: PortChoice::All,
            };
        }

        if let Some(value) = properties.drives_to_redirect() {
            for entry in DriveRedirection::unrecognized_entries(value) {
                eprintln!("Warning: ignoring unsupported drivestoredirect entry {entry:?}");
            }
        }
        // `--drives` and `--dynamic-drives` override the `.rdp` drive selection; named drives
        // (`--drive`, `--home-drive`) have no standard `.rdp` representation.
        let drive_redirection = match (args.all_drives, args.dynamic_drives) {
            (true, _) => Some(DriveRedirection::all()),
            (false, true) => Some(properties.drive_redirection().unwrap_or_default().with_dynamic(true)),
            (false, false) => properties.drive_redirection(),
        };
        let mut named_drives = args.drives.clone();
        if args.home_drive {
            let home = std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .context("--home-drive requires USERPROFILE or HOME")?;
            named_drives.push(NamedDrive {
                name: "home".to_owned(),
                path: PathBuf::from(home),
            });
        }
        let cli_drives = args.all_drives || args.dynamic_drives || !named_drives.is_empty();

        if let Some(program) = args.remote_app.take() {
            properties.set_remote_application_mode(true);
            properties.set_remote_application_program(program);
        }
        if let Some(directory) = args.remote_app_dir.take() {
            properties.set_shell_working_directory(directory);
        }

        // The library overlays everything expressible as a `.rdp` property: destination, credentials,
        // transport, channels, desktop size, audio, DVC proxies, etc.
        let builder = ConfigBuilder::from_property_set(&properties)?;

        // Whether the `.rdp` file requested clipboard redirection; the CLI `--clipboard-type` is
        // resolved against this when applied below.
        let redirect_clipboard = properties.redirect_clipboard().unwrap_or(true);
        let redirect_webauthn = properties.redirect_webauthn().unwrap_or(true);
        // Opt-in only: the client feature default for smartcard is `true`, which must not silently
        // attach a WinSCard backend. Require CLI `--smartcard` or an explicit property.
        let enable_smartcard = args.smartcard || properties.enable_smartcard().unwrap_or(false);
        let usb = if args.usb.is_empty() {
            None
        } else {
            let mut usb = UsbRedirectionConfig::default();
            for rules in &args.usb {
                usb.merge(UsbRedirectionConfig::parse_cli(rules).with_context(|| format!("invalid --usb {rules}"))?);
            }
            Some(usb)
        };

        // CLI arguments take precedence: apply them on top of the `.rdp`-derived builder.
        let mut builder = apply_cli_to_builder(builder, args, redirect_clipboard, redirect_webauthn);
        if let Some(usb) = usb {
            builder = builder.with_usb_redirection(Some(usb));
        }
        builder = if enable_smartcard {
            builder.with_rdpdr(true).with_smartcard(true)
        } else {
            builder.with_smartcard(false)
        };
        if printers.is_some() {
            builder = builder.with_rdpdr(true).with_printers(true);
        }
        if let Some(drive_redirection) = drive_redirection {
            builder = builder.with_drive_redirection(drive_redirection);
        }
        if cli_drives {
            builder = builder.with_rdpdr(true);
        }
        if ports.is_enabled() {
            builder = builder.with_rdpdr(true).with_ports(true);
        }

        Ok(Self {
            builder,
            log_file,
            dump_rdp,
            rpc: false,
            rpc_endpoint: None,
            printers,
            named_drives,
            ports,
            list_usb_devices: false,
            list_audio_devices,
        })
    }

    pub fn into_config(self) -> anyhow::Result<Config> {
        // When dumping, the built config is only used to observe the effective, secret-stripped
        // PropertySet; we never start a session. Secrets are stripped on `build()` anyway, so there
        // is no point prompting for them: fill a placeholder instead.
        let dump = self.dump_rdp.is_some();
        prompt_missing(self.builder, dump)
    }

    /// Path to the log file requested on the CLI, if any.
    pub fn log_file(&self) -> Option<&str> {
        self.log_file.as_deref()
    }

    /// Path to dump the effective `.rdp` PropertySet to, if `--dump-rdp` was given.
    pub fn dump_rdp(&self) -> Option<&std::path::Path> {
        self.dump_rdp.as_deref()
    }

    /// Whether the session redirects local printers.
    pub fn redirect_printers(&self) -> bool {
        self.printers.is_some()
    }

    /// Local printers the session redirects, if any.
    pub fn printers(&self) -> Option<&PrinterOptions> {
        self.printers.as_ref()
    }

    /// Local directories requested as named drives (`--drive`, `--home-drive`).
    pub fn named_drives(&self) -> &[NamedDrive] {
        &self.named_drives
    }

    /// Serial and parallel ports the session should redirect.
    pub fn ports(&self) -> &PortOptions {
        &self.ports
    }

    /// Whether `--usb list` asked to print the local USB devices instead of connecting.
    pub fn list_usb_devices(&self) -> bool {
        self.list_usb_devices
    }

    /// Whether the viewer should host the local RPC server.
    pub fn rpc_mode(&self) -> bool {
        self.rpc
    }

    /// Whether to list the audio devices instead of connecting.
    pub fn list_audio_devices(&self) -> bool {
        self.list_audio_devices
    }

    /// Endpoint explicitly selected for the RPC host.
    pub fn rpc_endpoint(&self) -> Option<&str> {
        self.rpc_endpoint.as_deref()
    }
}

/// Print the local USB devices, with the values `--usb` rules match against.
pub fn print_usb_devices() -> anyhow::Result<()> {
    let devices = ironrdp::client::usb::local_devices().context("failed to list USB devices")?;

    println!("{:<11} {:<7} {:<6} {:<10} DEVICE", "ID", "ADDR", "CLASS", "DRIVER");
    for device in devices {
        let identity = &device.identity;
        let name = [device.manufacturer.as_deref(), device.product.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:<11} {:<7} {:<6} {:<10} {name}",
            format!("{:04x}:{:04x}", identity.vendor_id, identity.product_id),
            format!("{:x}:{:x}", identity.bus_number, identity.address),
            format!("{:02x}", identity.device_class),
            device.driver.as_deref().unwrap_or("-"),
        );
    }

    Ok(())
}

/// Apply CLI overrides on top of a builder that already reflects the `.rdp` file. Every flag that is
/// present overwrites the corresponding builder (and mirrored property) value.
fn apply_cli_to_builder(
    mut builder: ConfigBuilder,
    args: Args,
    redirect_clipboard: bool,
    redirect_webauthn: bool,
) -> ConfigBuilder {
    // Validate the codecs early to surface help text before connecting.
    {
        let codecs: Vec<_> = args.codecs.iter().map(String::as_str).collect();
        if let Err(help) = client_codecs_capabilities(&codecs) {
            print!("{help}");
            std::process::exit(0);
        }
    }

    if let Some(destination) = args.destination {
        builder = builder.with_destination(destination);
    }
    if let Some(username) = args.username {
        builder = builder.with_username(username);
    }
    if let Some(password) = args.password {
        builder = builder.with_password(password);
    }
    if let Some(domain) = args.domain {
        builder = builder.with_domain(domain);
    }
    if let Some(scale) = args.scale_desktop {
        builder = builder.with_desktop_scale_factor(scale);
    }
    if let Some(enabled) = args.udp {
        builder = builder.with_udp_transport(enabled);
    }
    if let Some(version) = args.udp_offer {
        use ironrdp::client::config::UdpVersion;
        builder = builder.with_udp_offer_version(match version {
            1 => UdpVersion::V1,
            2 => UdpVersion::V2,
            _ => UdpVersion::V3,
        });
    }
    if let Some(enabled) = args.egfx {
        builder = builder.with_graphics_pipeline(enabled);
    }
    if let Some(path) = args.egfx_cache {
        builder = builder.with_egfx_persistent_cache(Some(path));
    }
    if let Some(width) = args.desktop_width {
        builder = builder.with_desktop_width(width);
    }
    if let Some(height) = args.desktop_height {
        builder = builder.with_desktop_height(height);
    }
    if let Some(color_depth) = args.color_depth {
        builder = builder.with_color_depth(color_depth);
    }
    if args.no_credssp {
        builder = builder.with_credssp(false);
    }
    if args.no_tls {
        builder = builder.with_tls(false);
    }
    if args.no_server_pointer {
        builder = builder.with_server_pointer(false);
    }
    if args.autologon {
        builder = builder.with_autologon(true);
    }
    if args.no_compression {
        builder = builder.with_compression(false);
    }
    if let Some(level) = args.compression_level {
        builder = builder.with_compression_level(level);
    }
    if let Some(minutes) = args.prevent_session_lock {
        builder = builder.with_fake_events_interval(Duration::from_secs(u64::from(minutes) * 60));
    }

    // Transport overrides: RDCleanPath takes precedence over Gateway.
    if let Some(vm_id) = args.vmconnect {
        let mode = if args.vmconnect_basic {
            VmConnectMode::Basic
        } else {
            VmConnectMode::Enhanced
        };
        builder = builder.with_vmconnect_mode(vm_id, mode);
    }
    #[cfg(windows)]
    if args.vmconnect_current_user {
        builder = builder.with_vmconnect_current_user(true);
    }

    if let Some(url) = args.rdcleanpath_url {
        builder = builder.with_transport(TransportKind::RDCleanPath { url });

        if let Some(token) = args.rdcleanpath_token {
            builder = builder.with_rdcleanpath_token(token);
        }
    } else if let Some(endpoint) = args.gw_endpoint {
        builder = builder.with_transport(TransportKind::Gateway {
            endpoint,
            prefer_direct: false,
        });

        if let Some(username) = args.gw_user {
            builder = builder.with_gateway_username(username);
        }
        if let Some(password) = args.gw_pass {
            builder = builder.with_gateway_password(password);
        }
    }

    builder = builder.with_clipboard(resolve_clipboard_type(args.clipboard_type, redirect_clipboard));

    // Viewer enables ironrdp-client `webauthn` through `client-all`.
    let webauthn = if args.webauthn {
        true
    } else if args.no_webauthn {
        false
    } else {
        redirect_webauthn
    };
    builder = builder.with_webauthn(webauthn);

    if args.microphone {
        builder = builder.with_audio_capture(true);
    }
    if let Some(device) = args.audio_output_device {
        builder = builder.with_audio_output_device(Some(device));
    }
    if let Some(device) = args.audio_input_device {
        builder = builder.with_audio_input_device(Some(device));
    }
    if let Some(latency) = args.audio_latency {
        builder = builder.with_audio_latency(Duration::from_millis(latency));
    }

    if !args.cameras.is_empty() || args.camera_raw {
        let mut cameras = builder.cameras().clone();
        if !args.cameras.is_empty() {
            cameras.apply_rdp_value(&args.cameras.join(";"));
        }
        if args.camera_raw {
            cameras.encode_video = false;
        }
        builder = builder.with_cameras(cameras);
    }

    // CLI-only knobs that are not representable as `.rdp` properties.
    // TODO/FIXME: Some of these, we may want to add support for storing in .rdp files (e.g.: IME file name can be reasonably seen as a connection option)
    builder = builder
        .with_keyboard_type(args.keyboard_type.into_pdu())
        .with_keyboard_subtype(args.keyboard_subtype)
        .with_keyboard_functional_keys_count(args.keyboard_functional_keys_count)
        .with_ime_file_name(args.ime_file_name)
        .with_dig_product_id(args.dig_product_id)
        .with_codecs(args.codecs);

    for proxy in args.dvc_proxy {
        builder = builder.with_dvc_pipe_proxy(proxy);
    }

    #[cfg(windows)]
    for plugin in args.dvc_plugin {
        builder = builder.with_dvc_plugin(plugin);
    }

    builder
}

/// Resolve the remaining [`MissingField`]s by prompting for credentials/addresses and deriving the
/// frontend-specific client identity, then build the [`Config`].
///
/// When `dump` is set, the resulting config is only used to observe the effective, secret-stripped
/// PropertySet (no session is started). Secret fields are stripped on `build()` regardless, so they
/// are filled with a placeholder instead of being prompted for.
fn prompt_missing(mut builder: ConfigBuilder, dump: bool) -> anyhow::Result<Config> {
    // Stripped on `build()`, so any value works when only dumping the PropertySet.
    const DUMP_SECRET_PLACEHOLDER: &str = "<stripped>";

    for field in builder.missing() {
        builder = match field {
            MissingField::ServerAddress => {
                let dest = inquire::Text::new("Server address:")
                    .prompt()
                    .context("Address prompt")?
                    .pipe(Destination::new)?;
                builder.with_destination(dest)
            }
            MissingField::Username => {
                let username = inquire::Text::new("Username:").prompt().context("Username prompt")?;
                builder.with_username(username)
            }
            MissingField::Password if dump => builder.with_password(DUMP_SECRET_PLACEHOLDER),
            MissingField::Password => {
                let password = inquire::Password::new("Password:")
                    .without_confirmation()
                    .prompt()
                    .context("Password prompt")?;
                builder.with_password(password)
            }
            MissingField::GatewayUsername => {
                let username = inquire::Text::new("Gateway username:")
                    .prompt()
                    .context("Gateway username prompt")?;
                builder.with_gateway_username(username)
            }
            MissingField::GatewayPassword if dump => builder.with_gateway_password(DUMP_SECRET_PLACEHOLDER),
            MissingField::GatewayPassword => {
                let password = inquire::Password::new("Gateway password:")
                    .without_confirmation()
                    .prompt()
                    .context("Gateway password prompt")?;
                builder.with_gateway_password(password)
            }
            MissingField::RDCleanPathToken if dump => builder.with_rdcleanpath_token(DUMP_SECRET_PLACEHOLDER),
            MissingField::RDCleanPathToken => {
                let token = inquire::Text::new("RDCleanPath token:")
                    .prompt()
                    .context("RDCleanPath token prompt")?;
                builder.with_rdcleanpath_token(token)
            }
            // Frontend-derived identity: never prompted.
            MissingField::ClientBuild => builder.with_client_build(client_build()),
            MissingField::ClientDir => {
                // NOTE: hardcode this value like in freerdp
                // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
                builder.with_client_dir("C:\\Windows\\System32\\mstscax.dll")
            }
            MissingField::Platform => builder.with_platform(current_platform()),
            MissingField::ClientName => builder.with_client_name(client_name()),
        };
    }

    builder.build()
}

fn client_build() -> u32 {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_or(0, |v| v.major * 100 + v.minor * 10 + v.patch)
        .try_into()
        .unwrap_or(0)
}

fn client_name() -> String {
    whoami::hostname().unwrap_or_else(|_| "ironrdp".to_owned())
}

fn current_platform() -> MajorPlatformType {
    match whoami::platform() {
        whoami::Platform::Windows => MajorPlatformType::WINDOWS,
        whoami::Platform::Linux => MajorPlatformType::UNIX,
        whoami::Platform::Mac => MajorPlatformType::MACINTOSH,
        whoami::Platform::Ios => MajorPlatformType::IOS,
        whoami::Platform::Android => MajorPlatformType::ANDROID,
        _ => MajorPlatformType::UNSPECIFIED,
    }
}

pub fn parse_config() -> anyhow::Result<Config> {
    ViewerConfig::parse_args()?.into_config()
}

pub fn parse_config_from<I, T>(args: I) -> anyhow::Result<Config>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    ViewerConfig::parse_from(args)?.into_config()
}

fn resolve_clipboard_type(cli: ClipboardType, redirect_clipboard: bool) -> ResolvedClipboardType {
    if !redirect_clipboard {
        return ResolvedClipboardType::Disable;
    }

    match cli {
        ClipboardType::Enable => ResolvedClipboardType::Enable,
        ClipboardType::Disable => ResolvedClipboardType::Disable,
        ClipboardType::Stub => ResolvedClipboardType::Stub,
    }
}
