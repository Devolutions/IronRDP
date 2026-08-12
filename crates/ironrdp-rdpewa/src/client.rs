//! MS-RDPEWA client DVC processor.

use std::sync::Arc;

use ironrdp_core::{Encode, EncodeResult, WriteCursor, ensure_size, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, decode_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{
    DeviceInfo, E_FAIL, E_NOTIMPL, RdpewaRequest, RdpewaResponse, RpcCommand, S_OK, WebAuthnPara,
    WebAuthnResponsePayload, WebAuthnSubcommand,
};

/// Result type for platform handler operations.
pub type RdpewaResult<T> = Result<T, RdpewaHandlerError>;

/// Handler-level error mapped to an HRESULT for the wire response.
#[derive(Debug)]
pub struct RdpewaHandlerError {
    pub hresult: u32,
    pub message: &'static str,
}

impl RdpewaHandlerError {
    pub fn new(hresult: u32, message: &'static str) -> Self {
        Self { hresult, message }
    }

    pub fn not_impl(message: &'static str) -> Self {
        Self::new(E_NOTIMPL, message)
    }

    pub fn fail(message: &'static str) -> Self {
        Self::new(E_FAIL, message)
    }
}

impl core::fmt::Display for RdpewaHandlerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} (HRESULT 0x{:08X})", self.message, self.hresult)
    }
}

impl core::error::Error for RdpewaHandlerError {}

/// Inputs for MakeCredential / GetAssertion.
#[derive(Debug, Clone)]
pub struct WebAuthnOperationRequest {
    pub subcommand: WebAuthnSubcommand,
    pub rp_id: Option<String>,
    pub timeout_ms: u32,
    pub transaction_id: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub para: WebAuthnPara,
    /// CTAP CBOR map (without subcommand byte).
    pub ctap_cbor: Vec<u8>,
}

/// Successful WebAuthn operation result from the platform backend.
#[derive(Debug, Clone)]
pub struct WebAuthnOperationResponse {
    pub device_info: DeviceInfo,
    pub status: u32,
    /// CTAP status byte + CBOR body.
    pub response: Vec<u8>,
}

/// Platform abstraction for MS-RDPEWA client work.
///
/// Long-running WebAuthn UI operations should complete asynchronously via
/// [`RdpewaClientHandler::begin_webauthn`] / [`RdpewaResponseSender`].
pub trait RdpewaClientHandler: Send {
    /// Windows WebAuthn API version number.
    fn api_version(&mut self) -> RdpewaResult<u32>;

    /// `WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable`.
    fn is_uvpaa(&mut self) -> RdpewaResult<bool>;

    /// Cancel an in-flight operation identified by `cancellation_id`.
    fn cancel_current_operation(&mut self, cancellation_id: &[u8]) -> RdpewaResult<()>;

    /// Begin MakeCredential or GetAssertion.
    ///
    /// Return [`WebAuthnDispatch::Sync`] when the result is immediately available
    /// (tests / non-UI paths). Return [`WebAuthnDispatch::Async`] when the backend
    /// will later deliver a response through the provided sender.
    fn begin_webauthn(
        &mut self,
        request: WebAuthnOperationRequest,
        reply: RdpewaResponseSender,
    ) -> RdpewaResult<WebAuthnDispatch>;
}

/// How the handler handles a WEB_AUTHN request.
#[derive(Debug)]
pub enum WebAuthnDispatch {
    /// Immediate result (encoded as WEB_AUTHN payload).
    Sync(WebAuthnOperationResponse),
    /// Completion will be delivered later via [`RdpewaResponseSender`].
    Async,
}

/// Callback used to inject completed DVC messages into the active session loop.
pub type RdpewaWriteCallback = Arc<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send + Sync>;

/// Sends a completed RDPEWA response PDU back into the session.
///
/// Production wiring encodes the response as DVC SVC messages and posts them through
/// the session input channel (`SendDvcMessages`).
pub struct RdpewaResponseSender {
    inner: Box<dyn FnMut(Vec<u8>) + Send>,
}

impl core::fmt::Debug for RdpewaResponseSender {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RdpewaResponseSender")
    }
}

impl RdpewaResponseSender {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        Self { inner: Box::new(f) }
    }

    /// No-op sender used when async completion is not wired.
    pub fn sink() -> Self {
        Self::new(|_| {})
    }

    pub fn send_raw(&mut self, response_pdu: Vec<u8>) {
        (self.inner)(response_pdu);
    }

    pub fn send(&mut self, response: RdpewaResponse) {
        self.send_raw(response.to_bytes());
    }

    pub fn send_webauthn(&mut self, hresult: u32, payload: &WebAuthnResponsePayload) {
        match payload.encode() {
            Ok(bytes) => self.send(RdpewaResponse::with_payload(hresult, bytes)),
            Err(_) => self.send(RdpewaResponse::from_hresult(E_FAIL)),
        }
    }
}

