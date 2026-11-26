use ironrdp::pdu::rdp::client_info::PerformanceFlags;

use self::ffi::PerformanceFlagsType;

#[cfg_attr(
    not(windows),
    expect(dead_code, reason = "redirected-drive settings are consumed only by Windows builds")
)]
#[derive(Clone)]
pub(crate) struct RedirectedDriveConfig {
    pub(crate) device_id: u32,
    pub(crate) display_name: String,
    pub(crate) root_path: String,
    pub(crate) read_only: bool,
}

#[cfg(any(windows, test))]
/// Returns the drive name as announced in DeviceData, without a trailing colon and uppercased, for duplicate detection.
///
/// The uppercase fold and the dropped trailing colon are a deliberately conservative client-side policy:
/// Windows resolves `\\tsclient\<name>` case-insensitively,
/// [MS-RDPEFS] 2.2.1.3, "Device Announce Header (DEVICE_ANNOUNCE)", allows a colon only at the end of a device name
/// without saying whether the server keeps it,
/// and both can only reject more names, never accept a colliding one.
///
/// [MS-RDPEFS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/32e34332-774b-4ead-8c9d-5d64720d6bf9
fn drive_wire_name(name: &str) -> String {
    // The device ID is only used to retrieve the single entry in this temporary collection.
    let mut devices = ironrdp::rdpdr::pdu::efs::Devices::new();
    devices.add_drive(0, name.to_owned());
    let data = devices.device(0).expect("the drive was just added").device_data();
    let data = data.strip_suffix(&[0]).unwrap_or(data);
    let data = data.strip_suffix(b":").unwrap_or(data);
    String::from_utf8_lossy(data).to_uppercase()
}

/// The bitmap settings used when the caller sets none, matching the native client.
///
/// Without a codec on offer, the server falls back to sending the screen as nearly raw bitmaps,
/// which is so much data that it starts dropping frames. Offering RemoteFX fixes that.
fn default_bitmap_config() -> Result<ironrdp::connector::BitmapConfig, Box<crate::error::ffi::IronRdpError>> {
    let codecs = ironrdp::pdu::rdp::capability_sets::client_codecs_capabilities(&[])
        .map_err(|help| -> Box<crate::error::ffi::IronRdpError> { help.as_str().into() })?;

    Ok(ironrdp::connector::BitmapConfig {
        color_depth: 32,
        lossy_compression: true,
        codecs,
    })
}

// `ironrdp::pdu::gcc::KeyboardType` is a permissive newtype (MS-RDPBCGR's value table for this
// field has grown across revisions), not a fieldless enum, so `diplomat::enum_convert` no longer
// applies to it; this conversion is written by hand instead. It must live outside the
// `#[diplomat::bridge]` module below, since that macro only understands inherent impls on the
// bridged types, not trait impls. The FFI-facing enum keeps its existing fixed set of variants
// unchanged.
impl From<ffi::KeyboardType> for ironrdp::pdu::gcc::KeyboardType {
    fn from(value: ffi::KeyboardType) -> Self {
        match value {
            ffi::KeyboardType::IbmPcXt => Self::IBM_PC_XT,
            ffi::KeyboardType::OlivettiIco => Self::OLIVETTI_ICO,
            ffi::KeyboardType::IbmPcAt => Self::IBM_PC_AT,
            ffi::KeyboardType::IbmEnhanced => Self::IBM_ENHANCED,
            ffi::KeyboardType::Nokia1050 => Self::NOKIA_1050,
            ffi::KeyboardType::Nokia9140 => Self::NOKIA_9140,
            ffi::KeyboardType::Japanese => Self::JAPANESE,
        }
    }
}

#[diplomat::bridge]
pub mod ffi {
    use ironrdp::connector::Credentials;
    use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;

    use crate::dvc::direct_channel::ffi::DirectDvcConfig;
    use crate::dvc::ffi::DvcPipeProxyConfig;
    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct Config {
        pub(crate) connector: ironrdp::connector::Config,
        pub(crate) dvc_pipe_proxy: Option<DvcPipeProxyConfig>,
        pub(crate) direct_dvc: Option<DirectDvcConfig>,
        pub(crate) redirected_drives: Vec<super::RedirectedDriveConfig>,
    }

    impl Config {
        pub fn get_builder() -> Box<ConfigBuilder> {
            Box::<ConfigBuilder>::default()
        }

