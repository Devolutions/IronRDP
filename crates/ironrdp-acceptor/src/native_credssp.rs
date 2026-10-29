use ironrdp_async::{Framed, FramedRead, FramedWrite};
use ironrdp_connector::sspi::credssp::{NStatusCode, TsRequest, read_ts_credentials};
use ironrdp_connector::sspi::{AuthIdentity, CredentialsBuffers, Secret};
use ironrdp_connector::{ConnectorErrorExt as _, ConnectorResult, custom_err, general_err};
use ironrdp_pdu::PduHint;
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::{SEC_E_OK, SEC_I_COMPLETE_AND_CONTINUE, SEC_I_COMPLETE_NEEDED, SEC_I_CONTINUE_NEEDED};
use windows::Win32::Security::Authentication::Identity::{
    ASC_REQ_CONFIDENTIALITY, ASC_REQ_CONNECTION, ASC_REQ_INTEGRITY, ASC_REQ_REPLAY_DETECT, ASC_REQ_SEQUENCE_DETECT,
    ASC_RET_CONFIDENTIALITY, ASC_RET_INTEGRITY, ASC_RET_NULL_SESSION, AcceptSecurityContext, AcquireCredentialsHandleW,
    CompleteAuthToken, DecryptMessage, DeleteSecurityContext, EncryptMessage, FreeContextBuffer, FreeCredentialsHandle,
    QueryContextAttributesW, SECBUFFER_DATA, SECBUFFER_PADDING, SECBUFFER_STREAM, SECBUFFER_TOKEN, SECBUFFER_VERSION,
    SECPKG_ATTR_NEGOTIATION_INFO, SECPKG_ATTR_SIZES, SECPKG_CRED_INBOUND, SECPKG_NEGOTIATION_COMPLETE,
    SECPKG_NEGOTIATION_DIRECT, SECURITY_NATIVE_DREP, SecBuffer, SecBufferDesc, SecPkgContext_NegotiationInfoW,
    SecPkgContext_Sizes,
};
use windows::Win32::Security::Credentials::SecHandle;
use windows::core::{PCWSTR, w};

const VERSION: u32 = 6;
const MAX_REQUEST_SIZE: usize = 60 * 1024;
const MAX_TOKEN_SIZE: usize = 48 * 1024;
const MAX_SEALED_SIZE: usize = 16 * 1024;
const MAX_ROUNDS: usize = 16;
const CLIENT_BINDING_MAGIC: &[u8] = b"CredSSP Client-To-Server Binding Hash\0";
const SERVER_BINDING_MAGIC: &[u8] = b"CredSSP Server-To-Client Binding Hash\0";

#[derive(Debug)]
struct RequestHint;

impl PduHint for RequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        let size = (|| -> std::io::Result<Option<usize>> {
            if bytes.first().is_some_and(|tag| *tag != 0x30) {
                return Err(invalid_der());
            }
            let Some((header, content)) = der_header(bytes)? else {
                return Ok(None);
            };
            let length = header + content;
            if MAX_REQUEST_SIZE < length || TsRequest::read_length(bytes)? != length {
                return Err(invalid_der());
            }
            Ok(Some(length))
        })()
        .map_err(|error| ironrdp_core::other_err!("native CredSSP framing", source: error))?;
        Ok(size.map(|length| (true, length)))
    }
}

