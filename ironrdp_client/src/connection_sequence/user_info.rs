use std::{env, net};

use ironrdp::{
    gcc::{
        Channel, ChannelOptions, ClientCoreData, ClientCoreOptionalData,
        ClientEarlyCapabilityFlags, ClientGccBlocks, ClientNetworkData, ClientSecurityData,
        ColorDepth, ConnectionType, HighColorDepth, RdpVersion, SecureAccessSequence,
        SupportedColorDepths,
    },
    nego::SecurityProtocol,
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
    CapabilitySet, ClientConfirmActive,
};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{config::Config, RdpError, RdpResult};

const SOURCE_DESCRIPTOR: &str = "IRONRDP";

pub fn create_gcc_blocks(
    config: &Config,
    selected_protocol: SecurityProtocol,
) -> RdpResult<ClientGccBlocks> {
    Ok(ClientGccBlocks {
        core: create_core_data(config, selected_protocol)?,
        security: create_security_data(),
        network: Some(create_network_data(config)),
        cluster: None,
        monitor: None,
        message_channel: None,
        multi_transport_channel: None,
        monitor_extended: None,
    })
}

pub fn create_client_info_pdu(config: &Config) -> RdpResult<ClientInfoPdu> {
    let security_header = BasicSecurityHeader::new(BasicSecurityHeaderFlags::INFO_PKT);
    let client_info = ClientInfo {
        credentials: config.input.credentials.clone(),
        code_page: 0, // ignored if the keyboardLayout field of the Client Core Data is set to zero
        flags: ClientInfoFlags::UNICODE,
        compression_type: CompressionType::K8, // ignored if ClientInfoFlags::COMPRESSION is not set
        alternate_shell: String::new(),
        work_dir: String::new(),
        extra_info: ExtendedClientInfo {
            address_family: match config.routing_addr {
                net::SocketAddr::V4(_) => AddressFamily::INet,
                net::SocketAddr::V6(_) => AddressFamily::INet6,
            },
            address: config.routing_addr.ip().to_string(),
            dir: env::current_dir()
                .map_err(|e| {
                    RdpError::UserInfoError(format!(
                        "Failed to get current directory path: {:?}",
                        e
                    ))
                })?
                .to_string_lossy()
                .to_string(),
            optional_data: ExtendedClientOptionalInfo::default(),
        },
    };

    Ok(ClientInfoPdu::new(security_header, client_info))
}

