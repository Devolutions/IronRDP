#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::num::ParseIntError;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use clap::clap_derive::ValueEnum;
use ironrdp_client::config::{
    ClipboardType as ResolvedClipboardType, Config, ConfigBuilder, Destination, DvcProxyInfo, PropertySet,
    RDCleanPathConfig,
};
use tap::prelude::*;
use url::Url;

/// CLI selection for the clipboard backend.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum ClipboardType {
    Enable,
    Disable,
    Stub,
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
            KeyboardType::IbmEnhanced => ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
            KeyboardType::IbmPcAt => ironrdp::pdu::gcc::KeyboardType::IbmPcAt,
            KeyboardType::IbmPcXt => ironrdp::pdu::gcc::KeyboardType::IbmPcXt,
            KeyboardType::OlivettiIco => ironrdp::pdu::gcc::KeyboardType::OlivettiIco,
            KeyboardType::Nokia1050 => ironrdp::pdu::gcc::KeyboardType::Nokia1050,
            KeyboardType::Nokia9140 => ironrdp::pdu::gcc::KeyboardType::Nokia9140,
            KeyboardType::Japanese => ironrdp::pdu::gcc::KeyboardType::Japanese,
        }
    }
}

pub fn apply_cli_args_to_properties(properties: &mut PropertySet, args: &Args) {
    if let Some(dest) = &args.destination {
        let host = dest
            .name()
            .parse::<core::net::IpAddr>()
            .map(ironrdp_cfg::TargetHost::Ip)
            .unwrap_or_else(|_| ironrdp_cfg::TargetHost::Domain(dest.name().to_owned()));
        properties.insert("full address", format!("{host}:{}", dest.port()));
    }

    if let Some(username) = &args.username {
        properties.insert("username", username.as_str());
    }
    if let Some(password) = &args.password {
        properties.insert("ClearTextPassword", password.as_str());
    }
    if let Some(domain) = &args.domain {
        properties.insert("domain", domain.as_str());
    }
    if let Some(scale) = args.scale_desktop {
        properties.insert("desktopscalefactor", i64::from(scale));
    }
    if let Some(width) = args.desktop_width {
        properties.insert("desktopwidth", i64::from(width));
    }
    if let Some(height) = args.desktop_height {
        properties.insert("desktopheight", i64::from(height));
    }
    if let Some(gw_host) = &args.gw_endpoint {
        properties.insert("gatewayhostname", gw_host.as_str());
        properties.insert(
            "gatewayusagemethod",
            ironrdp_cfg::GatewayUsageMethod::UseAlways.as_i64(),
        );
    }
    if let Some(gw_user) = &args.gw_user {
        properties.insert("gatewayusername", gw_user.as_str());
    }
    if let Some(gw_pass) = &args.gw_pass {
        properties.insert("GatewayPassword", gw_pass.as_str());
    }
    if args.no_credssp {
        properties.insert("enablecredsspsupport", 0i64);
    }
    if let Some(enabled) = args.compression_enabled {
        properties.insert("compression", enabled);
    }
}

fn parse_hex(input: &str) -> Result<u32, ParseIntError> {
    if input.starts_with("0x") {
        u32::from_str_radix(input.get(2..).unwrap_or(""), 16)
    } else {
        input.parse::<u32>()
    }
}

/// Devolutions IronRDP viewer
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP viewer")]
#[clap(version, long_about = None)]
pub struct Args {
    #[clap(short, long, value_parser)]
    pub log_file: Option<String>,

    #[clap(long, value_parser)]
    pub gw_endpoint: Option<String>,
    #[clap(long, value_parser)]
    pub gw_user: Option<String>,
    #[clap(long, value_parser)]
    pub gw_pass: Option<String>,

    pub destination: Option<Destination>,

    #[clap(long)]
    pub rdp_file: Option<PathBuf>,

    #[clap(short, long)]
    pub username: Option<String>,
    #[clap(short, long)]
    pub domain: Option<String>,
    #[clap(short, long)]
    pub password: Option<String>,

    #[clap(long, requires("rdcleanpath_token"))]
    pub rdcleanpath_url: Option<Url>,
    #[clap(long, requires("rdcleanpath_url"))]
    pub rdcleanpath_token: Option<String>,

    #[clap(long, value_enum, default_value_t = KeyboardType::IbmEnhanced)]
    pub keyboard_type: KeyboardType,
    #[clap(long, default_value_t = 0)]
    pub keyboard_subtype: u32,
    #[clap(long, default_value_t = 12)]
    pub keyboard_functional_keys_count: u32,
    #[clap(long, default_value_t = String::from(""))]
    pub ime_file_name: String,
    #[clap(long, default_value_t = String::from(""))]
    pub dig_product_id: String,

    #[clap(long)]
    pub thin_client: bool,
    #[clap(long)]
    pub small_cache: bool,

    #[clap(long, value_parser = clap::value_parser!(u32).range(100..=500))]
    pub scale_desktop: Option<u32>,
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    pub desktop_width: Option<u16>,
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    pub desktop_height: Option<u16>,
    #[clap(long)]
    pub color_depth: Option<u32>,

    #[clap(long)]
    pub no_server_pointer: bool,
    #[clap(long, value_parser = parse_hex, default_value_t = 0)]
    pub capabilities: u32,

    #[clap(long)]
    pub autologon: bool,
    #[clap(long)]
    pub no_tls: bool,
    #[clap(long, alias = "no-nla")]
    pub no_credssp: bool,

    #[clap(long, value_enum, default_value_t = ClipboardType::Enable)]
    pub clipboard_type: ClipboardType,

    #[clap(long, num_args = 1.., value_delimiter = ',')]
    pub codecs: Vec<String>,