pub(crate) async fn accept<S>(framed: &mut Framed<S>, public_key: &[u8]) -> ConnectorResult<AuthIdentity>
where
    S: FramedRead + FramedWrite,
{
    if public_key.is_empty() || MAX_SEALED_SIZE < public_key.len() {
        return Err(general_err!("native CredSSP requires a bounded TLS public key"));
    }
    let mut security = NativeSecurityContext::new()?;
    let mut peer_version = None;
    let mut nonce = None;
    let mut complete = false;

    for _ in 0..MAX_ROUNDS {
        let request = read_request(framed, &mut peer_version, &mut nonce).await?;
        if request.version < 5 {
            send_failure(framed, request.version, NStatusCode(0xc000_00bb)).await;
            return Err(general_err!("native CredSSP requires protocol version 5 or later"));
        }
        if request.auth_info.is_some() {
            return Err(general_err!("native CredSSP received credentials before TLS binding"));
        }

        let token = if complete {
            if request.nego_tokens.is_some() {
                return Err(general_err!(
                    "native CredSSP received a token after Negotiate completed"
                ));
            }
            Vec::new()
        } else {
            let input = request
                .nego_tokens
                .as_deref()
                .filter(|token| !token.is_empty() && token.len() <= MAX_TOKEN_SIZE)
                .ok_or_else(|| general_err!("native CredSSP requires a bounded Negotiate token"))?;
            match security.accept(input) {
                Ok((token, finished)) => {
                    complete = finished;
                    token
                }
                Err(error) => {
                    send_failure(framed, request.version, NStatusCode::LOGON_FAILURE).await;
                    return Err(error);
                }
            }
        };

        if let Some(binding) = request.pub_key_auth {
            if !complete {
                return Err(general_err!(
                    "native CredSSP received TLS binding before Negotiate completed"
                ));
            }
            let nonce = request
                .client_nonce
                .ok_or_else(|| general_err!("native CredSSP TLS binding omitted the client nonce"))?;
            let plaintext = security.decrypt(binding, 0)?;
            let expected = binding_hash(CLIENT_BINDING_MAGIC, &nonce, public_key);
            if plaintext.as_ref().as_slice() != expected {
                return Err(general_err!("native CredSSP client TLS binding mismatch"));
            }
            // MS-CSSP 3.1.5 omits negoTokens from the server's TLS-binding response.
            let response = TsRequest {
                version: VERSION,
                pub_key_auth: Some(security.encrypt(&binding_hash(SERVER_BINDING_MAGIC, &nonce, public_key))?),
                ..TsRequest::default()
            };
            write_request(framed, &response).await?;

            let mut nonce = Some(nonce);
            let request = read_request(framed, &mut peer_version, &mut nonce).await?;
            if request.nego_tokens.is_some() || request.pub_key_auth.is_some() {
                return Err(general_err!(
                    "native CredSSP received unexpected fields with delegated credentials"
                ));
            }
            let encrypted = request
                .auth_info
                .ok_or_else(|| general_err!("native CredSSP client omitted delegated password credentials"))?;
            // Each direction has its own sequence: client binding 0, credentials 1, server binding 0.
            let plaintext = security.decrypt(encrypted, 1)?;
            validate_password_credentials(plaintext.as_ref())
                .map_err(|error| custom_err!("validate native CredSSP password credentials", error))?;
            let credentials = read_ts_credentials(plaintext.as_ref().as_slice())
                .map_err(|_| general_err!("invalid or unsupported native CredSSP password credentials"))?;
            let CredentialsBuffers::AuthIdentity(credentials) = credentials else {
                return Err(general_err!("native CredSSP requires delegated password credentials"));
            };
            if credentials.user.is_empty()
                || credentials.user.as_slice().contains(&0u16)
                || credentials.domain.as_slice().contains(&0u16)
                || credentials.password.as_ref().as_ref().as_slice().contains(&0u16)
            {
                return Err(general_err!(
                    "native CredSSP credentials contain an empty user or embedded NUL"
                ));
            }
            return AuthIdentity::try_from(credentials)
                .map_err(|_| general_err!("native CredSSP delegated username is invalid"));
        }

        if token.is_empty() {
            return Err(general_err!(
                "native CredSSP Negotiate completed without a client TLS binding"
            ));
        }
        write_request(
            framed,
            &TsRequest {
                version: VERSION,
                nego_tokens: Some(token),
                ..TsRequest::default()
            },
        )
        .await?;
    }
    Err(general_err!("native CredSSP exceeded the Negotiate round limit"))
}

async fn read_request<S>(
    framed: &mut Framed<S>,
    peer_version: &mut Option<u32>,
    nonce: &mut Option<[u8; 32]>,
) -> ConnectorResult<TsRequest>
where
    S: FramedRead,
{
    let bytes = framed
        .read_by_hint(&RequestHint)
        .await
        .map_err(|error| custom_err!("read native CredSSP request", error))?;
    validate_request(&bytes).map_err(|error| custom_err!("validate native CredSSP request", error))?;
    let request =
        TsRequest::from_buffer(&bytes).map_err(|error| custom_err!("decode native CredSSP request", error))?;
    if request.error_code.is_some() {
        return Err(general_err!("native CredSSP client sent a server-only error field"));
    }
    if peer_version.is_some_and(|version| version != request.version) {
        return Err(general_err!("native CredSSP client changed protocol version"));
    }
    *peer_version = Some(request.version);
    if let Some(received) = request.client_nonce {
        if nonce.is_some_and(|nonce| nonce != received) {
            return Err(general_err!("native CredSSP client changed its nonce"));
        }
        *nonce = Some(received);
    }
    Ok(request)
}

