#[diplomat::bridge]
pub mod ffi {
    use ironrdp::connector::sspi::network_client;
    use ironrdp_blocking::connect_begin;

    use crate::{
        connector::result::ffi::ConnectionResult,
        error::{
            ffi::{IronRdpError, IronRdpErrorKind},
            NullPointerError,
        },
        tls::TlsStream,
        utils::ffi::{StdTcpStream, VecU8},
    };

    #[diplomat::opaque]
    pub struct BlockingTcpFrame(pub Option<ironrdp_blocking::Framed<std::net::TcpStream>>);

    impl BlockingTcpFrame {
        pub fn from_tcp_stream(stream: &mut StdTcpStream) -> Result<Box<BlockingTcpFrame>, Box<IronRdpError>> {
            let Some(stream) = stream.0.take() else {
                return Err(NullPointerError::for_item("tcp_stream")
                    .reason("tcp stream has been consumed")
                    .into());
            };

            let framed = ironrdp_blocking::Framed::new(stream);

            Ok(Box::new(BlockingTcpFrame(Some(framed))))
        }

        pub fn into_tcp_steam_no_leftover(&mut self) -> Result<Box<StdTcpStream>, Box<IronRdpError>> {
            let Some(stream) = self.0.take() else {
                return Err(NullPointerError::for_item("BlockingTcpFrame")
                    .reason("BlockingTcpFrame has been consumed")
                    .into());
            };

            let stream = stream.into_inner_no_leftover();

            Ok(Box::new(StdTcpStream(Some(stream))))
        }
    }

    #[diplomat::opaque]
    pub struct BlockingUpgradedFrame(pub Option<ironrdp_blocking::Framed<TlsStream>>);

    impl BlockingUpgradedFrame {
        pub fn from_upgraded_stream(
            stream: &mut crate::tls::ffi::UpgradedStream,
        ) -> Result<Box<BlockingUpgradedFrame>, Box<IronRdpError>> {
            let Some(stream) = stream.0.take() else {
                return Err(NullPointerError::for_item("upgraded_stream")
                    .reason("upgraded stream has been consumed")
                    .into());
            };

            let framed = ironrdp_blocking::Framed::new(stream);

            Ok(Box::new(BlockingUpgradedFrame(Some(framed))))
        }
    }

    #[diplomat::opaque] // Diplomat does not support direct function calls, so we need to wrap the function in a struct
    pub struct IronRdpBlocking;

    #[diplomat::opaque]
    pub struct ShouldUpgrade(pub Option<ironrdp_blocking::ShouldUpgrade>);

    #[diplomat::opaque]
    pub struct Upgraded(pub Option<ironrdp_blocking::Upgraded>);

    impl IronRdpBlocking {
        pub fn new() -> Box<IronRdpBlocking> {
            Box::new(IronRdpBlocking)
        }

        pub fn connect_begin(
            framed: &mut BlockingTcpFrame,
            connector: &mut crate::connector::ffi::ClientConnector,
        ) -> Result<Box<ShouldUpgrade>, Box<IronRdpError>> {
            let Some(ref mut connector) = connector.0 else {
                return Err(IronRdpErrorKind::NullPointer.into());
            };

            let Some(framed) = framed.0.as_mut() else {
                return Err(NullPointerError::for_item("framed")
                    .reason("framed has been consumed")
                    .into());
            };

            let result = connect_begin(framed, connector)?;

            Ok(Box::new(ShouldUpgrade(Some(result))))
        }

        pub fn mark_as_upgraded(
            should_upgrade: &mut ShouldUpgrade,
            connector: &mut crate::connector::ffi::ClientConnector,
        ) -> Result<Box<Upgraded>, Box<IronRdpError>> {
            let Some(ref mut connector) = connector.0 else {
                return Err(NullPointerError::for_item("connector")
                    .reason("inner connector is missing")
                    .into());
            };

            let Some(should_upgrade) = should_upgrade.0.take() else {
                return Err(NullPointerError::for_item("should_upgrade")
                    .reason("ShouldUpgrade is missing, Note: ShouldUpgrade should be used only once")
                    .into());
            };

            let result = ironrdp_blocking::mark_as_upgraded(should_upgrade, connector);

            Ok(Box::new(Upgraded(Some(result))))
        }

        pub fn skip_connect_begin(
            connector: &mut crate::connector::ffi::ClientConnector,
        ) -> Result<Box<ShouldUpgrade>, Box<IronRdpError>> {
            let Some(ref mut connector) = connector.0 else {
                return Err(NullPointerError::for_item("connector")
                    .reason("inner connector is missing")
                    .into());
            };

            let result = ironrdp_blocking::skip_connect_begin(connector);

            Ok(Box::new(ShouldUpgrade(Some(result))))
        }

        pub fn connect_finalize(
            upgraded: &mut Upgraded,
            upgraded_framed: &mut BlockingUpgradedFrame,
            connector: &mut crate::connector::ffi::ClientConnector,
            server_name: &crate::connector::ffi::ServerName,
            server_public_key: &VecU8,
            kerberos_config: Option<&crate::credssp::ffi::KerberosConfig>,
        ) -> Result<Box<ConnectionResult>, Box<IronRdpError>> {
            let Some(connector) = connector.0.take() else {
                return Err(NullPointerError::for_item("connector")
                    .reason("inner connector is missing")
                    .into());
            };

            let Some(upgraded) = upgraded.0.take() else {
                return Err(NullPointerError::for_item("upgraded")
                    .reason("Upgraded inner is missing, Note: Upgraded should be used only once")
                    .into());
            };

            let Some(framed) = upgraded_framed.0.as_mut() else {
                return Err(NullPointerError::for_item("framed")
                    .reason("framed has been consumed")
                    .into());
            };

            let server_name = server_name.0.clone();
            let mut network_client = network_client::reqwest_network_client::ReqwestNetworkClient::default();

            let kerberos_config = kerberos_config.as_ref().map(|config| config.0.clone());

            let result = ironrdp_blocking::connect_finalize(
                upgraded,
                framed,
                connector,
                server_name,
                server_public_key.0.clone(),
                &mut network_client,
                kerberos_config,
            )?;

            Ok(Box::new(ConnectionResult(result)))
        }
    }
}