/// Factory invoked for each async WEB_AUTHN request to obtain a response sender.
///
/// Prefer [`RdpewaClient::with_write_callback`] for production wiring. Keep this factory
/// for unit tests that assert on raw response bytes without DVC framing.
pub type RdpewaResponseSenderFactory = Box<dyn FnMut() -> RdpewaResponseSender + Send>;

/// Client-side MS-RDPEWA DVC processor.
pub struct RdpewaClient {
    handler: Box<dyn RdpewaClientHandler>,
    write_callback: Option<RdpewaWriteCallback>,
    sender_factory: Option<RdpewaResponseSenderFactory>,
    channel_id: Option<u32>,
}

impl RdpewaClient {
    pub fn new(handler: Box<dyn RdpewaClientHandler>) -> Self {
        Self {
            handler,
            write_callback: None,
            sender_factory: None,
            channel_id: None,
        }
    }

    /// Wire async WEB_AUTHN completions into the active session via `SendDvcMessages`.
    ///
    /// The callback receives the DVC channel id and fully framed DRDYNVC SVC messages.
    #[must_use]
    pub fn with_write_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send + Sync + 'static,
    {
        self.write_callback = Some(Arc::new(callback));
        self
    }

    /// Override the response-sender factory (primarily for tests).
    ///
    /// When set, this takes precedence over [`Self::with_write_callback`].
    #[must_use]
    pub fn with_response_sender_factory<F>(mut self, factory: F) -> Self
    where
        F: FnMut() -> RdpewaResponseSender + Send + 'static,
    {
        self.sender_factory = Some(Box::new(factory));
        self
    }

    fn make_reply_sender(&mut self) -> RdpewaResponseSender {
        if let Some(factory) = self.sender_factory.as_mut() {
            return factory();
        }

        let Some(callback) = self.write_callback.clone() else {
            return RdpewaResponseSender::sink();
        };

        let channel_id = self.channel_id.unwrap_or(0);
        RdpewaResponseSender::new(move |response_pdu| {
            let messages = match encode_dvc_messages(
                channel_id,
                vec![Box::new(RawDataDvcMessage(response_pdu))],
                ChannelFlags::SHOW_PROTOCOL,
            ) {
                Ok(messages) => messages,
                Err(error) => {
                    warn!(%error, channel_id, "Failed to encode async RDPEWA DVC response");
                    return;
                }
            };

            if let Err(error) = callback(channel_id, messages) {
                warn!(%error, channel_id, "Failed to submit async RDPEWA DVC response");
            }
        })
    }

    fn handle_request(&mut self, request: RdpewaRequest) -> PduResult<Option<RdpewaResponse>> {
        match request.command {
            RpcCommand::ApiVersion => {
                let version = self.handler.api_version().unwrap_or_else(|e| {
                    warn!(error = %e, "api_version failed");
                    0
                });
                let hresult = if version == 0 { E_FAIL } else { S_OK };
                if hresult == S_OK {
                    Ok(Some(RdpewaResponse::with_u32(S_OK, version)))
                } else {
                    Ok(Some(RdpewaResponse::from_hresult(hresult)))
                }
            }
            RpcCommand::Iuvpaa => match self.handler.is_uvpaa() {
                Ok(available) => Ok(Some(RdpewaResponse::with_u32(S_OK, u32::from(available)))),
                Err(e) => {
                    warn!(error = %e, "is_uvpaa failed");
                    Ok(Some(RdpewaResponse::from_hresult(e.hresult)))
                }
            },
            RpcCommand::CancelCurOp => {
                let cancellation_id = request
                    .webauthn_para
                    .as_ref()
                    .and_then(|p| p.cancellation_id.as_deref())
                    .unwrap_or(&[]);
                match self.handler.cancel_current_operation(cancellation_id) {
                    Ok(()) => Ok(Some(RdpewaResponse::ok_empty())),
                    Err(e) => {
                        warn!(error = %e, "cancel_current_operation failed");
                        Ok(Some(RdpewaResponse::from_hresult(e.hresult)))
                    }
                }
            }
            RpcCommand::WebAuthn => self.handle_webauthn(request),
            RpcCommand::GetCredentials | RpcCommand::GetAuthenticatorList => {
                Ok(Some(RdpewaResponse::from_hresult(E_NOTIMPL)))
            }
        }
    }

