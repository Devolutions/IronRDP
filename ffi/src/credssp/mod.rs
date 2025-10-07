#![allow(clippy::needless_lifetimes)] // Diplomat requires lifetimes
pub mod network;

#[diplomat::bridge]
pub mod ffi {

    use ironrdp::connector::ClientConnectorState;

    use super::network::ffi::{ClientState, CredsspProcessGenerator};
    use crate::connector::ffi::{ClientConnector, PduHint};
    use crate::connector::result::ffi::Written;
    use crate::error::ffi::IronRdpError;
    use crate::error::ValueConsumedError;
    use crate::pdu::ffi::WriteBuf;

    #[diplomat::opaque]
    pub struct KerberosConfig(pub ironrdp::connector::credssp::KerberosConfig);

    impl KerberosConfig {
        /// Creates a new KerberosConfig for KDC proxy support.
        ///
        /// # Arguments
        /// * `kdc_proxy_url` - KDC proxy URL (e.g., "https://gateway.example.com/KdcProxy/{token}"), empty string if not used
        /// * `hostname` - Client hostname for Kerberos, empty string if not used
        pub fn new(
            kdc_proxy_url: &str,
            hostname: &str,
        ) -> Result<Box<KerberosConfig>, Box<IronRdpError>> {
            let kdc_proxy_url_opt = if kdc_proxy_url.is_empty() {
                None
            } else {
                Some(kdc_proxy_url.to_owned())
            };

            let hostname_opt = if hostname.is_empty() {
                None
            } else {
                Some(hostname.to_owned())
            };

            let config = ironrdp::connector::credssp::KerberosConfig::new(
                kdc_proxy_url_opt,
                hostname_opt,
            )?;
            Ok(Box::new(KerberosConfig(config)))
        }
    }

    #[diplomat::opaque]
    pub struct CredsspSequence(pub ironrdp::connector::credssp::CredsspSequence);

    #[diplomat::opaque]
    pub struct TsRequest(pub sspi::credssp::TsRequest);

    #[diplomat::opaque]
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
            kerbero_configs: Option<&KerberosConfig>,
        ) -> Result<Box<CredsspSequenceInitResult>, Box<IronRdpError>> {
            let Some(connector) = connector.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            match connector.state {
                ClientConnectorState::Credssp { selected_protocol } => {
                    let (credssp_sequence, ts_request) = ironrdp::connector::credssp::CredsspSequence::init(
                        connector.config.credentials.clone(),
                        connector.config.domain.as_deref(),
                        selected_protocol,
                        server_name.into(),
                        server_public_key.to_owned(),
                        kerbero_configs.map(|config| config.0.clone()),
                    )?;

                    Ok(Box::new(CredsspSequenceInitResult {
                        credssp_sequence: Some(Box::new(CredsspSequence(credssp_sequence))),
                        ts_request: Some(Box::new(TsRequest(ts_request))),
                    }))
                }
                _ => Err(ironrdp::connector::general_err!("invalid connector state for CredSSP sequence").into()),
            }
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
