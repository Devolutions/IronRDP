use ironrdp_async::{Framed, FramedRead, FramedWrite};
use ironrdp_connector::sspi::credssp::{CredSspMode, TsRequest, write_ts_credentials};
use ironrdp_connector::sspi::{AuthIdentityBuffers, CredentialsBuffers};
use ironrdp_connector::{ConnectorResult, ServerName, custom_err, general_err};
use ironrdp_pdu::PduHint;
use sha2::{Digest as _, Sha256};
use tracing::{debug, trace};
use windows::Win32::Foundation::{SEC_E_OK, SEC_I_COMPLETE_AND_CONTINUE, SEC_I_COMPLETE_NEEDED, SEC_I_CONTINUE_NEEDED};
use windows::Win32::Security::Authentication::Identity::{
    AcquireCredentialsHandleW, CompleteAuthToken, DecryptMessage, DeleteSecurityContext, EncryptMessage,
    FreeCredentialsHandle, ISC_REQ_CONFIDENTIALITY, ISC_REQ_FLAGS, ISC_REQ_INTEGRITY, ISC_REQ_MUTUAL_AUTH,
    ISC_REQ_USE_SESSION_KEY, ISC_RET_CONFIDENTIALITY, ISC_RET_INTEGRITY, InitializeSecurityContextW,
    QueryContextAttributesW, SECBUFFER_DATA, SECBUFFER_EMPTY, SECBUFFER_TOKEN, SECBUFFER_VERSION, SECPKG_ATTR_SIZES,
    SECPKG_CRED_OUTBOUND, SECURITY_NATIVE_DREP, SecBuffer, SecBufferDesc, SecPkgContext_Sizes,
};
use windows::Win32::Security::Credentials::SecHandle;
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::core::{PCWSTR, w};

const TS_REQUEST_VERSION: u32 = 6;
const NONCE_SIZE: usize = 32;
const MAX_NEGOTIATE_TOKEN_SIZE: usize = 64 * 1024;
const CLIENT_SERVER_HASH_MAGIC: &[u8] = b"CredSSP Client-To-Server Binding Hash\0";
const SERVER_CLIENT_HASH_MAGIC: &[u8] = b"CredSSP Server-To-Client Binding Hash\0";
const VMCONNECT_AUTHENTICATION_SERVICE_CLASS: &str = "Microsoft Virtual Console Service";

#[derive(Clone, Copy, Debug)]
struct CredsspTsRequestHint;

const CREDSSP_TS_REQUEST_HINT: CredsspTsRequestHint = CredsspTsRequestHint;

impl PduHint for CredsspTsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        match TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some((true, length))),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(ironrdp_core::other_err!("CredsspTsRequestHint", source: error)),
        }
    }
}

pub(super) async fn perform<S>(
    framed: &mut Framed<S>,
    server_name: ServerName,
    server_public_key: &[u8],
) -> ConnectorResult<()>
where
    S: FramedRead + FramedWrite,
{
    let mut sequence = NativeCredssp::new(server_name, server_public_key)?;
    let mut incoming = None;

    loop {
        let outgoing = sequence.process(incoming.take())?;
        let mut encoded = Vec::with_capacity(usize::from(outgoing.buffer_len()));
        outgoing
            .encode_ts_request(&mut encoded)
            .map_err(|error| custom_err!("encode native CredSSP request", error))?;
        trace!(length = encoded.len(), "Send native CredSSP request");
        framed
            .write_all(&encoded)
            .await
            .map_err(|error| custom_err!("write native CredSSP request", error))?;

        if sequence.is_finished() {
            return Ok(());
        }

        let response = framed
            .read_by_hint(&CREDSSP_TS_REQUEST_HINT)
            .await
            .map_err(|error| custom_err!("read native CredSSP response", error))?;
        trace!(length = response.len(), "Received native CredSSP response");
        incoming = Some(
            TsRequest::from_buffer(&response).map_err(|error| custom_err!("decode native CredSSP response", error))?,
        );
    }
}

#[derive(Debug)]
enum State {
    NegoToken,
    PubKeyAuth,
    Finished,
}

struct NativeCredssp {
    state: State,
    security: NativeSecurityContext,
    public_key: Vec<u8>,
    nonce: [u8; NONCE_SIZE],
    peer_version: Option<u32>,
}

