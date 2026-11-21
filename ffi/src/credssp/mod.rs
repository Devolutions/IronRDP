#![allow(clippy::needless_lifetimes)] // Diplomat requires lifetimes
pub mod network;

#[diplomat::bridge]
pub mod ffi {

    use ironrdp::connector::ClientConnectorState;

    use super::network::ffi::{ClientState, CredsspProcessGenerator};
    use crate::connector::ffi::{ClientConnector, PduHint};
    use crate::connector::result::ffi::Written;
    use crate::error::ValueConsumedError;
    use crate::error::ffi::IronRdpError;
    use crate::pdu::ffi::WriteBuf;

    #[diplomat::opaque]
    pub struct KerberosConfig(pub ironrdp::connector::credssp::KerberosConfig);

    impl KerberosConfig {
        /// Kerberos with the KDC found from the user's realm, which comes from the domain or the user principal
        /// name in the connector's credentials.
        ///
        /// The client computer name sent to the KDC is the connector's client name.
        pub fn new() -> Box<KerberosConfig> {
            Box::new(KerberosConfig(ironrdp::connector::credssp::KerberosConfig {
                kdc_resolution: ironrdp::connector::credssp::KdcResolution::KdcUrl(None),
                hostname: String::new(),
            }))
        }

        /// Kerberos with an explicit KDC: `tcp://dc.example.com:88`, or a KDC proxy (MS-KKDCP) such as
        /// `https://gateway.example.com/KdcProxy`. The .NET transport carries `tcp`, `http` and `https`.
        ///
        /// The client computer name sent to the KDC is the connector's client name.
        pub fn new_with_kdc_url(kdc_url: &str) -> Result<Box<KerberosConfig>, Box<IronRdpError>> {
            let config =
                ironrdp::connector::credssp::KerberosConfig::new_with_kdc_url(Some(kdc_url.to_owned()), String::new())?;
            if let ironrdp::connector::credssp::KdcResolution::KdcUrl(Some(url)) = &config.kdc_resolution
                && !matches!(url.scheme(), "tcp" | "http" | "https")
            {
                return Err(ironrdp::connector::general_err!("unsupported KDC URL scheme").into());
            }
            Ok(Box::new(KerberosConfig(config)))
        }
    }

    /// The security package CredSSP authenticates with.
    #[diplomat::enum_convert(ironrdp::connector::credssp::CredsspPackage)]
    pub enum CredsspPackage {
        /// NTLM only. Takes no Kerberos configuration.
        Ntlm,
        /// SPNEGO, preferring Kerberos and allowing NTLM. Without a Kerberos configuration, Kerberos is used when a
        /// KDC is found for the user's realm, and NTLM otherwise. With one, Kerberos starts right away, and a KDC that
        /// cannot be found during the CredSSP exchange fails it instead of falling back to NTLM.
        Negotiate,
        /// SPNEGO restricted to Kerberos: NTLM is never negotiated or used as a fallback.
        Kerberos,
    }

    #[diplomat::opaque_mut]
    pub struct CredsspSequence(pub ironrdp::connector::credssp::CredsspSequence);

    #[diplomat::opaque]
    pub struct TsRequest(pub sspi::credssp::TsRequest);

    #[diplomat::opaque_mut]
    pub struct CredsspSequenceInitResult {
        pub credssp_sequence: Option<Box<CredsspSequence>>,
        pub ts_request: Option<Box<TsRequest>>,
    }

    impl CredsspSequenceInitResult {
        pub fn get_credssp_sequence(&mut self) -> Result<Box<CredsspSequence>, Box<IronRdpError>> {
            let Some(credssp_sequence) = self.credssp_sequence.take() else {
                return Err(ValueConsumedError::for_item("credssp_sequence").into());
            };
            Ok(credssp_sequence)
        }

        pub fn get_ts_request(&mut self) -> Result<Box<TsRequest>, Box<IronRdpError>> {
            let Some(ts_request) = self.ts_request.take() else {
                return Err(ValueConsumedError::for_item("ts_request").into());
            };
            Ok(ts_request)
        }
    }

