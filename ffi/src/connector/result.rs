#[diplomat::bridge]
pub mod ffi {
    use crate::connector::config::ffi::DesktopSize;
    use crate::error::ValueConsumedError;
    use crate::error::ffi::IronRdpError;
    use crate::utils::ffi::{OptionalUsize, VecU8};
    use diplomat_runtime::DiplomatWriteable;

    #[diplomat::opaque]
    pub struct Written(pub ironrdp::connector::Written);

    pub enum WrittenType {
        Size,
        Nothing,
    }

    impl Written {
        #[diplomat::attr(auto, getter = "written_type")]
        pub fn get_written_type(&self) -> WrittenType {
            match &self.0 {
                ironrdp::connector::Written::Size(_) => WrittenType::Size,
                ironrdp::connector::Written::Nothing => WrittenType::Nothing,
            }
        }

        #[diplomat::attr(auto, getter = "size")]
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
        #[diplomat::attr(auto, getter = "io_channel_id")]
        pub fn get_io_channel_id(&self) -> Result<u16, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .io_channel_id)
        }

        #[diplomat::attr(auto, getter = "user_channel_id")]
        pub fn get_user_channel_id(&self) -> Result<u16, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .user_channel_id)
        }

        #[diplomat::attr(auto, getter = "share_id")]
        pub fn get_share_id(&self) -> Result<u32, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .share_id)
        }

        #[diplomat::attr(auto, getter = "desktop_size")]
        pub fn get_desktop_size(&self) -> Result<Box<DesktopSize>, Box<IronRdpError>> {
            Ok(Box::new(DesktopSize(
                self.0
                    .as_ref()
                    .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                    .desktop_size,
            )))
        }

        #[diplomat::attr(auto, getter = "enable_server_pointer")]
        pub fn get_enable_server_pointer(&self) -> Result<bool, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .enable_server_pointer)
        }

        #[diplomat::attr(auto, getter = "pointer_software_rendering")]
        pub fn get_pointer_software_rendering(&self) -> Result<bool, Box<IronRdpError>> {
            Ok(self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ConnectionResult"))?
                .pointer_software_rendering)
        }
    }

    /// A Server Redirection PDU (MS-RDPBCGR 2.2.13.1): the server asked the client to reconnect elsewhere.
    ///
    /// Reconnect to the target, sending `load_balance_info` as the X.224 routing token, `session_id` as the
    /// redirected session ID of the GCC Client Cluster Data, and `password` as the Client Info password.
    #[diplomat::opaque]
    pub struct ServerRedirection(pub ironrdp::pdu::rdp::server_redirection::ServerRedirectionPdu);

    impl ServerRedirection {
        #[diplomat::attr(auto, getter = "session_id")]
        pub fn get_session_id(&self) -> u32 {
            self.0.session_id
        }

        /// The `RedirFlags` field.
        #[diplomat::attr(auto, getter = "flags")]
        pub fn get_flags(&self) -> u32 {
            self.0.wire_flags().bits()
        }

        /// Writes the target FQDN, or nothing when the PDU has none.
        pub fn get_target_fqdn(&self, writeable: &mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            write_optional(writeable, self.0.target_fqdn.as_deref())
        }

        /// Writes the target NetBIOS name, or nothing when the PDU has none.
        pub fn get_target_netbios_name(&self, writeable: &mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            write_optional(writeable, self.0.target_netbios_name.as_deref())
        }

        /// Writes the target IP address, or nothing when the PDU has none.
        pub fn get_target_net_address(&self, writeable: &mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            write_optional(writeable, self.0.target_net_address.as_deref())
        }

        #[diplomat::attr(auto, getter = "target_net_addresses_count")]
        pub fn get_target_net_addresses_count(&self) -> usize {
            self.0.target_net_addresses.as_ref().map_or(0, Vec::len)
        }

        /// Writes the IP address at `index` of the `TargetNetAddresses` field.
        pub fn get_target_net_addresses_at(
            &self,
            index: usize,
            writeable: &mut DiplomatWriteable,
        ) -> Result<(), Box<IronRdpError>> {
            let address = self
                .0
                .target_net_addresses
                .as_ref()
                .and_then(|addresses| addresses.get(index))
                .ok_or("target net address index out of range")?;
            write_optional(writeable, Some(address))
        }

        /// Writes the user name, or nothing when the PDU has none.
        pub fn get_username(&self, writeable: &mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            write_optional(writeable, self.0.username.as_deref())
        }

        /// Writes the domain, or nothing when the PDU has none.
        pub fn get_domain(&self, writeable: &mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            write_optional(writeable, self.0.domain.as_deref())
        }

        /// A copy of the load-balancing information, empty when the PDU has none.
        #[diplomat::attr(auto, getter = "load_balance_info")]
        pub fn get_load_balance_info(&self) -> Box<VecU8> {
            VecU8::from_bytes(self.0.load_balance_info.as_deref().unwrap_or_default())
        }

        /// A copy of the password or password cookie, empty when the PDU has none.
        #[diplomat::attr(auto, getter = "password")]
        pub fn get_password(&self) -> Box<VecU8> {
            VecU8::from_bytes(self.0.password.as_deref().unwrap_or_default())
        }

        /// A copy of the DER-encoded certificate of the target, empty when the PDU has none.
        #[diplomat::attr(auto, getter = "target_certificate")]
        pub fn get_target_certificate(&self) -> Box<VecU8> {
            VecU8::from_bytes(
                self.0
                    .target_certificate
                    .as_ref()
                    .and_then(|container| container.certificate_der())
                    .unwrap_or_default(),
            )
        }
    }

    fn write_optional(writeable: &mut DiplomatWriteable, value: Option<&str>) -> Result<(), Box<IronRdpError>> {
        use core::fmt::Write as _;

        if let Some(value) = value {
            writeable.write_str(value).map_err(|_| "failed to write string")?;
        }
        writeable.flush();
        Ok(())
    }
}