impl NativeCredssp {
    fn new(server_name: ServerName, public_key: &[u8]) -> ConnectorResult<Self> {
        let mut nonce = [0u8; NONCE_SIZE];
        // SAFETY: `nonce` is a writable byte slice and the system-preferred RNG needs no algorithm handle.
        unsafe { BCryptGenRandom(None, &mut nonce, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
            .ok()
            .map_err(|error| custom_err!("generate native CredSSP nonce", error))?;

        let target_name = format!("{VMCONNECT_AUTHENTICATION_SERVICE_CLASS}/{}", server_name.into_inner());
        Ok(Self {
            state: State::NegoToken,
            security: NativeSecurityContext::new(&target_name)?,
            public_key: public_key.to_vec(),
            nonce,
            peer_version: None,
        })
    }

    fn is_finished(&self) -> bool {
        matches!(self.state, State::Finished)
    }

    fn process(&mut self, incoming: Option<TsRequest>) -> ConnectorResult<TsRequest> {
        match self.state {
            State::NegoToken => self.process_nego_token(incoming),
            State::PubKeyAuth => self.process_pub_key_auth(incoming),
            State::Finished => Err(general_err!("native CredSSP sequence is already finished")),
        }
    }

    fn process_nego_token(&mut self, incoming: Option<TsRequest>) -> ConnectorResult<TsRequest> {
        let input_token = if let Some(request) = incoming {
            request
                .check_error()
                .map_err(|error| custom_err!("native CredSSP server status", error))?;
            self.observe_peer_version(request.version)?;
            request.nego_tokens.unwrap_or_default()
        } else {
            Vec::new()
        };

        let step = self.security.initialize(&input_token)?;
        let mut request = TsRequest {
            version: TS_REQUEST_VERSION,
            nego_tokens: (!step.token.is_empty()).then_some(step.token),
            auth_info: None,
            pub_key_auth: None,
            error_code: None,
            client_nonce: None,
        };

        if step.complete {
            let peer_version = self
                .peer_version
                .ok_or_else(|| general_err!("native CredSSP completed before receiving the server version"))?;
            if peer_version < 5 {
                return Err(general_err!(
                    "native current-user CredSSP requires server version 5 or later"
                ));
            }
            let binding = binding_hash(CLIENT_SERVER_HASH_MAGIC, &self.nonce, &self.public_key);
            request.pub_key_auth = Some(self.security.encrypt(&binding, 0)?);
            if peer_version >= 5 {
                request.client_nonce = Some(self.nonce);
            }
            self.state = State::PubKeyAuth;
            debug!("Native CredSSP completed the Negotiate stage");
        }

        Ok(request)
    }

    fn process_pub_key_auth(&mut self, incoming: Option<TsRequest>) -> ConnectorResult<TsRequest> {
        let request = incoming.ok_or_else(|| general_err!("native CredSSP expected the server public-key binding"))?;
        request
            .check_error()
            .map_err(|error| custom_err!("native CredSSP server status", error))?;
        self.observe_peer_version(request.version)?;
        let encrypted_binding = request
            .pub_key_auth
            .ok_or_else(|| general_err!("native CredSSP server omitted its public-key binding"))?;
        let binding = self.security.decrypt(&encrypted_binding, 0)?;
        let expected = binding_hash(SERVER_CLIENT_HASH_MAGIC, &self.nonce, &self.public_key);
        if binding != expected {
            return Err(general_err!("native CredSSP server public-key binding mismatch"));
        }

        let credentials = CredentialsBuffers::AuthIdentity(AuthIdentityBuffers::default());
        let credentials = write_ts_credentials(&credentials, CredSspMode::CredentialLess)
            .map_err(|error| custom_err!("encode credential-less native CredSSP credentials", error))?;
        let auth_info = self.security.encrypt(&credentials, 1)?;
        self.state = State::Finished;
        debug!("Native CredSSP authenticated with current Windows credentials");

        Ok(TsRequest {
            version: TS_REQUEST_VERSION,
            nego_tokens: None,
            auth_info: Some(auth_info),
            pub_key_auth: None,
            error_code: None,
            client_nonce: (request.version >= 5).then_some(self.nonce),
        })
    }

    fn observe_peer_version(&mut self, version: u32) -> ConnectorResult<()> {
        match self.peer_version {
            Some(peer_version) if peer_version != version => {
                Err(general_err!("native CredSSP server changed protocol version"))
            }
            Some(_) => Ok(()),
            None => {
                self.peer_version = Some(version);
                Ok(())
            }
        }
    }
}

struct NativeSecurityContext {
    credentials: SecHandle,
    context: Option<SecHandle>,
    target_name: Vec<u16>,
    sizes: Option<SecPkgContext_Sizes>,
    security_trailer_length: Option<usize>,
}

struct InitializeStep {
    token: Vec<u8>,
    complete: bool,
}

impl NativeSecurityContext {
    fn new(target_name: &str) -> ConnectorResult<Self> {
        let mut credentials = SecHandle::default();
        // SAFETY: null principal/auth data request the current process token; the output handle is writable.
        unsafe {
            AcquireCredentialsHandleW(
                PCWSTR::null(),
                w!("Negotiate"),
                SECPKG_CRED_OUTBOUND,
                None,
                None,
                None,
                None,
                &mut credentials,
                None,
            )
        }
        .map_err(|error| custom_err!("acquire current Windows credentials", error))?;

        Ok(Self {
            credentials,
            context: None,
            target_name: target_name.encode_utf16().chain(core::iter::once(0)).collect(),
            sizes: None,
            security_trailer_length: None,
        })
    }

    fn initialize(&mut self, input_token: &[u8]) -> ConnectorResult<InitializeStep> {
        let has_context = self.context.is_some();
        let mut input = input_token.to_vec();
        let mut input_buffers = [
            SecBuffer {
                cbBuffer: u32::try_from(input.len())
                    .map_err(|_| general_err!("native Negotiate input token is too large"))?,
                BufferType: SECBUFFER_TOKEN,
                pvBuffer: input.as_mut_ptr().cast(),
            },
            SecBuffer {
                cbBuffer: 0,
                BufferType: SECBUFFER_EMPTY,
                pvBuffer: core::ptr::null_mut(),
            },
        ];
        let input_desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: u32::try_from(input_buffers.len()).expect("fixed buffer count fits"),
            pBuffers: input_buffers.as_mut_ptr(),
        };

        let mut output = vec![0u8; MAX_NEGOTIATE_TOKEN_SIZE];
        let mut output_buffer = SecBuffer {
            cbBuffer: u32::try_from(output.len()).expect("fixed token buffer fits"),
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: output.as_mut_ptr().cast(),
        };
        let mut output_desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut output_buffer,
        };
        let mut context_attributes = 0u32;
        let mut new_context = self.context.unwrap_or_default();
        let context_requirements: ISC_REQ_FLAGS =
            ISC_REQ_MUTUAL_AUTH | ISC_REQ_USE_SESSION_KEY | ISC_REQ_INTEGRITY | ISC_REQ_CONFIDENTIALITY;

        // SAFETY: all descriptors point to live buffers for this call and both SSPI handles are valid.
        let status = unsafe {
            InitializeSecurityContextW(
                Some(&self.credentials),
                self.context.as_ref().map(core::ptr::from_ref),
                Some(self.target_name.as_ptr()),
                context_requirements,
                0,
                SECURITY_NATIVE_DREP,
                has_context.then_some(&input_desc),
                0,
                Some(&mut new_context),
                Some(&mut output_desc),
                &mut context_attributes,
                None,
            )
        };
        self.context = Some(new_context);

        let (complete, continue_needed) = match status {
            SEC_E_OK => (true, false),
            SEC_I_CONTINUE_NEEDED => (false, true),
            SEC_I_COMPLETE_NEEDED => {
                self.complete_auth_token(&output_desc)?;
                (true, false)
            }
            SEC_I_COMPLETE_AND_CONTINUE => {
                self.complete_auth_token(&output_desc)?;
                (false, true)
            }
            error => {
                return Err(custom_err!(
                    "initialize current Windows security context",
                    windows::core::Error::from_hresult(error)
                ));
            }
        };

        let output_length = usize::try_from(output_buffer.cbBuffer)
            .map_err(|_| general_err!("native Negotiate output token length does not fit usize"))?;
        if output_length > output.len() {
            return Err(general_err!("native Negotiate returned an oversized output token"));
        }
        output.truncate(output_length);
        if complete {
            let required_attributes = ISC_RET_INTEGRITY | ISC_RET_CONFIDENTIALITY;
            if context_attributes & required_attributes != required_attributes {
                return Err(general_err!(
                    "native Windows security context lacks integrity or confidentiality"
                ));
            }
            self.query_sizes()?;
        }
        trace!(continue_needed, output_length, "Advanced native Negotiate context");
        Ok(InitializeStep {
            token: output,
            complete,
        })
    }