async fn write_request<S>(framed: &mut Framed<S>, request: &TsRequest) -> ConnectorResult<()>
where
    S: FramedWrite,
{
    let mut bytes = Vec::new();
    request
        .encode_ts_request(&mut bytes)
        .map_err(|error| custom_err!("encode native CredSSP response", error))?;
    framed
        .write_all(&bytes)
        .await
        .map_err(|error| custom_err!("write native CredSSP response", error))
}

async fn send_failure<S>(framed: &mut Framed<S>, peer_version: u32, status: NStatusCode)
where
    S: FramedWrite,
{
    // MS-CSSP 2.2.1 excludes versions 2 and 5 from errorCode reporting.
    if peer_version < 3 || peer_version == 5 {
        return;
    }
    if let Err(error) = write_request(
        framed,
        &TsRequest {
            version: VERSION,
            error_code: Some(status),
            ..TsRequest::default()
        },
    )
    .await
    {
        tracing::debug!(%error, "Failed to send native CredSSP rejection");
    }
}

struct NativeSecurityContext {
    credentials: SecHandle,
    context: Option<SecHandle>,
    sizes: Option<SecPkgContext_Sizes>,
}

impl NativeSecurityContext {
    fn new() -> ConnectorResult<Self> {
        let mut credentials = SecHandle::default();
        // SAFETY: Null principal and authentication data select the service's inbound Windows credentials.
        unsafe {
            AcquireCredentialsHandleW(
                PCWSTR::null(),
                w!("Negotiate"),
                SECPKG_CRED_INBOUND,
                None,
                None,
                None,
                None,
                &mut credentials,
                None,
            )
        }
        .map_err(|error| custom_err!("acquire inbound Windows Negotiate credentials", error))?;
        Ok(Self {
            credentials,
            context: None,
            sizes: None,
        })
    }

    fn accept(&mut self, input: &[u8]) -> ConnectorResult<(Vec<u8>, bool)> {
        let mut input = input.to_vec();
        let mut input_buffers = [buffer(SECBUFFER_TOKEN, &mut input)?];
        let input_desc = descriptor(&mut input_buffers);
        let mut output = vec![0; MAX_TOKEN_SIZE];
        let mut output_buffers = [buffer(SECBUFFER_TOKEN, &mut output)?];
        let mut output_desc = descriptor(&mut output_buffers);
        let mut attributes = 0;
        let mut context = self.context.unwrap_or(SecHandle {
            dwLower: usize::MAX,
            dwUpper: usize::MAX,
        });
        // SAFETY: Handles and descriptors refer to live, exclusively borrowed storage for this call.
        let status = unsafe {
            AcceptSecurityContext(
                Some(&self.credentials),
                self.context.as_ref().map(core::ptr::from_ref),
                Some(&input_desc),
                ASC_REQ_CONNECTION
                    | ASC_REQ_INTEGRITY
                    | ASC_REQ_CONFIDENTIALITY
                    | ASC_REQ_REPLAY_DETECT
                    | ASC_REQ_SEQUENCE_DETECT,
                SECURITY_NATIVE_DREP,
                Some(&mut context),
                Some(&mut output_desc),
                &mut attributes,
                None,
            )
        };
        if context.dwLower != usize::MAX || context.dwUpper != usize::MAX {
            self.context = Some(context);
        }
        let complete = match status {
            SEC_E_OK => true,
            SEC_I_CONTINUE_NEEDED => false,
            SEC_I_COMPLETE_NEEDED | SEC_I_COMPLETE_AND_CONTINUE => {
                // SAFETY: The owned context and output buffers belong to this authentication exchange.
                unsafe { CompleteAuthToken(self.context()?, &output_desc) }
                    .map_err(|error| custom_err!("complete inbound Windows Negotiate token", error))?;
                status == SEC_I_COMPLETE_NEEDED
            }
            status => {
                return Err(custom_err!(
                    "accept inbound Windows Negotiate token",
                    windows::core::Error::from_hresult(status)
                ));
            }
        };
        let token = buffer_contents(&output, &output_buffers[0], SECBUFFER_TOKEN)?.to_vec();
        if complete {
            let required = ASC_RET_INTEGRITY | ASC_RET_CONFIDENTIALITY;
            if attributes & required != required || attributes & ASC_RET_NULL_SESSION != 0 {
                return Err(general_err!(
                    "native Negotiate requires authenticated integrity and confidentiality"
                ));
            }
            self.validate_package()?;
            let mut sizes = SecPkgContext_Sizes::default();
            // SAFETY: The output structure matches SECPKG_ATTR_SIZES.
            unsafe {
                QueryContextAttributesW(
                    self.context()?,
                    SECPKG_ATTR_SIZES,
                    core::ptr::addr_of_mut!(sizes).cast(),
                )
            }
            .map_err(|error| custom_err!("query inbound Windows Negotiate sizes", error))?;
            let max_overhead = u32::try_from(MAX_SEALED_SIZE).expect("fixed overhead limit fits u32");
            if max_overhead < sizes.cbSecurityTrailer || max_overhead < sizes.cbBlockSize {
                return Err(general_err!("native Negotiate returned excessive message overhead"));
            }
            self.sizes = Some(sizes);
        }
        Ok((token, complete))
    }