        #[diplomat::attr(auto, getter = "dvc_pipe_proxy")]
        pub fn get_dvc_pipe_proxy(&self) -> Option<Box<DvcPipeProxyConfig>> {
            self.dvc_pipe_proxy.as_ref().map(|dvc| Box::new(dvc.clone()))
        }

        #[diplomat::attr(auto, getter = "direct_dvc")]
        pub fn get_direct_dvc(&self) -> Option<Box<DirectDvcConfig>> {
            self.direct_dvc.as_ref().map(|dvc| Box::new(dvc.clone()))
        }
    }

    #[expect(
        clippy::partial_pub_fields,
        reason = "Diplomat exposes this opaque builder through methods only"
    )]
    #[derive(Default)]
    #[diplomat::opaque_mut]
    pub struct ConfigBuilder {
        pub credentials: Option<Credentials>,
        pub domain: Option<String>,
        pub enable_tls: Option<bool>,
        pub enable_credssp: Option<bool>,
        pub keyboard_type: Option<ironrdp::pdu::gcc::KeyboardType>,
        pub keyboard_subtype: Option<u32>,
        pub keyboard_layout: Option<u32>,
        pub keyboard_functional_keys_count: Option<u32>,
        pub ime_file_name: Option<String>,
        pub dig_product_id: Option<String>,
        pub desktop_size: Option<ironrdp::connector::DesktopSize>,
        pub bitmap: Option<ironrdp::connector::BitmapConfig>,
        pub client_build: Option<u32>,
        pub client_name: Option<String>,
        pub client_dir: Option<String>,
        pub platform: Option<MajorPlatformType>,
        pub enable_server_pointer: Option<bool>,
        pub autologon: Option<bool>,
        pub audio_playback_mode: Option<AudioPlaybackMode>,
        pub pointer_software_rendering: Option<bool>,
        pub performance_flags: Option<ironrdp::pdu::rdp::client_info::PerformanceFlags>,
        pub timezone_info: Option<ironrdp::pdu::rdp::client_info::TimezoneInfo>,
        pub dvc_pipe_proxy: Option<DvcPipeProxyConfig>,
        pub direct_dvc: Option<DirectDvcConfig>,
        redirected_drives: Vec<super::RedirectedDriveConfig>,
        /// Sanitized names of `redirected_drives`, kept so each addition checks for collisions without re-deriving them.
        #[cfg(windows)]
        redirected_drive_wire_names: Vec<String>,
    }

    /// Where the server plays audio (the `audiomode` `.rdp` setting).
    #[diplomat::enum_convert(ironrdp::connector::AudioPlaybackMode)]
    pub enum AudioPlaybackMode {
        /// Redirect audio to this computer.
        ///
        /// This is also the default.
        /// The connector asks for audio only when a playback channel is attached when the connection starts,
        /// not counting the silent RDPSND channel installed for drive or smartcard redirection.
        /// Otherwise it asks for no audio.
        Client,
        /// Leave audio playing on the remote computer.
        Server,
        /// Do not play audio.
        Disabled,
    }

    pub enum KeyboardType {
        IbmPcXt,
        OlivettiIco,
        IbmPcAt,
        IbmEnhanced,
        Nokia1050,
        Nokia9140,
        Japanese,
    }

    #[diplomat::opaque]
    pub struct DesktopSize(pub ironrdp::connector::DesktopSize);

    impl DesktopSize {
        #[diplomat::attr(auto, getter = "width")]
        pub fn get_width(&self) -> u16 {
            self.0.width
        }

        #[diplomat::attr(auto, getter = "height")]
        pub fn get_height(&self) -> u16 {
            self.0.height
        }
    }

    impl ConfigBuilder {
        pub fn new() -> Box<Self> {
            Box::<ConfigBuilder>::default()
        }

        pub fn with_username_and_password(&mut self, username: &str, password: &str) {
            self.credentials = Some(Credentials::UsernamePassword {
                username: username.to_owned(),
                password: password.to_owned(),
            });
        }

        #[diplomat::attr(auto, setter = "domain")]
        pub fn set_domain(&mut self, domain: &str) {
            self.domain = Some(domain.to_owned());
        }

        #[diplomat::attr(auto, setter = "enable_tls")]
        pub fn set_enable_tls(&mut self, enable_tls: bool) {
            self.enable_tls = Some(enable_tls);
        }

        #[diplomat::attr(auto, setter = "enable_credssp")]
        pub fn set_enable_credssp(&mut self, enable_credssp: bool) {
            self.enable_credssp = Some(enable_credssp);
        }

        #[diplomat::attr(auto, setter = "keyboard_layout")]
        pub fn set_keyboard_layout(&mut self, keyboard_layout: u32) {
            self.keyboard_layout = Some(keyboard_layout);
        }

        #[diplomat::attr(auto, setter = "keyboard_type")]
        pub fn set_keyboard_type(&mut self, keyboard_type: KeyboardType) {
            self.keyboard_type = Some(keyboard_type.into());
        }

        #[diplomat::attr(auto, setter = "keyboard_subtype")]
        pub fn set_keyboard_subtype(&mut self, keyboard_subtype: u32) {
            self.keyboard_subtype = Some(keyboard_subtype);
        }

        #[diplomat::attr(auto, setter = "keyboard_functional_keys_count")]
        pub fn set_keyboard_functional_keys_count(&mut self, keyboard_functional_keys_count: u32) {
            self.keyboard_functional_keys_count = Some(keyboard_functional_keys_count);
        }

        #[diplomat::attr(auto, setter = "ime_file_name")]
        pub fn set_ime_file_name(&mut self, ime_file_name: &str) {
            self.ime_file_name = Some(ime_file_name.to_owned());
        }

        #[diplomat::attr(auto, setter = "dig_product_id")]
        pub fn set_dig_product_id(&mut self, dig_product_id: &str) {
            self.dig_product_id = Some(dig_product_id.to_owned());
        }

        pub fn set_desktop_size(&mut self, height: u16, width: u16) {
            self.desktop_size = Some(ironrdp::connector::DesktopSize { width, height });
        }

        #[diplomat::attr(auto, setter = "performance_flags")]
        pub fn set_performance_flags(&mut self, performance_flags: &PerformanceFlags) {
            self.performance_flags = Some(performance_flags.0);
        }

        // pub fn set_timezone_info(&mut self, timezone_info: Option<ironrdp::pdu::rdp::client_info::TimezoneInfo>) {
        //     self.timezone_info = timezone_info;
        // }

        #[diplomat::attr(auto, setter = "bitmap_config")]
        pub fn set_bitmap_config(&mut self, bitmap: &BitmapConfig) {
            self.bitmap = Some(bitmap.0.clone());
        }

        #[diplomat::attr(auto, setter = "client_build")]
        pub fn set_client_build(&mut self, client_build: u32) {
            self.client_build = Some(client_build);
        }

        #[diplomat::attr(auto, setter = "client_name")]
        pub fn set_client_name(&mut self, client_name: &str) {
            self.client_name = Some(client_name.to_owned());
        }

        #[diplomat::attr(auto, setter = "client_dir")]
        pub fn set_client_dir(&mut self, client_dir: &str) {
            self.client_dir = Some(client_dir.to_owned());
        }

        #[diplomat::attr(auto, setter = "enable_server_pointer")]
        pub fn set_enable_server_pointer(&mut self, enable_server_pointer: bool) {
            self.enable_server_pointer = Some(enable_server_pointer);
        }

        #[diplomat::attr(auto, setter = "autologon")]
        pub fn set_autologon(&mut self, autologon: bool) {
            self.autologon = Some(autologon);
        }

        #[diplomat::attr(auto, setter = "audio_playback_mode")]
        pub fn set_audio_playback_mode(&mut self, mode: AudioPlaybackMode) {
            self.audio_playback_mode = Some(mode);
        }

        #[diplomat::attr(auto, setter = "pointer_software_rendering")]
        pub fn set_pointer_software_rendering(&mut self, pointer_software_rendering: bool) {
            self.pointer_software_rendering = Some(pointer_software_rendering);
        }

        #[diplomat::attr(auto, setter = "dvc_pipe_proxy")]
        pub fn set_dvc_pipe_proxy(&mut self, dvc_pipe_proxy: &DvcPipeProxyConfig) {
            self.dvc_pipe_proxy = Some(dvc_pipe_proxy.clone());
        }

        #[diplomat::attr(auto, setter = "direct_dvc")]
        pub fn set_direct_dvc(&mut self, direct_dvc: &DirectDvcConfig) {
            self.direct_dvc = Some(direct_dvc.clone());
        }

        /// Selects a Windows directory to redirect as a remote drive.
        ///
        /// The root must be an absolute, normalized drive-letter path.
        /// Drive IDs are assigned in selection order, starting at one.
        /// Read-only drives reject remote writes.
        /// This setting applies to every connector built from this configuration and requires reconnecting to change.
        /// Drive redirection is unsupported on non-Windows platforms.
        /// RDPDR announces redirected devices with [MS-RDPEFS] 2.2.3.1, "Client Device List Announce".
        ///
        /// [MS-RDPEFS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d8b2bc1c-0207-4c15-abe3-62eaa2afcaf1
        pub fn add_redirected_drive(
            &mut self,
            display_name: &str,
            root_path: &str,
            read_only: bool,
        ) -> Result<(), Box<IronRdpError>> {
            #[cfg(windows)]
            {
                let wire_name = super::drive_wire_name(display_name);
                if self.redirected_drive_wire_names.contains(&wire_name) {
                    return Err("redirected drive name is already in use".into());
                }
                let device_id =
                    u32::try_from(self.redirected_drives.len() + 1).map_err(|_| "too many redirected drives")?;
                ironrdp_rdpdr_native::RedirectedDrive::new(
                    device_id,
                    display_name.to_owned(),
                    std::path::PathBuf::from(root_path),
                    read_only,
                )
                .map_err(|error| {
                    crate::error::GenericError(anyhow::Error::new(error).context("invalid redirected drive"))
                })?;
                self.redirected_drives.push(super::RedirectedDriveConfig {
                    device_id,
                    display_name: display_name.to_owned(),
                    root_path: root_path.to_owned(),
                    read_only,
                });
                self.redirected_drive_wire_names.push(wire_name);
                Ok(())
            }

            #[cfg(not(windows))]
            {
                let _ = (display_name, root_path, read_only);
                Err("drive redirection is supported only on Windows".into())
            }
        }

        #[expect(
            deprecated,
            reason = "a `Config` literal must name every field, including the deprecated one"
        )]
        pub fn build(&self) -> Result<Box<Config>, Box<IronRdpError>> {
            let connector = ironrdp::connector::Config {
                credentials: self.credentials.clone().ok_or("credentials not set")?,
                domain: self.domain.clone(),
                enable_tls: self.enable_tls.unwrap_or(false),
                enable_credssp: self.enable_credssp.unwrap_or(true),
                enable_standard_rdp_security: false,
                keyboard_layout: self.keyboard_layout.unwrap_or(0),
                keyboard_type: self
                    .keyboard_type
                    .unwrap_or(ironrdp::pdu::gcc::KeyboardType::IBM_ENHANCED),
                keyboard_subtype: self.keyboard_subtype.unwrap_or(0),
                keyboard_functional_keys_count: self.keyboard_functional_keys_count.unwrap_or(12),
                connection_type: ironrdp::pdu::gcc::ConnectionType::Lan,
                ime_file_name: self.ime_file_name.clone().unwrap_or_default(),
                dig_product_id: self.dig_product_id.clone().unwrap_or_default(),
                desktop_size: self.desktop_size.ok_or("desktop size not set")?,
                monitor_layout: None,
                bitmap: Some(match &self.bitmap {
                    Some(bitmap) => bitmap.clone(),
                    None => super::default_bitmap_config()?,
                }),
                client_build: self.client_build.unwrap_or(0),
                client_name: self.client_name.clone().ok_or("client name not set")?,
                client_dir: self.client_dir.clone().ok_or("client dir not set")?,

                #[cfg(windows)]
                platform: MajorPlatformType::WINDOWS,
                #[cfg(target_os = "macos")]
                platform: MajorPlatformType::MACINTOSH,
                #[cfg(target_os = "ios")]
                platform: MajorPlatformType::IOS,
                #[cfg(target_os = "linux")]
                platform: MajorPlatformType::UNIX,
                #[cfg(target_os = "android")]
                platform: MajorPlatformType::ANDROID,
                #[cfg(target_os = "freebsd")]
                platform: MajorPlatformType::UNIX,
                #[cfg(target_os = "dragonfly")]
                platform: MajorPlatformType::UNIX,
                #[cfg(target_os = "openbsd")]
                platform: MajorPlatformType::UNIX,
                #[cfg(target_os = "netbsd")]
                platform: MajorPlatformType::UNIX,

                enable_server_pointer: self.enable_server_pointer.unwrap_or(false),
                autologon: self.autologon.unwrap_or(false),
                // The connector decides the playback request when the connection starts.
                enable_audio_playback: false,
                audio_playback_mode: self.audio_playback_mode.map(Into::into),
                enable_audio_capture: false,
                request_data: None,
                compression_type: None,
                pointer_software_rendering: self.pointer_software_rendering.unwrap_or(false),
                multitransport_flags: None,
                support_dyn_vc_gfx_protocol: false,
                rdp_version: ironrdp::pdu::gcc::RdpVersion::V5_PLUS,
                performance_flags: self.performance_flags.ok_or("performance flag is missing")?,
                desktop_scale_factor: 0,
                desktop_physical_size: None,
                desktop_orientation: None,
                device_scale_factor: None,
                monitor_extended_layout: None,
                relative_mouse_input: false,
                hardware_id: None,
                license_cache: None,
                timezone_info: self.timezone_info.clone().unwrap_or_default(),
                alternate_shell: String::new(),
                work_dir: String::new(),
                remote_application_mode: false,
                rail_support_level: ironrdp::pdu::rdp::capability_sets::RailSupportLevel::empty(),
            };
            let dvc_pipe_proxy = self.dvc_pipe_proxy.clone();
            let direct_dvc = self.direct_dvc.clone();

            tracing::debug!(config=?connector, "Built config");

            Ok(Box::new(Config {
                connector,
                dvc_pipe_proxy,
                direct_dvc,
                redirected_drives: self.redirected_drives.clone(),
            }))
        }
    }

    #[diplomat::opaque_mut]
    #[derive(Default)]
    pub struct PerformanceFlags(pub ironrdp::pdu::rdp::client_info::PerformanceFlags);

    pub enum PerformanceFlagsType {
        DisableWallpaper,
        DisableFullWindowDrag,
        DisableMenuAnimations,
        DisableTheming,
        Reserved1,
        DisableCursorShadow,
        DisableCursorSettings,
        EnableFontSmoothing,
        EnableDesktopComposition,
        Reserved2,
    }

    impl PerformanceFlags {
        pub fn new_default() -> Box<Self> {
            Box::<PerformanceFlags>::default()
        }

        pub fn new_empty() -> Box<Self> {
            Box::new(PerformanceFlags(
                ironrdp::pdu::rdp::client_info::PerformanceFlags::empty(),
            ))
        }

        pub fn add_flag(&mut self, flag: PerformanceFlagsType) {
            self.0.insert(flag.into());
        }
    }

    #[diplomat::opaque]
    pub struct BitmapConfig(pub ironrdp::connector::BitmapConfig);
}

