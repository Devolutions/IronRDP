//! Client side of the `Microsoft::Windows::RDS::InteractiveEntraAuth` dynamic virtual channel.
//!
//! A session host uses this channel to ask the client for a fresh Microsoft Entra sign-in during the session.
//! No Microsoft Open Specification documents it; the exchange follows the Windows App 2.0.1485.0 client:
//!
//! 1. The server sends a JSON request with `version` 1, a `scenario_id` GUID, and an optional `username_hint`.
//! 2. The client runs a TLS handshake inside the channel, as the TLS client.
//! 3. The server certificate's common name is the session host's Entra device ID. The embedder acquires an RDS AAD
//!    token for that device and supplies it with the tenant's Entra P2P root certificates, and the server certificate
//!    must be issued by one of those roots.
//! 4. Inside TLS, the client and server run the RDS AAD exchange of [MS-RDPBCGR] section 5.4.5.4.1.
//!
//! On failure the client sends `{"error_code": <HRESULT>}`, inside TLS once the handshake has completed.
//! Windows App also closes the channel then; a dynamic channel processor cannot close its own channel, so this client
//! stops answering instead.
//!
//! The [interactive Entra authentication section of `PROTOCOL.md`][protocol] describes the exchange in more detail.
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
//! [protocol]: https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp-w365/PROTOCOL.md#interactive-entra-authentication

use core::fmt;
use std::sync::Arc;

use ironrdp_connector::entra::{ENTRA_JSON_HINT, authentication_result, server_nonce, write_assertion};
use ironrdp_core::{Encode, EncodeResult, WriteBuf, WriteCursor, ensure_size, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduHint as _, PduResult};
use ironrdp_svc::ChannelFlags;
use picky::x509::Cert;
use picky::x509::date::UtcDate;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};
use zeroize::Zeroize as _;

use crate::entra::{EntraAuthSession, RdpResource};
use crate::output_channel::OutputEventSender;
use crate::rdp::{RdpInputEvent, RdpInputSender, RdpOutputEvent};

/// Dynamic virtual channel name.
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::InteractiveEntraAuth";

/// The scenario IDs Windows App accepts, without braces.
///
/// Windows App rejects any other scenario with `E_NOTIMPL` and does not otherwise use the ID.
const KNOWN_SCENARIOS: [&str; 2] = [
    "75cc6d06-3a48-43a1-bcd0-cb46eb8f9aae",
    "b419e9ce-25cd-44bc-8da5-6ff75edd1e28",
];