    fn validate_package(&self) -> ConnectorResult<()> {
        let mut info = SecPkgContext_NegotiationInfoW::default();
        // SAFETY: The output structure matches SECPKG_ATTR_NEGOTIATION_INFO.
        unsafe {
            QueryContextAttributesW(
                self.context()?,
                SECPKG_ATTR_NEGOTIATION_INFO,
                core::ptr::addr_of_mut!(info).cast(),
            )
        }
        .map_err(|error| custom_err!("query inbound Windows Negotiate package", error))?;
        // SAFETY: SSPI owns and initializes PackageInfo and its null-terminated package name on success.
        let package = unsafe { info.PackageInfo.as_ref() }
            .ok_or_else(|| general_err!("native Negotiate omitted the selected package"))?;
        // SAFETY: The package name remains allocated until FreeContextBuffer below.
        let name = unsafe { PCWSTR(package.Name.cast_const()).to_string() };
        // SAFETY: QueryContextAttributesW allocated this buffer for the caller to release.
        let _ = unsafe { FreeContextBuffer(info.PackageInfo.cast()) };
        let name = name.map_err(|error| custom_err!("read inbound Windows Negotiate package name", error))?;
        if !(matches!(
            info.NegotiationState,
            SECPKG_NEGOTIATION_COMPLETE | SECPKG_NEGOTIATION_DIRECT
        ) && (name.eq_ignore_ascii_case("NTLM") || name.eq_ignore_ascii_case("Kerberos")))
        {
            return Err(general_err!(
                "native CredSSP requires completed NTLM or Kerberos authentication"
            ));
        }
        Ok(())
    }

    fn context(&self) -> ConnectorResult<&SecHandle> {
        self.context
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context is missing"))
    }

    fn encrypt(&self, plaintext: &[u8]) -> ConnectorResult<Vec<u8>> {
        let sizes = self
            .sizes
            .as_ref()
            .ok_or_else(|| general_err!("native Negotiate context is incomplete"))?;
        let mut token = vec![
            0;
            usize::try_from(sizes.cbSecurityTrailer)
                .map_err(|_| general_err!("native Negotiate trailer size does not fit usize"))?
        ];
        let mut data = plaintext.to_vec();
        let mut padding = vec![
            0;
            usize::try_from(sizes.cbBlockSize)
                .map_err(|_| general_err!("native Negotiate padding size does not fit usize"))?
        ];
        let mut buffers = [
            buffer(SECBUFFER_TOKEN, &mut token)?,
            buffer(SECBUFFER_DATA, &mut data)?,
            buffer(SECBUFFER_PADDING, &mut padding)?,
        ];
        let desc = descriptor(&mut buffers);
        // SAFETY: The established context and writable buffers remain valid throughout the call.
        let status = unsafe { EncryptMessage(self.context()?, 0, &desc, 0) };
        if status != SEC_E_OK {
            return Err(custom_err!(
                "encrypt native CredSSP TLS binding",
                windows::core::Error::from_hresult(status)
            ));
        }
        let mut result = buffer_contents(&token, &buffers[0], SECBUFFER_TOKEN)?.to_vec();
        result.extend_from_slice(buffer_contents(&data, &buffers[1], SECBUFFER_DATA)?);
        result.extend_from_slice(buffer_contents(&padding, &buffers[2], SECBUFFER_PADDING)?);
        if MAX_SEALED_SIZE < result.len() {
            return Err(general_err!("native CredSSP encrypted binding exceeds its size limit"));
        }
        Ok(result)
    }