pub fn create_client_confirm_active(
    config: &Config,
    mut server_capability_sets: Vec<CapabilitySet>,
) -> RdpResult<ClientConfirmActive> {
    let current_monitor = get_current_monitor()?;

    server_capability_sets.retain(|capability_set| match capability_set {
        CapabilitySet::Order(_) => true,
        _ => false,
    });
    server_capability_sets.extend_from_slice(&[
        create_general_capability_set(),
        create_bitmap_capability_set(&current_monitor),
        create_bitmap_cache_capability_set(),
        create_input_capability_set(config),
        create_pointer_capability_set(),
        create_brush_capability_set(),
        create_glyph_cache_capability_set(),
        create_offscreen_bitmap_cache_capability_set(),
        create_virtual_channel_capability_set(),
        create_sound_capability_set(),
    ]);

    Ok(ClientConfirmActive::new(ironrdp::DemandActive::new(
        SOURCE_DESCRIPTOR.to_string(),
        server_capability_sets,
    )))
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

fn create_general_capability_set() -> CapabilitySet {
    CapabilitySet::General(General {
        major_platform_type: match whoami::platform() {
            whoami::Platform::Windows => MajorPlatformType::Windows,
            whoami::Platform::Linux => MajorPlatformType::Unix,
            whoami::Platform::MacOS => MajorPlatformType::Macintosh,
            whoami::Platform::Ios => MajorPlatformType::IOs,
            whoami::Platform::Android => MajorPlatformType::Android,
            _ => MajorPlatformType::Unspecified,
        },
        minor_platform_type: MinorPlatformType::Unspecified,
        extra_flags: GeneralExtraFlags::empty(),
        refresh_rect_support: false,
        suppress_output_support: false,
    })
}

fn create_bitmap_capability_set(current_monitor: &winit::monitor::MonitorHandle) -> CapabilitySet {
    CapabilitySet::Bitmap(Bitmap {
        pref_bits_per_pix: get_color_depth(current_monitor).to_u16().unwrap(),
        desktop_width: current_monitor.size().width.round() as u16,
        desktop_height: current_monitor.size().height.round() as u16,
        desktop_resize_flag: false,
        drawing_flags: BitmapDrawingFlags::empty(),
    })
}

fn create_bitmap_cache_capability_set() -> CapabilitySet {
    CapabilitySet::BitmapCache(BitmapCache {
        caches: [CacheEntry {
            entries: 0,
            max_cell_size: 0,
        }; BITMAP_CACHE_ENTRIES_NUM],
    })
}

fn create_pointer_capability_set() -> CapabilitySet {
    CapabilitySet::Pointer(Pointer {
        color_pointer_cache_size: 0,
        pointer_cache_size: 0,
    })
}

fn create_input_capability_set(config: &Config) -> CapabilitySet {
    CapabilitySet::Input(Input {
        input_flags: InputFlags::SCANCODES,
        keyboard_layout: 0,
        keyboard_type: Some(config.input.keyboard_type),
        keyboard_subtype: config.input.keyboard_subtype,
        keyboard_function_key: config.input.keyboard_functional_keys_count,
        keyboard_ime_filename: config.input.ime_file_name.clone(),
    })
}

fn create_brush_capability_set() -> CapabilitySet {
    CapabilitySet::Brush(Brush {
        support_level: SupportLevel::Default,
    })
}

fn create_glyph_cache_capability_set() -> CapabilitySet {
    CapabilitySet::GlyphCache(GlyphCache {
        glyph_cache: [CacheDefinition {
            entries: 0,
            max_cell_size: 0,
        }; GLYPH_CACHE_NUM],
        frag_cache: CacheDefinition {
            entries: 0,
            max_cell_size: 0,
        },
        glyph_support_level: GlyphSupportLevel::None,
    })
}

fn create_offscreen_bitmap_cache_capability_set() -> CapabilitySet {
    CapabilitySet::OffscreenBitmapCache(OffscreenBitmapCache {
        is_supported: false,
        cache_size: 0,
        cache_entries: 0,
    })
}

fn create_virtual_channel_capability_set() -> CapabilitySet {
    CapabilitySet::VirtualChannel(VirtualChannel {
        flags: VirtualChannelFlags::NO_COMPRESSION,
        chunk_size: 0, // ignored
    })
}

fn create_sound_capability_set() -> CapabilitySet {
    CapabilitySet::Sound(Sound {
        flags: SoundFlags::empty(),
    })
}

fn get_current_monitor() -> RdpResult<winit::monitor::MonitorHandle> {
    Ok(
        winit::window::Window::new(&winit::event_loop::EventLoop::new())
            .map_err(|e| RdpError::UserInfoError(format!("Failed to create window: {:?}", e)))?
            .current_monitor(),
    )
}

fn get_color_depth(current_monitor: &winit::monitor::MonitorHandle) -> HighColorDepth {
    current_monitor
        .video_modes()
        .map(|video_mode| {
            let video_mode_bit_depth = video_mode.bit_depth();
            match HighColorDepth::from_u16(video_mode_bit_depth) {
                Some(bit_depth) => bit_depth,
                None if video_mode_bit_depth == 32 => HighColorDepth::Bpp24,
                _ => HighColorDepth::Bpp4,
            }
        })
        .max()
        .unwrap_or(HighColorDepth::Bpp4)
}
