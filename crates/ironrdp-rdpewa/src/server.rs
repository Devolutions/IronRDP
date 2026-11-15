//! MS-RDPEWA server DVC processors.

use std::collections::BTreeMap;

use ironrdp_core::{Encode, EncodeResult, WriteCursor, ensure_size, impl_as_any};
use ironrdp_dvc::{DvcEncode, DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err, encode_err};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{
    CTAPCLT_UV_NOT_REQUIRED_FLAG, CTAPCLT_UV_PREFERRED_FLAG, CTAPCLT_UV_REQUIRED_FLAG, E_ABORT, E_FAIL, E_NOTIMPL,
    RdpewaRequest, RdpewaResponse, RpcCommand, S_OK, UserVerification, WebAuthnPara, WebAuthnRequestBody,
    WebAuthnResponsePayload,
};

/// Server-initiated `CTAPCBOR_RPC_COMMAND_WEB_AUTHN` ceremony parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct RdpewaWebAuthnRequest {
    pub rp_id: Option<String>,
    pub timeout_ms: u32,
    pub transaction_id: [u8; 16],
    pub client_data_json: Vec<u8>,
    pub para: WebAuthnPara,
    pub body: WebAuthnRequestBody,
}

enum PendingRpc {
    Iuvpaa(Box<dyn FnOnce(Result<bool, u32>) + Send>),
    WebAuthn(Box<dyn FnOnce(Result<WebAuthnResponsePayload, u32>) + Send>),
    Cancel(Box<dyn FnOnce(Result<(), u32>) + Send>),
}

impl PendingRpc {
    fn fail(self, hresult: u32) {
        match self {
            Self::Iuvpaa(done) => done(Err(hresult)),
            Self::WebAuthn(done) => done(Err(hresult)),
            Self::Cancel(done) => done(Err(hresult)),
        }
    }
}

/// Server-side MS-RDPEWA DVC processor for one client-executed WebAuthn RPC.
///
/// Per [MS-RDPEWA] section 1, the server opens `WebAuthN_Channel`, sends one
/// CBOR request map, and completes when the client replies or closes the channel.
///
/// [MS-RDPEWA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpewa/3cca2157-ae15-4ec4-91f6-503bef9600fa
pub struct RdpewaServerRpc {
    request: Option<RdpewaRequest>,
    pending: Option<PendingRpc>,
    can_start: Option<Box<dyn Fn() -> bool + Send>>,
}

impl RdpewaServerRpc {
    /// Returns an inert processor that sends nothing when started.
    pub fn aborted() -> Self {
        Self {
            request: None,
            pending: None,
            can_start: None,
        }
    }

    /// Aborts before sending the request if `can_start` returns `false` when the channel opens.
    #[must_use]
    pub fn with_start_guard(mut self, can_start: Box<dyn Fn() -> bool + Send>) -> Self {
        self.can_start = Some(can_start);
        self
    }

    /// Queries `CTAPCBOR_RPC_COMMAND_IUVPAA` on a newly opened `WebAuthN_Channel`.
    pub fn iuvpaa(transaction_id: [u8; 16], on_complete: Box<dyn FnOnce(Result<bool, u32>) + Send>) -> Self {
        Self {
            request: Some(RdpewaRequest {
                command: RpcCommand::Iuvpaa,
                flags: 0,
                rp_id: None,
                timeout_ms: 0,
                transaction_id: transaction_id.to_vec(),
                client_data_json: None,
                webauthn_para: None,
                request_body: Vec::new(),
                raw: BTreeMap::new(),
            }),
            pending: Some(PendingRpc::Iuvpaa(on_complete)),
            can_start: None,
        }
    }

