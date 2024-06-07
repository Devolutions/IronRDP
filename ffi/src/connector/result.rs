#[diplomat::bridge]
pub mod ffi {
    use crate::{
        connector::config::ffi::DesktopSize,
        error::{ffi::IronRdpError, ValueConsumedError},
        utils::ffi::{OptionalUsize, VecU8},
    };

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
    pub struct ConnectionResult(pub Option<ironrdp::connector::ConnectionResult>);

    impl ConnectionResult {
        pub fn get_io_channel_id(&self) -> Result<u16, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .io_channel_id)
        }

        pub fn get_user_channel_id(&self) -> Result<u16, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .user_channel_id)
        }

        pub fn get_desktop_size(&self) -> Result<Box<DesktopSize>, Box<IronRdpError>> {
            Ok(Box::new(DesktopSize(
                self.0
                    .as_ref()
                    .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                    .desktop_size,
            )))
        }

        pub fn get_no_server_pointer(&self) -> Result<bool, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .no_server_pointer)
        }

        pub fn get_pointer_software_rendering(&self) -> Result<bool, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .pointer_software_rendering)
        }
    }
}
