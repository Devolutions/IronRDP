#[diplomat::bridge]
pub mod ffi {
    use ironrdp::{
        connector::{BitmapConfig, Credentials},
        pdu::rdp::capability_sets::MajorPlatformType,
    };

    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct Config(pub ironrdp::connector::Config);

    impl Config {
        pub fn get_builder() -> Box<crate::connector::config::ffi::ConfigBuilder> {
            Box::<ConfigBuilder>::default()
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
        pub keyboard_functional_keys_count: Option<u32>,
        pub ime_file_name: Option<String>,
        pub dig_product_id: Option<String>,
        pub desktop_size: Option<ironrdp::connector::DesktopSize>,
        pub graphics: Option<ironrdp::connector::GraphicsConfig>,
        pub bitmap: Option<BitmapConfig>,
        pub client_build: Option<u32>,
        pub client_name: Option<String>,
        pub client_dir: Option<String>,
        pub platform: Option<MajorPlatformType>,
        pub no_server_pointer: Option<bool>,
        pub autologon: Option<bool>,
        pub pointer_software_rendering: Option<bool>,
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

        pub fn with_username_and_passwrord(&mut self, username: &str, password: &str) {
            self.credentials = Some(Credentials::UsernamePassword {
                username: username.to_string(),
                password: password.to_string(),
            });
        }

        pub fn set_domain(&mut self, domain: &str) {
            self.domain = Some(domain.to_string());
        }

        pub fn set_enable_tls(&mut self, enable_tls: bool) {
            self.enable_tls = Some(enable_tls);
        }

        pub fn set_enable_credssp(&mut self, enable_credssp: bool) {
            self.enable_credssp = Some(enable_credssp);
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
            self.ime_file_name = Some(ime_file_name.to_string());
        }

        pub fn set_dig_product_id(&mut self, dig_product_id: &str) {
            self.dig_product_id = Some(dig_product_id.to_string());
        }

        pub fn set_desktop_size(&mut self, height: u16, width: u16) {
            self.desktop_size = Some(ironrdp::connector::DesktopSize { width, height });
        }

        pub fn set_graphics(&mut self, graphics: &crate::connector::result::ffi::GraphicsConfig) {
            self.graphics = Some(graphics.0.clone());
        }

        // TODO: set bitmap

        pub fn set_client_build(&mut self, client_build: u32) {
            self.client_build = Some(client_build);
        }

        pub fn set_client_name(&mut self, client_name: &str) {
            self.client_name = Some(client_name.to_string());
        }

        pub fn set_client_dir(&mut self, client_dir: &str) {
            self.client_dir = Some(client_dir.to_string());
        }

        pub fn set_no_server_pointer(&mut self, no_server_pointer: bool) {
            self.no_server_pointer = Some(no_server_pointer);
        }

        pub fn set_autologon(&mut self, autologon: bool) {
            self.autologon = Some(autologon);
        }

        pub fn set_pointer_software_rendering(&mut self, pointer_software_rendering: bool) {
            self.pointer_software_rendering = Some(pointer_software_rendering);
        }

        pub fn build(&self) -> Result<Box<Config>, Box<IronRdpError>> {
            let inner_config = ironrdp::connector::Config {
                credentials: self.credentials.clone().ok_or("Credentials not set")?,
                domain: self.domain.clone(),
                enable_tls: self.enable_tls.unwrap_or(false),
                enable_credssp: self.enable_credssp.unwrap_or(true),

                keyboard_type: self
                    .keyboard_type
                    .unwrap_or(ironrdp::pdu::gcc::KeyboardType::IbmEnhanced),
                keyboard_subtype: self.keyboard_subtype.unwrap_or(0),
                keyboard_functional_keys_count: self.keyboard_functional_keys_count.unwrap_or(12),
                ime_file_name: self.ime_file_name.clone().unwrap_or_default(),
                dig_product_id: self.dig_product_id.clone().unwrap_or_default(),
                desktop_size: self.desktop_size.ok_or("Desktop size not set")?,
                graphics: self.graphics.clone(),
                bitmap: None,
                client_build: self.client_build.unwrap_or(0),
                client_name: self.client_name.clone().ok_or("Client name not set")?,
                client_dir: self.client_dir.clone().ok_or("Client dir not set")?,

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

                no_server_pointer: self.no_server_pointer.unwrap_or(false),
                autologon: self.autologon.unwrap_or(false),
                pointer_software_rendering: self.pointer_software_rendering.unwrap_or(false),
            };

            Ok(Box::new(Config(inner_config)))
        }
    }
}