    /// Sends one `CTAPCBOR_RPC_COMMAND_WEB_AUTHN` MakeCredential or GetAssertion request.
    pub fn webauthn(
        request: RdpewaWebAuthnRequest,
        on_complete: Box<dyn FnOnce(Result<WebAuthnResponsePayload, u32>) + Send>,
    ) -> Self {
        let flags = match request.para.user_verification {
            UserVerification::Required => CTAPCLT_UV_REQUIRED_FLAG,
            UserVerification::Preferred => CTAPCLT_UV_PREFERRED_FLAG,
            UserVerification::Discouraged => CTAPCLT_UV_NOT_REQUIRED_FLAG,
            UserVerification::Any => 0,
        };
        Self {
            request: Some(RdpewaRequest {
                command: RpcCommand::WebAuthn,
                flags,
                rp_id: request.rp_id,
                timeout_ms: request.timeout_ms,
                transaction_id: request.transaction_id.to_vec(),
                client_data_json: Some(request.client_data_json),
                webauthn_para: Some(request.para),
                request_body: request.body.encode(),
                raw: BTreeMap::new(),
            }),
            pending: Some(PendingRpc::WebAuthn(on_complete)),
            can_start: None,
        }
    }

    /// Sends `CTAPCBOR_RPC_COMMAND_CANCEL_CUR_OP` for `cancellation_id`.
    pub fn cancel(
        cancellation_id: [u8; 16],
        transaction_id: [u8; 16],
        on_complete: Box<dyn FnOnce(Result<(), u32>) + Send>,
    ) -> Self {
        Self {
            request: Some(RdpewaRequest {
                command: RpcCommand::CancelCurOp,
                flags: 0,
                rp_id: None,
                timeout_ms: 0,
                transaction_id: transaction_id.to_vec(),
                client_data_json: None,
                webauthn_para: None,
                request_body: cancellation_id.to_vec(),
                raw: BTreeMap::new(),
            }),
            pending: Some(PendingRpc::Cancel(on_complete)),
            can_start: None,
        }
    }

    fn abort(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.fail(E_ABORT);
        }
    }
}

impl Drop for RdpewaServerRpc {
    fn drop(&mut self) {
        self.abort();
    }
}

impl_as_any!(RdpewaServerRpc);

impl DvcProcessor for RdpewaServerRpc {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(256 * 1024)
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if self.can_start.as_ref().is_some_and(|can_start| !can_start()) {
            self.request = None;
            self.abort();
            return Ok(Vec::new());
        }
        let Some(request) = self.request.take() else {
            return Ok(Vec::new());
        };
        let encoded = request.encode().map_err(|error| encode_err!(error))?;
        Ok(vec![Box::new(RawDvcRequest(encoded))])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let Some(pending) = self.pending.take() else {
            return Ok(Vec::new());
        };
        let response = match RdpewaResponse::decode(payload) {
            Ok(response) => response,
            Err(error) => {
                pending.fail(E_FAIL);
                return Err(decode_err!(error));
            }
        };
        if response.hresult != S_OK {
            pending.fail(response.hresult);
            return Ok(Vec::new());
        }
        match pending {
            PendingRpc::Iuvpaa(done) => match response.payload_u32() {
                Ok(available) => done(Ok(available != 0)),
                Err(error) => {
                    done(Err(E_FAIL));
                    return Err(decode_err!(error));
                }
            },
            PendingRpc::WebAuthn(done) => match WebAuthnResponsePayload::decode(&response.payload) {
                Ok(decoded) if !decoded.response.is_empty() => done(Ok(decoded)),
                Ok(_) => done(Err(E_FAIL)),
                Err(error) => {
                    done(Err(E_FAIL));
                    return Err(decode_err!(error));
                }
            },
            PendingRpc::Cancel(done) => done(Ok(())),
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.abort();
    }
}

impl DvcServerProcessor for RdpewaServerRpc {
    fn creation_failed(&mut self, _channel_id: u32, _status: ironrdp_dvc::pdu::CreationStatus) {
        self.abort();
    }
}

struct RawDvcRequest(Vec<u8>);

impl Encode for RawDvcRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.0.len());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RDPEWA_REQUEST"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawDvcRequest {}

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

    fn max_message_size(&self) -> Option<usize> {
        Some(256 * 1024)
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