impl From<PerformanceFlagsType> for PerformanceFlags {
    fn from(val: PerformanceFlagsType) -> Self {
        match val {
            PerformanceFlagsType::DisableCursorSettings => PerformanceFlags::DISABLE_CURSORSETTINGS,
            PerformanceFlagsType::DisableCursorShadow => PerformanceFlags::DISABLE_CURSOR_SHADOW,
            PerformanceFlagsType::DisableFullWindowDrag => PerformanceFlags::DISABLE_FULLWINDOWDRAG,
            PerformanceFlagsType::DisableMenuAnimations => PerformanceFlags::DISABLE_MENUANIMATIONS,
            PerformanceFlagsType::DisableTheming => PerformanceFlags::DISABLE_THEMING,
            PerformanceFlagsType::DisableWallpaper => PerformanceFlags::DISABLE_WALLPAPER,
            PerformanceFlagsType::EnableDesktopComposition => PerformanceFlags::ENABLE_DESKTOP_COMPOSITION,
            PerformanceFlagsType::EnableFontSmoothing => PerformanceFlags::ENABLE_FONT_SMOOTHING,
            PerformanceFlagsType::Reserved1 => PerformanceFlags::RESERVED1,
            PerformanceFlagsType::Reserved2 => PerformanceFlags::RESERVED2,
        }
    }
}

