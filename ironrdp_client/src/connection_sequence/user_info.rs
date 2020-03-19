use std::{env, net};

use ironrdp::{
    gcc::{
        ClientCoreData, ClientCoreOptionalData, ClientEarlyCapabilityFlags, ClientGccBlocks,
        ClientNetworkData, ClientSecurityData, ColorDepth, ConnectionType, HighColorDepth,
        RdpVersion, SecureAccessSequence, SupportedColorDepths,
    },
    nego::SecurityProtocol,
    rdp::{
        capability_sets::{
            Bitmap, BitmapCache, BitmapCodecs, BitmapDrawingFlags, Brush, CacheDefinition,
            CacheEntry, CaptureFlags, CmdFlags, Codec, CodecProperty, EntropyBits,
            FrameAcknowledge, General, GeneralExtraFlags, GlyphCache, GlyphSupportLevel, Input,
            InputFlags, LargePointer, LargePointerSupportFlags, MajorPlatformType,
            MinorPlatformType, MultifragmentUpdate, OffscreenBitmapCache, Order, OrderFlags,
            OrderSupportExFlags, Pointer, RemoteFxContainer, RfxCaps, RfxCapset,
            RfxClientCapsContainer, RfxICap, RfxICapFlags, Sound, SoundFlags, SupportLevel,
            SurfaceCommands, VirtualChannel, VirtualChannelFlags, BITMAP_CACHE_ENTRIES_NUM,
            GLYPH_CACHE_NUM,
        },
        AddressFamily, BasicSecurityHeader, BasicSecurityHeaderFlags, ClientInfo, ClientInfoFlags,
        ClientInfoPdu, CompressionType, Credentials, ExtendedClientInfo,
        ExtendedClientOptionalInfo, SERVER_CHANNEL_ID,
    },
    CapabilitySet, ClientConfirmActive,
};
use num_traits::ToPrimitive;

use crate::{config::Config, utils::CodecId, RdpError, RdpResult};

const SOURCE_DESCRIPTOR: &str = "IRONRDP";

pub fn create_gcc_blocks(
    config: &Config,
    selected_protocol: SecurityProtocol,
) -> RdpResult<ClientGccBlocks> {
    Ok(ClientGccBlocks {
        core: create_core_data(config, selected_protocol)?,
        security: create_security_data(),
        network: Some(create_network_data()),
        cluster: None,
        monitor: None,
        message_channel: None,
        multi_transport_channel: None,
        monitor_extended: None,
    })
}

pub fn create_client_info_pdu(config: &Config) -> RdpResult<ClientInfoPdu> {
    let security_header = BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::INFO_PKT,
    };
    let client_info = ClientInfo {
        credentials: auth_identity_to_credentials(config.input.credentials.clone()),
        code_page: 0, // ignored if the keyboardLayout field of the Client Core Data is set to zero
        flags: ClientInfoFlags::UNICODE
            | ClientInfoFlags::DISABLE_CTRL_ALT_DEL
            | ClientInfoFlags::LOGON_NOTIFY
            | ClientInfoFlags::LOGON_ERRORS
            | ClientInfoFlags::NO_AUDIO_PLAYBACK
            | ClientInfoFlags::VIDEO_DISABLE,
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

    Ok(ClientInfoPdu {
        security_header,
        client_info,
    })
}

pub fn create_client_confirm_active(
    config: &Config,
    mut server_capability_sets: Vec<CapabilitySet>,
) -> RdpResult<ClientConfirmActive> {
    server_capability_sets.retain(|capability_set| match capability_set {
        CapabilitySet::MultiFragmentUpdate(_) => true,
        _ => false,
    });
    server_capability_sets.extend_from_slice(&[
        create_general_capability_set(),
        create_bitmap_capability_set(config),
        create_orders_capability_set(),
        create_bitmap_cache_capability_set(),
        create_input_capability_set(config),
        create_pointer_capability_set(),
        create_brush_capability_set(),
        create_glyph_cache_capability_set(),
        create_offscreen_bitmap_cache_capability_set(),
        create_virtual_channel_capability_set(),
        create_sound_capability_set(),
        create_large_pointer_capability_set(),
        create_surface_commands_capability_set(),
        create_bitmap_codes_capability_set(),
        CapabilitySet::FrameAcknowledge(FrameAcknowledge {
            max_unacknowledged_frame_count: 2,
        }),
    ]);

    if server_capability_sets
        .iter()
        .find(|c| match c {
            CapabilitySet::MultiFragmentUpdate(_) => true,
            _ => false,
        })
        .is_none()
    {
        server_capability_sets.push(create_multi_fragment_update_capability_set());
    }

    Ok(ClientConfirmActive {
        originator_id: SERVER_CHANNEL_ID,
        pdu: ironrdp::DemandActive {
            source_descriptor: SOURCE_DESCRIPTOR.to_string(),
            capability_sets: server_capability_sets,
        },
    })
}

