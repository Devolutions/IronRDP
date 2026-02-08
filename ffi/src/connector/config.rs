use ironrdp::pdu::rdp::client_info::PerformanceFlags;

use self::ffi::PerformanceFlagsType;

#[diplomat::bridge]
pub mod ffi {
    use ironrdp::connector::Credentials;
    use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;

    use crate::dvc::ffi::DvcPipeProxyConfig;
    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct Config {
        pub connector: ironrdp::connector::Config,
        pub dvc_pipe_proxy: Option<DvcPipeProxyConfig>,
    }

    impl Config {
        pub fn get_builder() -> Box<ConfigBuilder> {
            Box::<ConfigBuilder>::default()
        }

        pub fn get_dvc_pipe_proxy(&self) -> Option<Box<DvcPipeProxyConfig>> {
            self.dvc_pipe_proxy.as_ref().map(|dvc| Box::new(dvc.clone()))
        }
    }

    #[derive(Default)]
    #[diplomat::opaque]
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
        pub no_audio_playback: Option<bool>,
        pub pointer_software_rendering: Option<bool>,
        pub performance_flags: Option<ironrdp::pdu::rdp::client_info::PerformanceFlags>,
        pub timezone_info: Option<ironrdp::pdu::rdp::client_info::TimezoneInfo>,
        pub dvc_pipe_proxy: Option<DvcPipeProxyConfig>,
    }

    #[diplomat::enum_convert(ironrdp::pdu::gcc::KeyboardType)]
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
        pub fn get_width(&self) -> u16 {
            self.0.width
        }

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

        pub fn set_domain(&mut self, domain: &str) {
            self.domain = Some(domain.to_owned());
        }

        pub fn set_enable_tls(&mut self, enable_tls: bool) {
            self.enable_tls = Some(enable_tls);
        }

        pub fn set_enable_credssp(&mut self, enable_credssp: bool) {
            self.enable_credssp = Some(enable_credssp);
        }

        pub fn set_keyboard_layout(&mut self, keyboard_layout: u32) {
            self.keyboard_layout = Some(keyboard_layout);
        }

        pub fn set_keyboard_type(&mut self, keyboard_type: KeyboardType) {
            self.keyboard_type = Some(keyboard_type.into());
        }

        pub fn set_keyboard_subtype(&mut self, keyboard_subtype: u32) {
            self.keyboard_subtype = Some(keyboard_subtype);
        }

        pub fn set_keyboard_functional_keys_count(&mut self, keyboard_functional_keys_count: u32) {
            self.keyboard_functional_keys_count = Some(keyboard_functional_keys_count);
        }

        pub fn set_ime_file_name(&mut self, ime_file_name: &str) {
            self.ime_file_name = Some(ime_file_name.to_owned());
        }

        pub fn set_dig_product_id(&mut self, dig_product_id: &str) {
            self.dig_product_id = Some(dig_product_id.to_owned());
        }

        pub fn set_desktop_size(&mut self, height: u16, width: u16) {
            self.desktop_size = Some(ironrdp::connector::DesktopSize { width, height });
        }

        pub fn set_performance_flags(&mut self, performance_flags: &PerformanceFlags) {
            self.performance_flags = Some(performance_flags.0);
        }

        // pub fn set_timezone_info(&mut self, timezone_info: Option<ironrdp::pdu::rdp::client_info::TimezoneInfo>) {
        //     self.timezone_info = timezone_info;
        // }

        pub fn set_bitmap_config(&mut self, bitmap: &BitmapConfig) {
            self.bitmap = Some(bitmap.0.clone());
        }

        pub fn set_client_build(&mut self, client_build: u32) {
            self.client_build = Some(client_build);
        }

        pub fn set_client_name(&mut self, client_name: &str) {
            self.client_name = Some(client_name.to_owned());
        }

        pub fn set_client_dir(&mut self, client_dir: &str) {
            self.client_dir = Some(client_dir.to_owned());
        }

        pub fn set_enable_server_pointer(&mut self, enable_server_pointer: bool) {
            self.enable_server_pointer = Some(enable_server_pointer);
        }

        pub fn set_autologon(&mut self, autologon: bool) {
            self.autologon = Some(autologon);
        }

        pub fn set_pointer_software_rendering(&mut self, pointer_software_rendering: bool) {
            self.pointer_software_rendering = Some(pointer_software_rendering);
        }

        pub fn set_dvc_pipe_proxy(&mut self, dvc_pipe_proxy: &DvcPipeProxyConfig) {
            self.dvc_pipe_proxy = Some(dvc_pipe_proxy.clone());
        }

        pub fn build(&self) -> Result<Box<Config>, Box<IronRdpError>> {
            let connector = ironrdp::connector::Config {
                credentials: self.credentials.clone().ok_or("credentials not set")?,
                domain: self.domain.clone(),
                enable_tls: self.enable_tls.unwrap_or(false),
                enable_credssp: self.enable_credssp.unwrap_or(true),
                keyboard_layout: self.keyboard_layout.unwrap_or(0),
                keyboard_type: self
                    .keyboard_type
                    .unwrap_or(ironrdp::pdu::gcc::KeyboardType::IbmEnhanced),
                keyboard_subtype: self.keyboard_subtype.unwrap_or(0),
                keyboard_functional_keys_count: self.keyboard_functional_keys_count.unwrap_or(12),
                ime_file_name: self.ime_file_name.clone().unwrap_or_default(),
                dig_product_id: self.dig_product_id.clone().unwrap_or_default(),
                desktop_size: self.desktop_size.ok_or("desktop size not set")?,
                bitmap: None,
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
                enable_audio_playback: self.no_audio_playback.unwrap_or(true),
                request_data: None,
                compression_type: None,
                pointer_software_rendering: self.pointer_software_rendering.unwrap_or(false),
                performance_flags: self.performance_flags.ok_or("performance flag is missing")?,
                desktop_scale_factor: 0,
                hardware_id: None,
                license_cache: None,
                timezone_info: self.timezone_info.clone().unwrap_or_default(),
            };
            let dvc_pipe_proxy = self.dvc_pipe_proxy.clone();

            tracing::debug!(config=?connector, "Built config");

            Ok(Box::new(Config {
                connector,
                dvc_pipe_proxy,
            }))
        }
    }

    #[diplomat::opaque]
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
            PerformanceFlagsType::EnableFontSmoothing => PerformanceFlags::ENABLE_DESKTOP_COMPOSITION,
            PerformanceFlagsType::Reserved1 => PerformanceFlags::RESERVED1,
            PerformanceFlagsType::Reserved2 => PerformanceFlags::RESERVED2,
        }
    }
}