    #[clap(long, action = clap::ArgAction::Set)]
    pub compression_enabled: Option<bool>,
    #[clap(long, value_parser = clap::value_parser!(u32).range(0..=3), default_value_t = 3)]
    pub compression_level: u32,

    #[clap(long)]
    pub prevent_session_lock: Option<u32>,

    #[clap(long)]
    pub dvc_proxy: Vec<DvcProxyInfo>,
    #[cfg(windows)]
    #[clap(long)]
    pub dvc_plugin: Vec<PathBuf>,

    #[clap(long)]
    pub dump_rdp: Option<PathBuf>,
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

/// Result of phase 1 parsing: the merged PropertySet plus CLI-only settings.
pub struct ParsedInputs {
    pub properties: PropertySet,
    pub args: Args,
    pub rdcleanpath: Option<RDCleanPathConfig>,
}

pub fn parse_inputs() -> anyhow::Result<ParsedInputs> {
    parse_inputs_from(std::env::args_os())
}

pub fn parse_inputs_from<I, T>(args_iter: I) -> anyhow::Result<ParsedInputs>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let args = Args::parse_from(args_iter);

    let mut properties = PropertySet::new();

    if let Some(rdp_file) = &args.rdp_file {
        let input =
            std::fs::read_to_string(rdp_file).with_context(|| format!("failed to read {}", rdp_file.display()))?;

        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &input) {
            for error in &errors {
                eprintln!("Warning: skipped entry in {}: {error}", rdp_file.display());
            }
        }
    }

    apply_cli_args_to_properties(&mut properties, &args);

    let rdcleanpath = args
        .rdcleanpath_url
        .as_ref()
        .zip(args.rdcleanpath_token.as_ref())
        .map(|(url, auth_token)| RDCleanPathConfig {
            url: url.clone(),
            auth_token: auth_token.clone(),
        });

    Ok(ParsedInputs {
        properties,
        args,
        rdcleanpath,
    })
}

/// Phase-2: turn parsed inputs into a typed [`Config`]; prompts via `inquire` for missing fields.
pub fn build_config(parsed: ParsedInputs) -> anyhow::Result<Config> {
    use ironrdp_cfg::PropertySetExt as _;

    let ParsedInputs {
        mut properties,
        args,
        rdcleanpath,
    } = parsed;

    // Prompt for missing credentials / server before handing off to the library builder.
    if properties.username().is_none() {
        let username = inquire::Text::new("Username:").prompt().context("username prompt")?;
        properties.insert("username", username);
    }
    if properties.clear_text_password().is_none() {
        let password = inquire::Password::new("Password:")
            .without_confirmation()
            .prompt()
            .context("password prompt")?;
        properties.insert("ClearTextPassword", password);
    }
    let server_known = properties
        .full_address()
        .ok()
        .flatten()
        .or_else(|| properties.alternate_full_address().ok().flatten())
        .is_some();
    if !server_known {
        let server = inquire::Text::new("Server address:")
            .prompt()
            .context("address prompt")?;
        let dest = server.pipe(Destination::new)?;
        let host = dest
            .name()
            .parse::<core::net::IpAddr>()
            .map(ironrdp_cfg::TargetHost::Ip)
            .unwrap_or_else(|_| ironrdp_cfg::TargetHost::Domain(dest.name().to_owned()));
        properties.insert("full address", format!("{host}:{}", dest.port()));
    }

    // Gateway prompts.
    let has_gw_host = properties.gateway_hostname().is_some();
    let use_gateway = properties
        .gateway_usage_method()
        .ok()
        .flatten()
        .map_or(has_gw_host, ironrdp_cfg::GatewayUsageMethod::is_gateway_required);
    if use_gateway && has_gw_host {
        if properties.gateway_username().is_none() {
            let v = inquire::Text::new("Gateway username:")
                .prompt()
                .context("gateway username prompt")?;
            properties.insert("gatewayusername", v);
        }
        if properties.gateway_password().is_none() {
            let v = inquire::Password::new("Gateway password:")
                .without_confirmation()
                .prompt()
                .context("gateway password prompt")?;
            properties.insert("GatewayPassword", v);
        }
    }

    let redirect_clipboard = properties.redirect_clipboard().unwrap_or(true);
    let clipboard_type = resolve_clipboard_type(args.clipboard_type, redirect_clipboard);

    let mut builder = ConfigBuilder::from_property_set(&properties)
        .with_log_file(args.log_file)
        .with_codecs(args.codecs)
        .with_color_depth(args.color_depth)
        .with_capabilities(args.capabilities)
        .with_no_tls(args.no_tls)
        .with_autologon(args.autologon)
        .with_no_server_pointer(args.no_server_pointer)
        .with_thin_client(args.thin_client)
        .with_small_cache(args.small_cache)
        .with_compression_level(Some(args.compression_level))
        .with_prevent_session_lock_minutes(args.prevent_session_lock)
        .with_clipboard_type(clipboard_type)
        .with_rdcleanpath(rdcleanpath)
        .with_keyboard_type(args.keyboard_type.into_pdu())
        .with_keyboard_subtype(args.keyboard_subtype)
        .with_keyboard_functional_keys_count(args.keyboard_functional_keys_count)
        .with_ime_file_name(args.ime_file_name)
        .with_dig_product_id(args.dig_product_id)
        .with_dvc_pipe_proxies(args.dvc_proxy);

    #[cfg(windows)]
    {
        builder = builder.with_dvc_plugins(args.dvc_plugin);
    }

    builder.build()
}

pub fn parse_config() -> anyhow::Result<Config> {
    build_config(parse_inputs()?)
}

pub fn parse_config_from<I, T>(args: I) -> anyhow::Result<Config>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    build_config(parse_inputs_from(args)?)
}