    fn decrypt(&self, ciphertext: Vec<u8>, sequence: u32) -> ConnectorResult<Secret<Vec<u8>>> {
        if self.sizes.is_none() || ciphertext.is_empty() || MAX_SEALED_SIZE < ciphertext.len() {
            return Err(general_err!(
                "native CredSSP requires a complete context and bounded ciphertext"
            ));
        }
        let mut ciphertext = Secret::new(ciphertext);
        // STREAM lets NTLM/Kerberos locate their own variable-length trailer without guessing a split.
        // UNTESTED: Kerberos wrapping must interoperate with a domain client; verify it when changing these buffers.
        let mut buffers = [
            buffer(SECBUFFER_STREAM, ciphertext.as_mut())?,
            SecBuffer {
                BufferType: SECBUFFER_DATA,
                ..SecBuffer::default()
            },
        ];
        let desc = descriptor(&mut buffers);
        // NTLM can leave pfQOP untouched; the negotiated confidentiality attribute is checked above.
        let mut quality = 0;
        // SAFETY: SSPI decrypts in the owned input allocation and returns a view through the DATA buffer.
        let status = unsafe { DecryptMessage(self.context()?, &desc, sequence, Some(&mut quality)) };
        if status != SEC_E_OK {
            return Err(custom_err!(
                "decrypt native CredSSP message",
                windows::core::Error::from_hresult(status)
            ));
        }
        if quality != 0 {
            return Err(ironrdp_connector::ConnectorError::reason(
                "native CredSSP requires encrypted messages",
                format!("SSPI reported quality of protection {quality:#010x}"),
            ));
        }
        let plaintext = buffer_contents(ciphertext.as_ref(), &buffers[1], SECBUFFER_DATA)?;
        Ok(Secret::new(plaintext.to_vec()))
    }
}

impl Drop for NativeSecurityContext {
    fn drop(&mut self) {
        if let Some(context) = self.context.as_ref() {
            // SAFETY: This object owns the context, including partially established contexts.
            let _ = unsafe { DeleteSecurityContext(context) };
        }
        // SAFETY: This object owns the successfully acquired credential handle.
        let _ = unsafe { FreeCredentialsHandle(&self.credentials) };
    }
}

fn buffer(kind: u32, bytes: &mut [u8]) -> ConnectorResult<SecBuffer> {
    Ok(SecBuffer {
        BufferType: kind,
        cbBuffer: u32::try_from(bytes.len()).map_err(|_| general_err!("native Negotiate buffer is too large"))?,
        pvBuffer: bytes.as_mut_ptr().cast(),
    })
}

fn descriptor(buffers: &mut [SecBuffer]) -> SecBufferDesc {
    SecBufferDesc {
        ulVersion: SECBUFFER_VERSION,
        cBuffers: u32::try_from(buffers.len()).expect("fixed SSPI buffer count fits u32"),
        pBuffers: buffers.as_mut_ptr(),
    }
}

fn buffer_contents<'a>(storage: &'a [u8], buffer: &SecBuffer, kind: u32) -> ConnectorResult<&'a [u8]> {
    if buffer.BufferType != kind {
        return Err(general_err!("native Negotiate returned an unexpected buffer type"));
    }
    if buffer.cbBuffer == 0 {
        return Ok(&[]);
    }
    let offset = buffer
        .pvBuffer
        .addr()
        .checked_sub(storage.as_ptr().addr())
        .ok_or_else(|| general_err!("native Negotiate returned a pointer outside its buffer"))?;
    let end = offset
        .checked_add(
            usize::try_from(buffer.cbBuffer)
                .map_err(|_| general_err!("native Negotiate buffer length does not fit usize"))?,
        )
        .ok_or_else(|| general_err!("native Negotiate returned an overflowing buffer range"))?;
    storage
        .get(offset..end)
        .ok_or_else(|| general_err!("native Negotiate returned a range outside its buffer"))
}

