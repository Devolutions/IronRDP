//! Shared logic to rebuild an IronRDP session for `IRDPREC1` replay.
//!
//! A replay has no live server, so it cannot obtain a [`ConnectionResult`] from the connector. This
//! crate reconstructs one from the recorded manifest values (the struct is directly constructible),
//! so the native (`ironrdp-replay-bench`) and .NET (FFI) replays build **identical** sessions and
//! therefore reproduce the same framebuffer checksum. See
//! `docs/plans/2026-06-03-ironrdp-benchmark-design.md`.

// Channel IDs are small and assigned sequentially; overflow is impossible in practice.
#![allow(clippy::arithmetic_side_effects)]

use core::any::TypeId;

use ironrdp::connector::connection_activation::ConnectionActivationSequence;
use ironrdp::connector::{Config as ConnectorConfig, ConnectionResult, Credentials, DesktopSize};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::dvc::DrdynvcClient;
use ironrdp::echo::client::EchoClient;
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};
use ironrdp::rdpdr::{NoopRdpdrBackend, Rdpdr};
use ironrdp::rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};
use ironrdp::svc::StaticChannelSet;

/// A negotiated static virtual channel from the manifest: RDP name + assigned channel ID.
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub name: String,
    pub id: u16,
}

/// Everything needed to rebuild a [`ConnectionResult`] for replay.
#[derive(Debug, Clone)]
pub struct ReplayParams {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    pub share_id: u32,
    pub desktop_width: u16,
    pub desktop_height: u16,
    pub enable_server_pointer: bool,
    pub pointer_software_rendering: bool,
    pub compression_type: Option<CompressionType>,
    pub channels: Vec<ChannelEntry>,
}

/// Maps the recorded `compression_type` Debug string (`"K8"`, `"Rdp61"`, …) back to the enum.
pub fn parse_compression(name: Option<&str>) -> Option<CompressionType> {
    match name {
        Some("K8") => Some(CompressionType::K8),
        Some("K64") => Some(CompressionType::K64),
        Some("Rdp6") => Some(CompressionType::Rdp6),
        Some("Rdp61") => Some(CompressionType::Rdp61),
        _ => None,
    }
}

/// Rebuilds the recorded static virtual channel set with matching channel IDs, so the x224 processor
/// routes captured slow-path/DVC traffic instead of rejecting unknown channel IDs. Graphics arrive
/// over fast-path (channel-independent), so no-op audio/device backends are sufficient — these
/// channels only need to consume their auxiliary PDUs without error.
fn build_static_channels(channels: &[ChannelEntry]) -> StaticChannelSet {
    let mut set = StaticChannelSet::new();
    for ch in channels {
        match ch.name.as_str() {
            "drdynvc" => {
                // Match the recorder's dynamic channels so DVC create/data PDUs are accepted.
                set.insert(
                    DrdynvcClient::new()
                        .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
                        .with_dynamic_channel(EchoClient::new()),
                );
                set.attach_channel_id(TypeId::of::<DrdynvcClient>(), ch.id);
            }
            "rdpsnd" => {
                set.insert(Rdpsnd::new(Box::new(NoopRdpsndBackend)));
                set.attach_channel_id(TypeId::of::<Rdpsnd>(), ch.id);
            }
            "rdpdr" => {
                set.insert(Rdpdr::new(Box::new(NoopRdpdrBackend {}), "IronRDP".to_owned()).with_smartcard(0));
                set.attach_channel_id(TypeId::of::<Rdpdr>(), ch.id);
            }
            _ => {
                // Unknown channel: skip. Its traffic (if any) will surface as a decode error, which is
                // the correct signal that the corpus needs a handler here.
            }
        }
    }
    set
}

/// Minimal connector config, consumed only by [`ConnectionActivationSequence::new`] (touched on
/// replay solely if the capture contains a Deactivate-All/reactivation, which benchmark captures
/// avoid). Default values are therefore sufficient.
fn replay_config(desktop_size: DesktopSize, compression_type: Option<CompressionType>) -> ConnectorConfig {
    ConnectorConfig {
        desktop_size,
        desktop_scale_factor: 0,
        enable_tls: false,
        enable_credssp: true,
        credentials: Credentials::UsernamePassword {
            username: String::new(),
            password: String::new(),
        },
        domain: None,
        client_build: 0,
        client_name: "ironrdp-replay".to_owned(),
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: MajorPlatformType::WINDOWS,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        performance_flags: PerformanceFlags::default(),
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
    }
}

/// Builds a [`ConnectionResult`] from recorded manifest values, ready to pass to
/// `ActiveStage::new`. Pointer settings mirror the recording so the framebuffer evolves identically
/// (server-driven pointer updates are deterministic on replay; only software-rendered pointers touch
/// the framebuffer).
pub fn build_connection_result(params: &ReplayParams) -> ConnectionResult {
    let desktop_size = DesktopSize {
        width: params.desktop_width,
        height: params.desktop_height,
    };

    ConnectionResult {
        io_channel_id: params.io_channel_id,
        user_channel_id: params.user_channel_id,
        share_id: params.share_id,
        static_channels: build_static_channels(&params.channels),
        desktop_size,
        enable_server_pointer: params.enable_server_pointer,
        pointer_software_rendering: params.pointer_software_rendering,
        connection_activation: ConnectionActivationSequence::new(
            replay_config(desktop_size, params.compression_type),
            params.io_channel_id,
            params.user_channel_id,
        ),
        compression_type: params.compression_type,
    }
}

/// CRC32 over the canonical framebuffer (RGBA with alpha masked to `0xFF`), used as the deterministic
/// correctness gate. MUST match `ironrdp_client::record::framebuffer_crc32`. `data` is the
/// `DecodedImage` RGBA buffer.
pub fn framebuffer_crc32(data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    let mut pixel = [0u8; 4];
    for chunk in data.chunks_exact(4) {
        pixel[0] = chunk[0];
        pixel[1] = chunk[1];
        pixel[2] = chunk[2];
        pixel[3] = 0xFF;
        hasher.update(&pixel);
    }
    hasher.finalize()
}
