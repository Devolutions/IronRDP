//! Minimal MS-RDPEWA server DVC processor skeleton.

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{E_NOTIMPL, RdpewaRequest, RdpewaResponse, RpcCommand, S_OK};

/// Server-side skeleton that accepts the channel and answers simple RPCs.
///
/// This is not a full host authenticator service. WEB_AUTHN returns `E_NOTIMPL`.
#[derive(Debug)]
pub struct RdpewaServer {
    api_version: u32,
}

impl Default for RdpewaServer {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpewaServer {
    pub fn new() -> Self {
        Self { api_version: 1 }
    }

    #[must_use]
    pub fn with_api_version(mut self, version: u32) -> Self {
        self.api_version = version;
        self
    }
}

impl_as_any!(RdpewaServer);

impl DvcProcessor for RdpewaServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let request = RdpewaRequest::decode(payload).map_err(|e| decode_err!(e))?;
        debug!(?request.command, "Server received RDPEWA request");

        let response = match request.command {
            RpcCommand::ApiVersion => RdpewaResponse::with_u32(S_OK, self.api_version),
            RpcCommand::Iuvpaa => RdpewaResponse::with_u32(S_OK, 0),
            RpcCommand::CancelCurOp => RdpewaResponse::ok_empty(),
            RpcCommand::WebAuthn | RpcCommand::GetCredentials | RpcCommand::GetAuthenticatorList => {
                RdpewaResponse::from_hresult(E_NOTIMPL)
            }
        };

        Ok(vec![Box::new(response)])
    }
}

impl DvcServerProcessor for RdpewaServer {}

#[cfg(test)]
mod tests {
    use super::RdpewaServer;

    #[test]
    fn default_uses_the_current_api_version() {
        assert_eq!(RdpewaServer::default().api_version, 1);
    }
}
