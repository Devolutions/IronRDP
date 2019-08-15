use std::{env, net};

use ironrdp::{
    gcc::{
        Channel, ChannelOptions, ClientCoreData, ClientCoreOptionalData,
        ClientEarlyCapabilityFlags, ClientGccBlocks, ClientNetworkData, ClientSecurityData,
        ColorDepth, ConnectionType, HighColorDepth, RdpVersion, SecureAccessSequence,
        SupportedColorDepths,
    },
    rdp::{
        capability_sets::{
            Bitmap, BitmapCache, BitmapDrawingFlags, Brush, CacheDefinition, CacheEntry, General,
            GeneralExtraFlags, GlyphCache, GlyphSupportLevel, Input, InputFlags, MajorPlatformType,
            MinorPlatformType, OffscreenBitmapCache, Pointer, Sound, SoundFlags, SupportLevel,
            VirtualChannel, VirtualChannelFlags, BITMAP_CACHE_ENTRIES_NUM, GLYPH_CACHE_NUM,
        },
        AddressFamily, BasicSecurityHeader, BasicSecurityHeaderFlags, ClientInfo, ClientInfoFlags,
        ClientInfoPdu, CompressionType, ExtendedClientInfo, ExtendedClientOptionalInfo,
    },
    CapabilitySet, ClientConfirmActive, SecurityProtocol,
};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{config::Config, utils, RdpError, RdpResult};

const SOURCE_DESCRIPTOR: &str = "IRONRDP";

pub fn create_gcc_blocks(
    config: &Config,
    selected_protocol: SecurityProtocol,
) -> RdpResult<ClientGccBlocks> {
    Ok(ClientGccBlocks {
        core: create_core_data(config, selected_protocol)?,
        security: create_security_data(),
        network: create_network_data(config),
        cluster: None,
        monitor: None,
        message_channel: None,
        multi_transport_channel: None,
        monitor_extended: None,
    })
}

fn create_core_data(
    config: &Config,
    selected_protocol: SecurityProtocol,
) -> RdpResult<ClientCoreData> {
    let current_monitor = get_current_monitor()?;

    Ok(ClientCoreData {
        version: RdpVersion::V5Plus,
        desktop_width: current_monitor.size().width.round() as u16,
        desktop_height: current_monitor.size().height.round() as u16,
        color_depth: ColorDepth::Bpp4, // ignored
        sec_access_sequence: SecureAccessSequence::Del,
        keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
        client_build: semver::Version::parse(clap::crate_version!())
            .map(|version| version.major * 100 + version.minor * 10 + version.patch)
            .unwrap_or(0) as u32,
        client_name: whoami::hostname(),
        keyboard_type: config.input.keyboard_type,
        keyboard_subtype: config.input.keyboard_subtype,
        keyboard_functional_keys_count: config.input.keyboard_functional_keys_count,
        ime_file_name: config.input.ime_file_name.clone(),
        optional_data: create_optional_core_data(config, selected_protocol, current_monitor)?,
    })
}

fn create_optional_core_data(
    config: &Config,
    selected_protocol: SecurityProtocol,
    current_monitor: winit::monitor::MonitorHandle,
) -> RdpResult<ClientCoreOptionalData> {
    Ok(ClientCoreOptionalData {
        post_beta_color_depth: Some(ColorDepth::Bpp4), // ignored
        client_product_id: Some(1),
        serial_number: Some(0),
        high_color_depth: Some(get_color_depth(&current_monitor)),
        supported_color_depths: Some(
            current_monitor
                .video_modes()
                .map(|video_mode| match video_mode.bit_depth() {
                    15 => SupportedColorDepths::BPP15,
                    16 => SupportedColorDepths::BPP16,
                    24 => SupportedColorDepths::BPP24,
                    32 => SupportedColorDepths::BPP32,
                    _ => SupportedColorDepths::empty(),
                })
                .collect(),
        ),
        early_capability_flags: Some(ClientEarlyCapabilityFlags::empty()),
        dig_product_id: Some(config.input.dig_product_id.clone()),
        connection_type: Some(ConnectionType::NotUsed),
        server_selected_protocol: Some(selected_protocol),
        desktop_physical_width: None,
        desktop_physical_height: None,
        desktop_orientation: None,
        desktop_scale_factor: None,
        device_scale_factor: None,
    })
}

fn create_security_data() -> ClientSecurityData {
    ClientSecurityData::no_security()
}

fn create_network_data(config: &Config) -> ClientNetworkData {
    ClientNetworkData {
        channels: config
            .input
            .static_channels
            .iter()
            .map(|name| Channel::new(name.to_string(), ChannelOptions::INITIALIZED))
            .collect(),
    }
}