fn binding_hash(magic: &[u8], nonce: &[u8; 32], public_key: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(magic);
    hash.update(nonce);
    hash.update(public_key);
    hash.finalize().into()
}

// The shared decoders do not enforce every enclosing DER length or reject trailing fields.
fn validate_request(bytes: &[u8]) -> std::io::Result<()> {
    let mut fields = exact_der(bytes, 0x30)?;
    let version = exact_der(take_der(&mut fields, 0xa0)?, 0x02)?;
    if version.is_empty() || version[0] & 0x80 != 0 || (1 < version.len() && version[0] == 0 && version[1] & 0x80 == 0)
    {
        return Err(invalid_der());
    }
    for tag in 0xa1..=0xa5 {
        if fields.first() != Some(&tag) {
            continue;
        }
        let field = take_der(&mut fields, tag)?;
        if tag == 0xa1 {
            let token = exact_der(exact_der(exact_der(exact_der(field, 0x30)?, 0x30)?, 0xa0)?, 0x04)?;
            if token.is_empty() || MAX_TOKEN_SIZE < token.len() {
                return Err(invalid_der());
            }
        } else {
            let value = exact_der(field, if tag == 0xa4 { 0x02 } else { 0x04 })?;
            if MAX_SEALED_SIZE < value.len() || (tag == 0xa5 && value.len() != 32) {
                return Err(invalid_der());
            }
        }
    }
    if !fields.is_empty() {
        return Err(invalid_der());
    }
    Ok(())
}

fn validate_password_credentials(bytes: &[u8]) -> std::io::Result<()> {
    let mut fields = exact_der(bytes, 0x30)?;
    if exact_der(take_der(&mut fields, 0xa0)?, 0x02)? != [1] {
        return Err(std::io::Error::other(
            "native CredSSP supports only password credentials",
        ));
    }
    let mut password = exact_der(exact_der(take_der(&mut fields, 0xa1)?, 0x04)?, 0x30)?;
    for tag in 0xa0..=0xa2 {
        let _ = exact_der(take_der(&mut password, tag)?, 0x04)?;
    }
    if !fields.is_empty() || !password.is_empty() {
        return Err(invalid_der());
    }
    Ok(())
}

fn exact_der(bytes: &[u8], tag: u8) -> std::io::Result<&[u8]> {
    let mut bytes = bytes;
    let value = take_der(&mut bytes, tag)?;
    if !bytes.is_empty() {
        return Err(invalid_der());
    }
    Ok(value)
}

fn take_der<'a>(bytes: &mut &'a [u8], tag: u8) -> std::io::Result<&'a [u8]> {
    if bytes.first() != Some(&tag) {
        return Err(invalid_der());
    }
    let (header, content) = der_header(bytes)?.ok_or_else(invalid_der)?;
    let value = bytes.get(header..header + content).ok_or_else(invalid_der)?;
    *bytes = &bytes[header + content..];
    Ok(value)
}

fn der_header(bytes: &[u8]) -> std::io::Result<Option<(usize, usize)>> {
    let Some(&length) = bytes.get(1) else {
        return Ok(None);
    };
    let (header, length) = match length {
        0..=0x7f => (2, usize::from(length)),
        0x81 => {
            let Some(&length) = bytes.get(2) else {
                return Ok(None);
            };
            if length < 0x80 {
                return Err(invalid_der());
            }
            (3, usize::from(length))
        }
        0x82 => {
            let Some(length) = bytes.get(2..4) else {
                return Ok(None);
            };
            if length[0] == 0 {
                return Err(invalid_der());
            }
            (4, usize::from(u16::from_be_bytes([length[0], length[1]])))
        }
        _ => return Err(invalid_der()),
    };
    if MAX_REQUEST_SIZE < header + length {
        return Err(invalid_der());
    }
    Ok(Some((header, length)))
}

fn invalid_der() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid or oversized native CredSSP DER",
    )
}
