#[diplomat::bridge]
pub mod ffi {
    use crate::{connector::config::ffi::DesktopSize, utils::ffi::OptionalUsize};

    #[diplomat::opaque]
    pub struct Written(pub ironrdp::connector::Written);

    pub enum WrittenType {
        Size,
        Nothing,
    }

    impl Written {
        pub fn get_written_type(&self) -> WrittenType {
            match &self.0 {
                ironrdp::connector::Written::Size(_) => WrittenType::Size,
                ironrdp::connector::Written::Nothing => WrittenType::Nothing,
            }
        }

        pub fn get_size(&self) -> Box<OptionalUsize> {
            match &self.0 {
                ironrdp::connector::Written::Size(size) => Box::new(OptionalUsize(Some(size.get()))),
                ironrdp::connector::Written::Nothing => Box::new(OptionalUsize(None)),
            }
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionResult(pub ironrdp::connector::ConnectionResult);

    impl ConnectionResult {
        pub fn get_io_channel_id(&self) -> u16 {
            self.0.io_channel_id
        }

        pub fn get_user_channel_id(&self) -> u16 {
            self.0.user_channel_id
        }

        pub fn get_static_channels(&self) -> Box<crate::svc::ffi::StaticChannelSet<'_>> {
            Box::new(crate::svc::ffi::StaticChannelSet(&self.0.static_channels))
        }

        pub fn get_desktop_size(&self) -> Box<DesktopSize> {
            Box::new(DesktopSize(self.0.desktop_size))
        }

        pub fn get_no_server_pointer(&self) -> bool {
            self.0.no_server_pointer
        }

        pub fn get_pointer_software_rendering(&self) -> bool {
            self.0.pointer_software_rendering
        }

        pub fn get_graphics_config(&self) -> Option<Box<GraphicsConfig>> {
            self.0.graphics_config.clone().map(GraphicsConfig).map(Box::new)
        }
    }

    #[diplomat::opaque]
    pub struct GraphicsConfig(pub ironrdp::connector::GraphicsConfig);

    impl GraphicsConfig {
        pub fn get_avc444(&self) -> bool {
            self.0.avc444
        }

        pub fn get_h264(&self) -> bool {
            self.0.h264
        }

        pub fn get_thin_client(&self) -> bool {
            self.0.thin_client
        }

        pub fn get_small_cache(&self) -> bool {
            self.0.small_cache
        }

        pub fn get_capabilities(&self) -> u32 {
            self.0.capabilities
        }
    }
}