const E_NOTIMPL: u32 = 0x8000_4001;
const E_FAIL: u32 = 0x8000_4005;
const E_ACCESSDENIED: u32 = 0x8007_0005;
const E_INVALIDARG: u32 = 0x8007_0057;
/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`, sent when the embedder declines the sign-in.
const E_CANCELLED: u32 = 0x8007_04C7;

// Local resource limits, not protocol-defined maximums.
const MAX_REQUEST_SIZE: usize = 64 * 1024;
const TRANSPORT_BUFFER_SIZE: usize = 256 * 1024;

/// The TLS stream that runs inside the channel.
type TlsTunnel = ironrdp_tls::TlsStream<DuplexStream>;

/// A session host's request for an interactive Microsoft Entra sign-in.
///
/// Acquire an RDS AAD token for [`Self::resource`], typically by prompting the user, and answer with [`Self::grant`].
/// Dropping the request or calling [`Self::decline`] tells the session host that the sign-in was cancelled.
pub struct InteractiveEntraAuthRequest {
    scenario_id: String,
    username_hint: Option<String>,
    device_id: String,
    response: oneshot::Sender<InteractiveEntraAuthGrant>,
}

impl fmt::Debug for InteractiveEntraAuthRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InteractiveEntraAuthRequest")
            .field("scenario_id", &self.scenario_id)
            .field("username_hint", &self.username_hint)
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}

impl InteractiveEntraAuthRequest {
    /// The scenario GUID the session host sent, lowercase and without braces.
    ///
    /// The meaning of the two scenarios Windows App accepts is not documented.
    pub fn scenario_id(&self) -> &str {
        &self.scenario_id
    }

    /// The account the session host suggests signing in with, if it sent one.
    pub fn username_hint(&self) -> Option<&str> {
        self.username_hint.as_deref()
    }

    /// The session host's Entra device ID, taken from the common name of its certificate in the channel.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// The RDP resource to acquire the token for.
    pub fn resource(&self) -> RdpResource {
        RdpResource::DeviceId(self.device_id.clone())
    }

    /// Answers the request with a token for [`Self::resource`].
    pub fn grant(self, grant: InteractiveEntraAuthGrant) {
        // The exchange has ended if the receiver is gone, so there is nobody left to tell.
        let _ = self.response.send(grant);
    }

    /// Tells the session host that the sign-in was cancelled.
    pub fn decline(self) {}
}

/// The embedder's answer to an [`InteractiveEntraAuthRequest`].
pub struct InteractiveEntraAuthGrant {
    session: Arc<EntraAuthSession>,
    p2p_root_certificates: Vec<Vec<u8>>,
}

impl fmt::Debug for InteractiveEntraAuthGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InteractiveEntraAuthGrant")
            .field("session", &self.session)
            .field("p2p_root_certificates", &self.p2p_root_certificates.len())
            .finish()
    }
}

impl InteractiveEntraAuthGrant {
    /// Creates a grant from an RDS AAD session for the requested resource and the tenant's DER-encoded Entra P2P root
    /// certificates.
    ///
    /// The session host's certificate in the channel must be issued by one of `p2p_root_certificates`; with none, the
    /// sign-in is refused, as Windows App does.
    pub fn new(session: Arc<EntraAuthSession>, p2p_root_certificates: Vec<Vec<u8>>) -> Self {
        Self {
            session,
            p2p_root_certificates,
        }
    }
}

/// Client processor for `Microsoft::Windows::RDS::InteractiveEntraAuth`.
///
/// Each time the channel opens, it starts a task that runs one exchange; channel data is forwarded to that task.
pub(crate) struct InteractiveEntraAuthChannel {
    server_name: String,
    input_sender: RdpInputSender,
    output_event_sender: OutputEventSender,
    exchange: Option<Exchange>,
}

struct Exchange {
    incoming: mpsc::UnboundedSender<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Exchange {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl InteractiveEntraAuthChannel {
    /// Creates the processor; `server_name` is the TLS server name sent in the channel's handshake.
    pub(crate) fn new(
        server_name: String,
        input_sender: RdpInputSender,
        output_event_sender: OutputEventSender,
    ) -> Self {
        Self {
            server_name,
            input_sender,
            output_event_sender,
            exchange: None,
        }
    }
}

impl_as_any!(InteractiveEntraAuthChannel);

impl DvcProcessor for InteractiveEntraAuthChannel {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            warn!("Interactive Entra authentication channel opened outside a Tokio runtime");
            return Ok(Vec::new());
        };
        let (incoming, receiver) = mpsc::unbounded_channel();
        let context = ExchangeContext {
            channel_id,
            server_name: self.server_name.clone(),
            input_sender: self.input_sender.clone(),
            output_event_sender: self.output_event_sender.clone(),
        };
        let task = runtime.spawn(run_exchange(context, receiver));
        self.exchange = Some(Exchange { incoming, task });
        debug!(channel_id, "Interactive Entra authentication channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if let Some(exchange) = &self.exchange {
            // A closed receiver means the exchange has finished and later data has no reader.
            let _ = exchange.incoming.send(payload.to_vec());
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.exchange = None;
        debug!("Interactive Entra authentication channel closed");
    }
}

impl DvcClientProcessor for InteractiveEntraAuthChannel {}

struct ExchangeContext {
    channel_id: u32,
    server_name: String,
    input_sender: RdpInputSender,
    output_event_sender: OutputEventSender,
}

impl ExchangeContext {
    /// Sends `data` on the channel, returning `false` once the session is gone.
    async fn send(&self, data: Vec<u8>) -> bool {
        let messages = match encode_dvc_messages(self.channel_id, vec![Box::new(RawData(data))], ChannelFlags::empty())
        {
            Ok(messages) => messages,
            Err(error) => {
                warn!(%error, "Unable to encode interactive Entra authentication data");
                return false;
            }
        };
        self.input_sender
            .send(RdpInputEvent::SendDvcMessages {
                channel_id: self.channel_id,
                messages,
            })
            .await
    }
}

struct RawData(Vec<u8>);

impl Encode for RawData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "InteractiveEntraAuthData"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawData {}

/// Runs one exchange: the server's request, then the TLS tunnel carrying the RDS AAD exchange.
///
/// UNTESTED: no test runs the exchange against a TLS server, so the tunnel, the event round trip, and the error
/// reporting inside TLS are checked only by inspection. The request parsing, error encoding, and certificate checks
/// it relies on are tested below.
async fn run_exchange(context: ExchangeContext, mut incoming: mpsc::UnboundedReceiver<Vec<u8>>) {
    let Some(request) = incoming.recv().await else {
        return;
    };
    let request = match parse_request(&request) {
        Ok(request) => request,
        Err(hresult) => {
            warn!(hresult, "Rejecting an interactive Entra authentication request");
            context.send(error_message(hresult)).await;
            return;
        }
    };
    debug!(scenario_id = %request.scenario_id, "Interactive Entra authentication requested");

    let (tls_io, transport) = tokio::io::duplex(TRANSPORT_BUFFER_SIZE);
    tokio::join!(
        pump_transport(&context, transport, incoming),
        authenticate(&context, request, tls_io)
    );
}

/// Copies channel data into the TLS transport and TLS output onto the channel until the TLS side is dropped.
async fn pump_transport(
    context: &ExchangeContext,
    transport: DuplexStream,
    mut incoming: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let (mut reader, mut writer) = tokio::io::split(transport);
    let to_tls = async {
        while let Some(data) = incoming.recv().await {
            // Once the TLS side is gone, later channel data is discarded.
            let _ = writer.write_all(&data).await;
        }
    };
    let from_tls = async {
        let mut buffer = vec![0; TRANSPORT_BUFFER_SIZE];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if !context.send(buffer[..read].to_vec()).await {
                        break;
                    }
                }
            }
        }
    };
    tokio::pin!(to_tls);
    tokio::select! {
        () = from_tls => {}
        () = &mut to_tls => {}
    }
}

async fn authenticate(context: &ExchangeContext, request: ParsedRequest, tls_io: DuplexStream) {
    // The certificate is checked against the embedder's tenant roots once the token is granted, as Windows App does.
    let (mut tls, certificate) = match ironrdp_tls::upgrade_with_certificate_validation(
        tls_io,
        &context.server_name,
        ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate,
    )
    .await
    {
        Ok(upgraded) => upgraded,
        Err(error) => {
            warn!(%error, "Interactive Entra authentication TLS handshake failed");
            context.send(error_message(E_FAIL)).await;
            return;
        }
    };

    match authenticate_over_tls(context, request, &mut tls, &certificate).await {
        Ok(hresult) => {
            debug!(hresult, "Interactive Entra authentication completed");
            // A closed output channel means the session is ending and nobody is left to tell.
            let _ = context
                .output_event_sender
                .send(RdpOutputEvent::InteractiveEntraAuthCompleted { hresult })
                .await;
        }
        Err(hresult) => {
            warn!(hresult, "Interactive Entra authentication failed");
            // The report is best effort: a broken tunnel cannot carry it either.
            if tls.write_all(&error_message(hresult)).await.is_ok() {
                let _ = tls.flush().await;
            }
        }
    }
}

async fn authenticate_over_tls(
    context: &ExchangeContext,
    request: ParsedRequest,
    tls: &mut TlsTunnel,
    certificate: &x509_cert::Certificate,
) -> Result<u32, u32> {
    use x509_cert::der::Encode as _;

    let certificate = certificate.to_der().map_err(|_| E_FAIL)?;
    let device_id = device_id_from_certificate(&certificate)?;

    let (response, granted) = oneshot::channel();
    let event = RdpOutputEvent::InteractiveEntraAuthRequested(InteractiveEntraAuthRequest {
        scenario_id: request.scenario_id,
        username_hint: request.username_hint,
        device_id: device_id.clone(),
        response,
    });
    context.output_event_sender.send(event).await.map_err(|_| E_CANCELLED)?;
    let grant = granted.await.map_err(|_| E_CANCELLED)?;

    let expected_resource = RdpResource::DeviceId(device_id).uri().map_err(|_| E_FAIL)?;
    if !grant.session.resource_uri().eq_ignore_ascii_case(&expected_resource) {
        warn!("The granted Entra token is not for the requesting session host");
        return Err(E_ACCESSDENIED);
    }
    verify_server_certificate(&certificate, &grant.p2p_root_certificates, &UtcDate::now())?;

    let mut reader = MessageReader::default();
    let server_nonce = server_nonce(&reader.next(tls).await?).map_err(|error| {
        warn!(%error, "Invalid RDS AAD server nonce");
        E_FAIL
    })?;
    let aad_nonce = grant.session.acquire_aad_nonce().await.map_err(|error| {
        warn!(%error, "Unable to acquire an Entra AAD nonce");
        E_FAIL
    })?;
    let assertion = grant
        .session
        .assertion(&server_nonce, &aad_nonce)
        .await
        .map_err(|error| {
            warn!(%error, "Unable to sign the RDP assertion");
            E_FAIL
        })?;
    let mut output = WriteBuf::new();
    write_assertion(assertion.as_str(), &mut output).map_err(|_| E_FAIL)?;
    let written = tls.write_all(output.filled()).await;
    output.filled_mut().zeroize();
    written.map_err(|_| E_FAIL)?;
    tls.flush().await.map_err(|_| E_FAIL)?;

    authentication_result(&reader.next(tls).await?).map_err(|error| {
        warn!(%error, "Invalid RDS AAD authentication result");
        E_FAIL
    })
}

/// Reads one NUL-terminated RDS AAD JSON message at a time from the TLS stream.
#[derive(Default)]
struct MessageReader {
    buffer: Vec<u8>,
}

impl MessageReader {
    async fn next(&mut self, stream: &mut TlsTunnel) -> Result<Vec<u8>, u32> {
        loop {
            match ENTRA_JSON_HINT.find_size(&self.buffer) {
                Ok(Some((_, length))) => return Ok(self.buffer.drain(..length).collect()),
                Ok(None) => {}
                Err(error) => {
                    warn!(%error, "Invalid RDS AAD message");
                    return Err(E_FAIL);
                }
            }
            let mut chunk = [0; 4096];
            let read = stream.read(&mut chunk).await.map_err(|_| E_FAIL)?;
            if read == 0 {
                return Err(E_FAIL);
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedRequest {
    scenario_id: String,
    username_hint: Option<String>,
}

/// Parses the server's request, returning the HRESULT Windows App reports for a rejected one.
fn parse_request(input: &[u8]) -> Result<ParsedRequest, u32> {
    if MAX_REQUEST_SIZE < input.len() {
        return Err(E_INVALIDARG);
    }
    let trimmed = input.strip_suffix(&[0]).unwrap_or(input);
    let request: Value = serde_json::from_slice(trimmed).map_err(|_| E_INVALIDARG)?;
    let version = request.get("version").and_then(Value::as_i64).ok_or(E_INVALIDARG)?;
    if version != 1 {
        return Err(E_NOTIMPL);
    }
    let scenario_id = request.get("scenario_id").and_then(Value::as_str).ok_or(E_INVALIDARG)?;
    let scenario_id = scenario_id
        .strip_prefix('{')
        .and_then(|id| id.strip_suffix('}'))
        .unwrap_or(scenario_id)
        .to_ascii_lowercase();
    if !KNOWN_SCENARIOS.contains(&scenario_id.as_str()) {
        return Err(E_NOTIMPL);
    }
    let username_hint = request
        .get("username_hint")
        .and_then(Value::as_str)
        .filter(|hint| !hint.is_empty())
        .map(str::to_owned);
    Ok(ParsedRequest {
        scenario_id,
        username_hint,
    })
}

/// Encodes the client's failure report.
///
/// The HRESULT is written as a signed 32-bit integer; whether Windows App writes it signed was not determined.
fn error_message(hresult: u32) -> Vec<u8> {
    let code = i32::from_ne_bytes(hresult.to_ne_bytes());
    serde_json::json!({ "error_code": code }).to_string().into_bytes()
}

/// Reads the session host's Entra device ID from its certificate's common name.
fn device_id_from_certificate(certificate: &[u8]) -> Result<String, u32> {
    let certificate = Cert::from_der(certificate).map_err(|_| E_FAIL)?;
    let device_id = certificate
        .subject_name()
        .find_common_name()
        .map(|name| name.to_utf8_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .ok_or(E_FAIL)?;
    Ok(device_id)
}

/// Checks that the session host's certificate is valid at `now` and issued by one of the tenant's P2P roots.
fn verify_server_certificate(certificate: &[u8], p2p_roots: &[Vec<u8>], now: &UtcDate) -> Result<(), u32> {
    let certificate = Cert::from_der(certificate).map_err(|_| E_ACCESSDENIED)?;
    let issued_by_root = p2p_roots
        .iter()
        .filter_map(|root| Cert::from_der(root).ok())
        .any(|root| {
            certificate
                .verifier()
                .exact_date(now)
                .chain(core::iter::once(&root))
                .verify()
                .is_ok()
        });
    if issued_by_root { Ok(()) } else { Err(E_ACCESSDENIED) }
}

#[cfg(test)]
mod tests {
    use picky::key::PrivateKey;
    use picky::x509::certificate::CertificateBuilder;
    use picky::x509::name::DirectoryName;
    use rstest::rstest;

    use super::*;

    const DEVICE_ID: &str = "8f2b6a1e-1d2c-4b5a-9e7f-0a1b2c3d4e5f";

    struct Certificates {
        root: Vec<u8>,
        other_root: Vec<u8>,
        leaf: Vec<u8>,
    }

    /// RSA key generation is slow in debug builds, so every test shares one set of certificates.
    fn certificates() -> &'static Certificates {
        static CERTIFICATES: std::sync::LazyLock<Certificates> = std::sync::LazyLock::new(generate_certificates);
        &CERTIFICATES
    }

    fn generate_certificates() -> Certificates {
        let root_key = PrivateKey::generate_rsa(2048).expect("root key");
        let other_root_key = PrivateKey::generate_rsa(2048).expect("other root key");
        let leaf_key = PrivateKey::generate_rsa(2048).expect("leaf key");
        let valid_from = UtcDate::ymd(2020, 1, 1).expect("date");
        let valid_to = UtcDate::ymd(2040, 1, 1).expect("date");
        let root = CertificateBuilder::new()
            .validity(valid_from.clone(), valid_to.clone())
            .self_signed(
                DirectoryName::new_common_name("MS-Organization-P2P-Access [2026]"),
                &root_key,
            )
            .ca(true)
            .build()
            .expect("root certificate");
        let other_root = CertificateBuilder::new()
            .validity(valid_from.clone(), valid_to)
            .self_signed(DirectoryName::new_common_name("Unrelated Root"), &other_root_key)
            .ca(true)
            .build()
            .expect("other root certificate");
        let leaf = CertificateBuilder::new()
            .validity(valid_from, UtcDate::ymd(2030, 1, 1).expect("date"))
            .subject(
                DirectoryName::new_common_name(DEVICE_ID),
                leaf_key.to_public_key().expect("leaf public key"),
            )
            .issuer_cert(&root, &root_key)
            .build()
            .expect("leaf certificate");
        Certificates {
            root: root.to_der().expect("root DER"),
            other_root: other_root.to_der().expect("other root DER"),
            leaf: leaf.to_der().expect("leaf DER"),
        }
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Request, which reads version, scenario_id, and username_hint
    #[test]
    fn request_with_a_known_scenario_is_accepted() {
        let request = br#"{"version":1,"scenario_id":"{75CC6D06-3A48-43A1-BCD0-CB46EB8F9AAE}","username_hint":"user@contoso.com"}"#;

        assert_eq!(
            parse_request(request),
            Ok(ParsedRequest {
                scenario_id: "75cc6d06-3a48-43a1-bcd0-cb46eb8f9aae".to_owned(),
                username_hint: Some("user@contoso.com".to_owned()),
            })
        );
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Request, whose username_hint is optional
    #[test]
    fn username_hint_is_optional() {
        let request = b"{\"version\":1,\"scenario_id\":\"b419e9ce-25cd-44bc-8da5-6ff75edd1e28\"}\0";

        assert_eq!(parse_request(request).map(|request| request.username_hint), Ok(None));
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Request, which fails these requests with E_NOTIMPL or E_INVALIDARG
    #[rstest]
    #[case::version_2(br#"{"version":2,"scenario_id":"75cc6d06-3a48-43a1-bcd0-cb46eb8f9aae"}"#.as_slice(), E_NOTIMPL)]
    #[case::unknown_scenario(br#"{"version":1,"scenario_id":"00000000-0000-0000-0000-000000000000"}"#.as_slice(), E_NOTIMPL)]
    #[case::missing_scenario(br#"{"version":1}"#.as_slice(), E_INVALIDARG)]
    #[case::not_json(b"not json".as_slice(), E_INVALIDARG)]
    fn invalid_requests_are_rejected(#[case] request: &[u8], #[case] hresult: u32) {
        assert_eq!(parse_request(request), Err(hresult));
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Errors
    #[test]
    fn error_message_carries_the_hresult() {
        assert_eq!(error_message(E_ACCESSDENIED), br#"{"error_code":-2147024891}"#);
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Server certificate
    #[test]
    fn device_id_comes_from_the_common_name() {
        let certificates = certificates();

        assert_eq!(device_id_from_certificate(&certificates.leaf), Ok(DEVICE_ID.to_owned()));
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Server certificate
    #[test]
    fn certificate_issued_by_a_p2p_root_is_accepted() {
        let certificates = certificates();
        let now = UtcDate::ymd(2026, 10, 28).expect("date");

        let roots = [certificates.other_root.clone(), certificates.root.clone()];

        assert_eq!(verify_server_certificate(&certificates.leaf, &roots, &now), Ok(()));
    }

    // oracle: reference Windows App 2.0.1485.0 as recorded in crates/ironrdp-w365/PROTOCOL.md §Interactive Entra authentication, Server certificate, which fails with E_ACCESSDENIED
    #[derive(Clone, Copy)]
    enum Roots {
        None,
        Unrelated,
        Issuer,
    }

    #[rstest]
    #[case::no_roots(Roots::None, (2026, 10, 28))]
    #[case::unrelated_root(Roots::Unrelated, (2026, 10, 28))]
    #[case::expired(Roots::Issuer, (2031, 1, 1))]
    fn other_certificates_are_refused(#[case] roots: Roots, #[case] (year, month, day): (u16, u8, u8)) {
        let certificates = certificates();
        let now = UtcDate::ymd(year, month, day).expect("date");
        let roots = match roots {
            Roots::None => Vec::new(),
            Roots::Unrelated => vec![certificates.other_root.clone()],
            Roots::Issuer => vec![certificates.root.clone()],
        };

        assert_eq!(
            verify_server_certificate(&certificates.leaf, &roots, &now),
            Err(E_ACCESSDENIED)
        );
    }
}