fn create_core_data(
    config: &Config,
    selected_protocol: SecurityProtocol,
) -> RdpResult<ClientCoreData> {
    Ok(ClientCoreData {
        version: RdpVersion::V5Plus,
        desktop_width: config.width,
        desktop_height: config.height,
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
        optional_data: create_optional_core_data(config, selected_protocol)?,
    })
}

fn create_optional_core_data(
    config: &Config,
    selected_protocol: SecurityProtocol,
) -> RdpResult<ClientCoreOptionalData> {
    Ok(ClientCoreOptionalData {
        post_beta_color_depth: Some(ColorDepth::Bpp4), // ignored
        client_product_id: Some(1),
        serial_number: Some(0),
        high_color_depth: Some(HighColorDepth::Bpp24),
        supported_color_depths: Some(SupportedColorDepths::all()),
        early_capability_flags: Some(
            ClientEarlyCapabilityFlags::VALID_CONNECTION_TYPE
                | ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION,
        ),
        dig_product_id: Some(config.input.dig_product_id.clone()),
        connection_type: Some(ConnectionType::Lan),
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

fn create_network_data() -> ClientNetworkData {
    ClientNetworkData {
        channels: Vec::new(),
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
        extra_flags: GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED
            | GeneralExtraFlags::NO_BITMAP_COMPRESSION_HDR,
        refresh_rect_support: false,
        suppress_output_support: false,
    })
}

fn create_bitmap_capability_set(config: &Config) -> CapabilitySet {
    CapabilitySet::Bitmap(Bitmap {
        pref_bits_per_pix: 32,
        desktop_width: config.width,
        desktop_height: config.height,
        desktop_resize_flag: false,
        drawing_flags: BitmapDrawingFlags::empty(),
    })
}

fn create_orders_capability_set() -> CapabilitySet {
    CapabilitySet::Order(Order::new(
        OrderFlags::NEGOTIATE_ORDER_SUPPORT | OrderFlags::ZERO_BOUNDS_DELTAS_SUPPORT,
        OrderSupportExFlags::empty(),
        0,
        0,
    ))
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
        chunk_size: Some(0), // ignored
    })
}

fn create_sound_capability_set() -> CapabilitySet {
    CapabilitySet::Sound(Sound {
        flags: SoundFlags::empty(),
    })
}

fn create_multi_fragment_update_capability_set() -> CapabilitySet {
    CapabilitySet::MultiFragmentUpdate(MultifragmentUpdate {
        max_request_size: 1024,
    })
}

fn create_large_pointer_capability_set() -> CapabilitySet {
    CapabilitySet::LargePointer(LargePointer {
        flags: LargePointerSupportFlags::UP_TO_96X96_PIXELS,
    })
}

fn create_surface_commands_capability_set() -> CapabilitySet {
    CapabilitySet::SurfaceCommands(SurfaceCommands {
        flags: CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS | CmdFlags::FRAME_MARKER,
    })
}

fn create_bitmap_codes_capability_set() -> CapabilitySet {
    CapabilitySet::BitmapCodecs(BitmapCodecs(vec![Codec {
        id: CodecId::RemoteFx.to_u8().unwrap(),
        property: CodecProperty::RemoteFx(RemoteFxContainer::ClientContainer(
            RfxClientCapsContainer {
                capture_flags: CaptureFlags::empty(),
                caps_data: RfxCaps(RfxCapset(vec![RfxICap {
                    flags: RfxICapFlags::empty(),
                    entropy_bits: EntropyBits::Rlgr3,
                }])),
            },
        )),
    }]))
}

fn auth_identity_to_credentials(auth_identity: sspi::AuthIdentity) -> Credentials {
    Credentials {
        username: auth_identity.username,
        password: auth_identity.password,
        domain: auth_identity.domain,
    }
}