    fn handle_webauthn(&mut self, request: RdpewaRequest) -> PduResult<Option<RdpewaResponse>> {
        let body = match request.webauthn_body() {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "invalid WEB_AUTHN request body");
                return Ok(Some(RdpewaResponse::from_hresult(crate::pdu::E_INVALIDARG)));
            }
        };

        let client_data_json = request.client_data_json.clone().unwrap_or_default();
        let para = request.webauthn_para.clone().unwrap_or_default();
        let op = WebAuthnOperationRequest {
            subcommand: body.subcommand,
            rp_id: request.rp_id.clone(),
            timeout_ms: request.timeout_ms,
            transaction_id: request.transaction_id.clone(),
            client_data_json,
            para,
            ctap_cbor: body.ctap_cbor,
        };

        let reply = self.make_reply_sender();
        match self.handler.begin_webauthn(op, reply) {
            Ok(WebAuthnDispatch::Sync(result)) => {
                let payload = WebAuthnResponsePayload {
                    device_info: result.device_info,
                    status: result.status,
                    response: result.response,
                };
                match payload.encode() {
                    Ok(bytes) => Ok(Some(RdpewaResponse::with_payload(S_OK, bytes))),
                    Err(e) => {
                        warn!(error = %e, "failed to encode WEB_AUTHN response");
                        Ok(Some(RdpewaResponse::from_hresult(E_FAIL)))
                    }
                }
            }
            Ok(WebAuthnDispatch::Async) => {
                debug!("WEB_AUTHN operation dispatched asynchronously");
                Ok(None)
            }
            Err(e) => {
                warn!(error = %e, "begin_webauthn failed");
                Ok(Some(RdpewaResponse::from_hresult(e.hresult)))
            }
        }
    }
}

impl_as_any!(RdpewaClient);

impl DvcProcessor for RdpewaClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        debug!(channel_id, "RDPEWA channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let request = RdpewaRequest::decode(payload).map_err(|e| decode_err!(e))?;
        debug!(?request.command, "Received RDPEWA request");

        match self.handle_request(request)? {
            Some(response) => Ok(vec![Box::new(response)]),
            None => Ok(Vec::new()),
        }
    }
}

impl DvcClientProcessor for RdpewaClient {}

struct RawDataDvcMessage(Vec<u8>);

impl Encode for RawDataDvcMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RdpewaRawDataDvcMessage"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawDataDvcMessage {}

/// Simple in-process handler for unit tests.
#[derive(Debug, Default)]
pub struct StubRdpewaHandler {
    pub api_version: u32,
    pub uvpaa: bool,
}

impl RdpewaClientHandler for StubRdpewaHandler {
    fn api_version(&mut self) -> RdpewaResult<u32> {
        Ok(self.api_version)
    }

    fn is_uvpaa(&mut self) -> RdpewaResult<bool> {
        Ok(self.uvpaa)
    }

    fn cancel_current_operation(&mut self, _cancellation_id: &[u8]) -> RdpewaResult<()> {
        Ok(())
    }

    fn begin_webauthn(
        &mut self,
        _request: WebAuthnOperationRequest,
        _reply: RdpewaResponseSender,
    ) -> RdpewaResult<WebAuthnDispatch> {
        Err(RdpewaHandlerError::not_impl("stub webauthn"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_version_sync_response() {
        let handler = StubRdpewaHandler {
            api_version: 4,
            uvpaa: true,
        };
        let mut client = RdpewaClient::new(Box::new(handler));
        let req = RdpewaRequest {
            command: RpcCommand::ApiVersion,
            flags: 0,
            rp_id: None,
            timeout_ms: 0,
            transaction_id: vec![0; 16],
            client_data_json: None,
            webauthn_para: None,
            request_body: Vec::new(),
            raw: Default::default(),
        };
        let encoded = req.encode().unwrap();
        let _ = client.start(7).unwrap();
        let messages = client.process(7, &encoded).unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn async_sender_encodes_dvc_messages() {
        use std::sync::Mutex;

        let captured = Arc::new(Mutex::new(None));
        let captured_cb = Arc::clone(&captured);
        let mut client = RdpewaClient::new(Box::new(StubRdpewaHandler {
            api_version: 1,
            uvpaa: false,
        }))
        .with_write_callback(move |channel_id, messages| {
            *captured_cb.lock().unwrap() = Some((channel_id, messages.len()));
            Ok(())
        });

        let _ = client.start(42).unwrap();
        let mut sender = client.make_reply_sender();
        sender.send(RdpewaResponse::with_u32(S_OK, 1));

        let (channel_id, message_count) = captured.lock().unwrap().take().expect("callback invoked");
        assert_eq!(channel_id, 42);
        assert!(message_count >= 1);
    }
}
