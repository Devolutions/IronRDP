pub type CredsspGeneratorState =
    sspi::generator::GeneratorState<sspi::generator::NetworkRequest, sspi::Result<sspi::credssp::ClientState>>;

#[diplomat::bridge]
pub mod ffi {

    use crate::{credssp::ffi::TsRequest, error::ffi::IronRdpError, utils::ffi::VecU8};

    use super::CredsspGeneratorState;

    #[diplomat::opaque]
    pub struct CredsspProcessGenerator<'a>(pub ironrdp::connector::credssp::CredsspProcessGenerator<'a>);

    #[diplomat::opaque]
    pub struct GeneratorState(pub CredsspGeneratorState);

    #[diplomat::opaque]
    pub struct NetworkRequest<'a>(pub &'a sspi::generator::NetworkRequest);

    #[diplomat::opaque]
    pub struct ClientState(pub sspi::credssp::ClientState);

    impl<'a> CredsspProcessGenerator<'a> {
        pub fn start(&mut self) -> Result<Box<GeneratorState>, Box<IronRdpError>> {
            let state = self.0.start();
            Ok(Box::new(GeneratorState(state)))
        }

        pub fn resume(&mut self, response: &VecU8) -> Result<Box<GeneratorState>, Box<IronRdpError>> {
            let state = self.0.resume(Ok(response.0.clone()));
            Ok(Box::new(GeneratorState(state)))
        }
    }

    impl GeneratorState {
        pub fn is_suspended(&self) -> bool {
            matches!(self.0, CredsspGeneratorState::Suspended(_))
        }

        pub fn is_completed(&self) -> bool {
            matches!(self.0, CredsspGeneratorState::Completed(_))
        }

        pub fn get_network_request_if_suspended<'a>(&'a self) -> Option<Box<NetworkRequest<'a>>> {
            match &self.0 {
                CredsspGeneratorState::Suspended(request) => Some(Box::new(NetworkRequest(request))),
                _ => None,
            }
        }

        pub fn get_client_state_if_completed(&self) -> Result<Box<ClientState>, Box<IronRdpError>> {
            match &self.0 {
                CredsspGeneratorState::Completed(Ok(res)) => Ok(res.clone()).map(ClientState).map(Box::new),
                CredsspGeneratorState::Completed(Err(e)) => Err(e.to_owned().into()),
                _ => Err("Generator is not completed".into()),
            }
        }
    }

    impl ClientState {
        pub fn is_reply_needed(&self) -> bool {
            matches!(self.0, sspi::credssp::ClientState::ReplyNeeded(_))
        }

        pub fn is_final_message(&self) -> bool {
            matches!(self.0, sspi::credssp::ClientState::FinalMessage(_))
        }

        pub fn get_ts_request(&self) -> Result<Box<TsRequest>, Box<IronRdpError>> {
            match &self.0 {
                sspi::credssp::ClientState::ReplyNeeded(ts_request) => Ok(Box::new(TsRequest(ts_request.clone()))),
                sspi::credssp::ClientState::FinalMessage(ts_request) => Ok(Box::new(TsRequest(ts_request.clone()))),
            }
        }
    }
}
