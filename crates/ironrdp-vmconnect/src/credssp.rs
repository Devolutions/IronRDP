use ironrdp_connector::credssp::{CredsspSequenceTrait, CredsspState};
use ironrdp_connector::{
    custom_err, general_err, ConnectorError, ConnectorErrorKind, ConnectorResult, ServerName, Written,
};
use ironrdp_core::WriteBuf;
use sspi::credssp::{self, ClientState, CredSspClient};
use tracing::debug;

use crate::config::Credentials;

// pub type CredsspProcessGenerator<'a> = Generator<'a, NetworkRequest, sspi::Result<Vec<u8>>, sspi::Result<ClientState>>;

#[derive(Debug)]
pub struct VmCredsspSequence {
    client: CredSspClient,
    state: CredsspState,
}

impl CredsspSequenceTrait for VmCredsspSequence {
    fn credssp_state(&self) -> &CredsspState {
        &self.state
    }

    fn set_credssp_state(&mut self, state: CredsspState) {
        self.state = state;
    }

    fn process_ts_request(
        &mut self,
        request: credssp::TsRequest,
    ) -> ironrdp_connector::credssp::CredsspProcessGenerator<'_> {
        self.client.process(request)
    }

    fn handle_process_result(&mut self, result: ClientState, output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (size, next_state) = match self.state {
            CredsspState::Ongoing => {
                let (ts_request_from_client, next_state) = match result {
                    ClientState::ReplyNeeded(ts_request) => (ts_request, CredsspState::Ongoing),
                    ClientState::FinalMessage(ts_request) => (ts_request, CredsspState::Finished),
                };

                debug!(message = ?ts_request_from_client, "Send");

                let written = write_credssp_request(ts_request_from_client, output)?;

                Ok((Written::from_size(written)?, next_state))
            }
            CredsspState::EarlyUserAuthResult => Ok((Written::Nothing, CredsspState::Finished)),
            CredsspState::Finished => Err(general_err!("CredSSP sequence is already done")),
        }?;

        self.state = next_state;

        Ok(size)
    }
}

/// The main difference between this and the `credssp::CredsspSequence` is that this sequence uses NTLM only
/// No Kerberos or Negotiate, as Hyper-V does not support it
impl VmCredsspSequence {
    /// `server_name` must be the actual target server hostname (as opposed to the proxy)
    pub fn init(
        credentials: Credentials,
        domain: Option<&str>,
        server_name: ServerName,
        server_public_key: Vec<u8>,
    ) -> ConnectorResult<(Self, credssp::TsRequest)> {
        let credentials: sspi::Credentials = credentials
            .to_sspi_auth_identity(domain)
            .map_err(|e| custom_err!("Invalid username", e))?
            .into();

        let server_name = server_name.into_inner();

        let service_principal_name = format!("TERMSRV/{}", &server_name);

        let credssp_config = Box::<sspi::ntlm::NtlmConfig>::default();
        debug!(?credssp_config);

        let client = CredSspClient::new(
            server_public_key,
            credentials,
            credssp::CredSspMode::WithCredentials,
            credssp::ClientMode::Ntlm(sspi::ntlm::NtlmConfig {
                client_computer_name: Some(server_name),
            }),
            service_principal_name,
        )
        .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

        let sequence = Self {
            client,
            state: CredsspState::Ongoing,
        };

        let initial_request = credssp::TsRequest::default();

        Ok((sequence, initial_request))
    }
}

fn write_credssp_request(ts_request: credssp::TsRequest, output: &mut WriteBuf) -> ConnectorResult<usize> {
    let length = usize::from(ts_request.buffer_len());

    let unfilled_buffer = output.unfilled_to(length);

    ts_request
        .encode_ts_request(unfilled_buffer)
        .map_err(|e| custom_err!("TsRequest", e))?;

    output.advance(length);

    Ok(length)
}