#[cfg(test)]
mod tests {
    use ironrdp::pdu::rdp::capability_sets::CodecProperty;

    fn buildable_builder() -> Box<super::ffi::ConfigBuilder> {
        let mut builder = super::ffi::ConfigBuilder::new();
        builder.with_username_and_password("user", "password");
        builder.set_desktop_size(768, 1024);
        builder.set_client_name("client");
        builder.set_client_dir("C:\\");
        builder.set_performance_flags(&super::ffi::PerformanceFlags::new_default());
        builder
    }

    #[cfg(windows)]
    // oracle: mutant reuse one read-only setting for every configured drive
    #[test]
    fn redirected_drives_keep_independent_read_only_settings() {
        let root_path = std::env::current_dir().unwrap().to_string_lossy().into_owned();
        let mut builder = buildable_builder();
        assert!(builder.add_redirected_drive("ReadOnly", &root_path, true).is_ok());
        assert!(builder.add_redirected_drive("Writable", &root_path, false).is_ok());

        let Ok(config) = builder.build() else {
            panic!("valid configuration");
        };

        assert_eq!(config.redirected_drives.len(), 2);
        assert!(config.redirected_drives[0].read_only);
        assert!(!config.redirected_drives[1].read_only);
    }

    // oracle: mutant sanitize two distinct names to different wire names
    // The colliding pairs follow the drive DeviceData encoding in ironrdp-rdpdr (`DeviceAnnounceHeader::new_drive`)
    // followed by Unicode uppercase conversion; no specification defines this mapping.
    #[test]
    fn drive_names_collide_after_wire_sanitization_and_case_folding() {
        assert_eq!(
            super::drive_wire_name("Projects/Docs"),
            super::drive_wire_name("projects_docs")
        );
        assert_eq!(super::drive_wire_name("\u{c4}"), super::drive_wire_name("\u{e4}"));
        assert_eq!(super::drive_wire_name("\u{3a3}"), super::drive_wire_name("\u{3c2}"));
        assert_ne!(super::drive_wire_name("Projects"), super::drive_wire_name("Project"));
    }