    fn complete_auth_token(&self, output: &SecBufferDesc) -> ConnectorResult<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context is missing"))?;
        // SAFETY: the context and output descriptor are valid for the current SSPI exchange.
        unsafe { CompleteAuthToken(context, output) }
            .map_err(|error| custom_err!("complete current Windows authentication token", error))
    }

    fn query_sizes(&mut self) -> ConnectorResult<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context is missing"))?;
        let mut sizes = SecPkgContext_Sizes::default();
        // SAFETY: `sizes` is the correct output structure for SECPKG_ATTR_SIZES.
        unsafe { QueryContextAttributesW(context, SECPKG_ATTR_SIZES, core::ptr::addr_of_mut!(sizes).cast()) }
            .map_err(|error| custom_err!("query current Windows security context sizes", error))?;
        self.sizes = Some(sizes);
        Ok(())
    }

    fn encrypt(&mut self, plaintext: &[u8], sequence: u32) -> ConnectorResult<Vec<u8>> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context is missing"))?;
        let sizes = self
            .sizes
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context sizes are missing"))?;
        let mut token = vec![
            0u8;
            usize::try_from(sizes.cbSecurityTrailer)
                .map_err(|_| general_err!("native security trailer size does not fit usize"))?
        ];
        let mut data = plaintext.to_vec();
        let mut buffers = [
            SecBuffer {
                cbBuffer: u32::try_from(token.len())
                    .map_err(|_| general_err!("native security trailer is too large"))?,
                BufferType: SECBUFFER_TOKEN,
                pvBuffer: token.as_mut_ptr().cast(),
            },
            SecBuffer {
                cbBuffer: u32::try_from(data.len())
                    .map_err(|_| general_err!("native CredSSP plaintext is too large"))?,
                BufferType: SECBUFFER_DATA,
                pvBuffer: data.as_mut_ptr().cast(),
            },
        ];
        let desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: u32::try_from(buffers.len()).expect("fixed buffer count fits"),
            pBuffers: buffers.as_mut_ptr(),
        };
        // SAFETY: context and mutable buffers are valid for the duration of the call.
        let status = unsafe { EncryptMessage(context, 0, &desc, sequence) };
        status
            .ok()
            .map_err(|error| custom_err!("encrypt native CredSSP message", error))?;

        let token_length = usize::try_from(buffers[0].cbBuffer)
            .map_err(|_| general_err!("native security trailer length does not fit usize"))?;
        if token_length > token.len() {
            return Err(general_err!("native Negotiate returned an oversized security trailer"));
        }
        token.truncate(token_length);
        self.security_trailer_length = Some(token_length);

        data.truncate(
            usize::try_from(buffers[1].cbBuffer)
                .map_err(|_| general_err!("native encrypted data length does not fit usize"))?,
        );
        token.extend_from_slice(&data);
        Ok(token)
    }

    fn decrypt(&self, ciphertext: &[u8], sequence: u32) -> ConnectorResult<Vec<u8>> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context is missing"))?;
        let trailer_length = self
            .security_trailer_length
            .ok_or_else(|| general_err!("native Negotiate security trailer length is missing"))?;
        if ciphertext.len() < trailer_length {
            return Err(general_err!("native CredSSP ciphertext is truncated"));
        }

        let mut ciphertext = ciphertext.to_vec();
        let (token, data) = ciphertext.split_at_mut(trailer_length);
        let mut buffers = [
            SecBuffer {
                cbBuffer: u32::try_from(data.len())
                    .map_err(|_| general_err!("native CredSSP ciphertext is too large"))?,
                BufferType: SECBUFFER_DATA,
                pvBuffer: data.as_mut_ptr().cast(),
            },
            SecBuffer {
                cbBuffer: u32::try_from(token.len())
                    .map_err(|_| general_err!("native security trailer is too large"))?,
                BufferType: SECBUFFER_TOKEN,
                pvBuffer: token.as_mut_ptr().cast(),
            },
        ];
        let desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: u32::try_from(buffers.len()).expect("fixed buffer count fits"),
            pBuffers: buffers.as_mut_ptr(),
        };
        // SAFETY: context and mutable buffers are valid for the duration of the call.
        let status = unsafe { DecryptMessage(context, &desc, sequence, None) };
        status
            .ok()
            .map_err(|error| custom_err!("decrypt native CredSSP message", error))?;

        let data_length = usize::try_from(buffers[0].cbBuffer)
            .map_err(|_| general_err!("native decrypted data length does not fit usize"))?;
        if data_length > data.len() {
            return Err(general_err!("native Negotiate returned oversized decrypted data"));
        }
        Ok(data[..data_length].to_vec())
    }
}

impl Drop for NativeSecurityContext {
    fn drop(&mut self) {
        if let Some(context) = self.context.as_ref() {
            // SAFETY: this object owns the initialized context handle.
            let _ = unsafe { DeleteSecurityContext(context) };
        }
        // SAFETY: this object owns the acquired credential handle.
        let _ = unsafe { FreeCredentialsHandle(&self.credentials) };
    }
}

pub(super) fn binding_hash(magic: &[u8], nonce: &[u8; NONCE_SIZE], public_key: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(magic);
    hasher.update(nonce);
    hasher.update(public_key);
    hasher.finalize().to_vec()
}