    impl CredsspSequence {
        pub fn next_pdu_hint<'a>(&'a self) -> Option<Box<PduHint<'a>>> {
            self.0.next_pdu_hint().map(|hint| Box::new(PduHint(hint)))
        }

        pub fn init(
            connector: &ClientConnector,
            server_name: &str,
            server_public_key: &[u8],
            package: CredsspPackage,
            kerberos_config: Option<&KerberosConfig>,
        ) -> Result<Box<CredsspSequenceInitResult>, Box<IronRdpError>> {
            let Some(connector) = connector.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            match connector.state {
                ClientConnectorState::Credssp { selected_protocol } => Self::init_with_selected_protocol(
                    connector,
                    server_name,
                    server_public_key,
                    selected_protocol,
                    package,
                    kerberos_config,
                ),
                _ => Err(ironrdp::connector::general_err!("invalid connector state for CredSSP sequence").into()),
            }
        }

        /// Init CredSSP with an explicit protocol (pre-X.224 VMConnect front).
        ///
        /// Enforces the same TLS + CredSSP prerequisites as
        /// [`ironrdp_vmconnect::prepare_connector`] / [`ironrdp_vmconnect::connect_front`].
        pub fn init_with_protocol(
            connector: &ClientConnector,
            server_name: &str,
            server_public_key: &[u8],
            selected_protocol: u32,
            package: CredsspPackage,
            kerberos_config: Option<&KerberosConfig>,
        ) -> Result<Box<CredsspSequenceInitResult>, Box<IronRdpError>> {
            let Some(connector) = connector.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            ironrdp_vmconnect::prepare_connector(connector)?;
            let selected_protocol = ironrdp::pdu::nego::SecurityProtocol::from_bits(selected_protocol)
                .ok_or_else(|| ironrdp::connector::general_err!("invalid security protocol"))?;
            Self::init_with_selected_protocol(
                connector,
                server_name,
                server_public_key,
                selected_protocol,
                package,
                kerberos_config,
            )
        }

        fn init_with_selected_protocol(
            connector: &ironrdp::connector::ClientConnector,
            server_name: &str,
            server_public_key: &[u8],
            selected_protocol: ironrdp::pdu::nego::SecurityProtocol,
            package: CredsspPackage,
            kerberos_config: Option<&KerberosConfig>,
        ) -> Result<Box<CredsspSequenceInitResult>, Box<IronRdpError>> {
            let kerberos_config = kerberos_config.map(|config| ironrdp::connector::credssp::KerberosConfig {
                hostname: connector.config.client_name.clone(),
                ..config.0.clone()
            });
            let (credssp_sequence, ts_request) = ironrdp::connector::credssp::CredsspSequence::init_with_package(
                connector.config.credentials.clone(),
                connector.config.domain.as_deref(),
                selected_protocol,
                server_name.into(),
                server_public_key.to_owned(),
                package.into(),
                kerberos_config,
            )?;

            Ok(Box::new(CredsspSequenceInitResult {
                credssp_sequence: Some(Box::new(CredsspSequence(credssp_sequence))),
                ts_request: Some(Box::new(TsRequest(ts_request))),
            }))
        }

        pub fn decode_server_message(&mut self, pdu: &[u8]) -> Result<Option<Box<TsRequest>>, Box<IronRdpError>> {
            let ts_request = self.0.decode_server_message(pdu)?;
            Ok(ts_request.map(|ts_request| Box::new(TsRequest(ts_request))))
        }

        pub fn process_ts_request<'a>(
            &'a mut self,
            ts_request: &TsRequest,
        ) -> Result<Box<CredsspProcessGenerator<'a>>, Box<IronRdpError>> {
            let ts_request = ts_request.0.clone();
            let generator = self.0.process_ts_request(ts_request);
            Ok(Box::new(CredsspProcessGenerator(generator)))
        }

        pub fn handle_process_result(
            &mut self,
            client_state: &ClientState,
            buf: &mut WriteBuf,
        ) -> Result<Box<Written>, Box<IronRdpError>> {
            let client_state = client_state.0.clone();
            let written = self.0.handle_process_result(client_state, &mut buf.0)?;
            Ok(Box::new(Written(written)))
        }
    }
}