    // oracle: spec MS-RDPEFS §2.2.1.3 allows a colon only at the end of a device name without saying the server keeps it,
    // so the conservative collision check treats a trailing colon as absent.
    #[test]
    fn drive_names_differing_only_by_trailing_colon_collide() {
        assert_eq!(super::drive_wire_name("C"), super::drive_wire_name("C:"));
        assert_eq!(super::drive_wire_name("data"), super::drive_wire_name("DATA:"));
        assert_ne!(super::drive_wire_name("C"), super::drive_wire_name("C:D"));
    }

    #[cfg(windows)]
    // oracle: mutant accept a second drive with the same sanitized wire name
    #[test]
    fn duplicate_wire_names_are_rejected_without_changing_drives() {
        let root_path = std::env::current_dir().unwrap().to_string_lossy().into_owned();
        let mut builder = buildable_builder();
        assert!(builder.add_redirected_drive("Projects/Docs", &root_path, true).is_ok());
        assert!(
            builder
                .add_redirected_drive("projects_docs", &root_path, false)
                .is_err()
        );
        assert!(builder.add_redirected_drive("Other", &root_path, false).is_ok());
        let Ok(config) = builder.build() else {
            panic!("valid configuration");
        };
        assert_eq!(config.redirected_drives.len(), 2);
        assert_eq!(config.redirected_drives[1].device_id, 2);
    }

    #[cfg(not(windows))]
    // oracle: mutant allow drive redirection on unsupported platforms
    #[test]
    fn redirected_drives_are_rejected_on_unsupported_platforms() {
        let mut builder = super::ffi::ConfigBuilder::new();

        assert!(builder.add_redirected_drive("Shared", "C:\\shared", true).is_err());
    }

    #[test]
    fn build_offers_remotefx_when_no_bitmap_config_is_set() {
        let builder = buildable_builder();

        let Ok(config) = builder.build() else {
            panic!("a builder with every required field should build");
        };
        let Some(bitmap) = config.connector.bitmap else {
            panic!("the config should carry bitmap settings");
        };

        assert_eq!(bitmap.color_depth, 32);
        assert!(bitmap.lossy_compression);
        assert!(
            bitmap
                .codecs
                .0
                .iter()
                .any(|codec| matches!(codec.property, CodecProperty::RemoteFx(_)))
        );
    }
}
