use sspi::{Username, UsernameParts};
use windows::Win32::Foundation::{SEC_E_OK, SEC_I_COMPLETE_AND_CONTINUE, SEC_I_COMPLETE_NEEDED, SEC_I_CONTINUE_NEEDED};
use windows::Win32::Security::Authentication::Identity::{
    AcquireCredentialsHandleW, CompleteAuthToken, DeleteSecurityContext, FreeCredentialsHandle, ISC_REQ_CONNECTION,
    ISC_REQ_FLAGS, ISC_REQ_INTEGRITY, ISC_REQ_MUTUAL_AUTH, ISC_RET_INTEGRITY, InitializeSecurityContextW,
    SECBUFFER_CHANNEL_BINDINGS, SECBUFFER_TOKEN, SECBUFFER_VERSION, SECPKG_CRED_OUTBOUND, SECURITY_NETWORK_DREP,
    SecBuffer, SecBufferDesc, SspiEncodeStringsAsAuthIdentity, SspiFreeAuthIdentity,
};
use windows::Win32::Security::Credentials::SecHandle;
use windows::core::PCWSTR;

use crate::{Error, GwErrorExt as _, GwErrorKind};

const MAX_NEGOTIATE_TOKEN_SIZE: usize = 64 * 1024;

pub(super) struct NativeHttpAuth {
    credentials: SecHandle,
    context: Option<SecHandle>,
    target_name: Vec<u16>,
    channel_binding: Vec<u8>,
    request_mutual_auth: bool,
}

pub(super) struct InitializeStep {
    pub(super) token: Vec<u8>,
    pub(super) complete: bool,
}

impl NativeHttpAuth {
    pub(super) fn new(
        username: &str,
        password: &str,
        target_name: &str,
        channel_binding: &[u8],
        package: &str,
    ) -> Result<Self, Error> {
        let credentials_buffers = CredentialsBuffers::new(username, password)?;
        let identity = NativeAuthIdentity::new(&credentials_buffers)?;
        let package_name: Vec<u16> = package.encode_utf16().chain(core::iter::once(0)).collect();
        let mut credentials = SecHandle::default();

        // SAFETY: `identity` and `package_name` remain valid for the call and `credentials` is writable.
        unsafe {
            AcquireCredentialsHandleW(
                PCWSTR::null(),
                PCWSTR::from_raw(package_name.as_ptr()),
                SECPKG_CRED_OUTBOUND,
                None,
                Some(identity.as_ptr()),
                None,
                None,
                &mut credentials,
                None,
            )
        }
        .map_err(|error| Error::custom("acquire native Windows HTTP credentials", error))?;

        Ok(Self {
            credentials,
            context: None,
            target_name: target_name.encode_utf16().chain(core::iter::once(0)).collect(),
            channel_binding: channel_binding.to_owned(),
            request_mutual_auth: package == "Negotiate",
        })
    }

    pub(super) fn initialize(&mut self, input_token: Option<&[u8]>) -> Result<InitializeStep, Error> {
        let has_context = self.context.is_some();
        let mut input_token = input_token.unwrap_or_default().to_vec();
        let mut input_buffers = Vec::with_capacity(2);
        if has_context || !input_token.is_empty() {
            input_buffers.push(SecBuffer {
                cbBuffer: u32::try_from(input_token.len())
                    .map_err(|_| Error::new("native HTTP auth input token is too large", GwErrorKind::Connect))?,
                BufferType: SECBUFFER_TOKEN,
                pvBuffer: input_token.as_mut_ptr().cast(),
            });
        }
        input_buffers.push(SecBuffer {
            cbBuffer: u32::try_from(self.channel_binding.len())
                .map_err(|_| Error::new("native HTTP auth channel binding is too large", GwErrorKind::Connect))?,
            BufferType: SECBUFFER_CHANNEL_BINDINGS,
            pvBuffer: self.channel_binding.as_mut_ptr().cast(),
        });
        let input_desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: u32::try_from(input_buffers.len()).expect("fixed input buffer count fits in u32"),
            pBuffers: input_buffers.as_mut_ptr(),
        };

        let mut output_token = vec![0; MAX_NEGOTIATE_TOKEN_SIZE];
        let mut output_buffer = SecBuffer {
            cbBuffer: u32::try_from(output_token.len()).expect("fixed token buffer fits in u32"),
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: output_token.as_mut_ptr().cast(),
        };
        let mut output_desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut output_buffer,
        };
        let mut context_attributes = 0u32;
        let mut new_context = self.context.unwrap_or_default();
        let mut context_requirements: ISC_REQ_FLAGS = ISC_REQ_CONNECTION | ISC_REQ_INTEGRITY;
        if self.request_mutual_auth {
            context_requirements |= ISC_REQ_MUTUAL_AUTH;
        }

        // SAFETY: descriptors and strings reference live data for the call, and the credential handle is valid.
        let status = unsafe {
            InitializeSecurityContextW(
                Some(&self.credentials),
                self.context.as_ref().map(core::ptr::from_ref),
                Some(self.target_name.as_ptr()),
                context_requirements,
                0,
                SECURITY_NETWORK_DREP,
                (!input_buffers.is_empty()).then_some(&input_desc),
                0,
                Some(&mut new_context),
                Some(&mut output_desc),
                &mut context_attributes,
                None,
            )
        };
        self.context = Some(new_context);

        let complete = match status {
            SEC_E_OK => true,
            SEC_I_CONTINUE_NEEDED => false,
            SEC_I_COMPLETE_NEEDED => {
                self.complete_auth_token(&output_desc)?;
                true
            }
            SEC_I_COMPLETE_AND_CONTINUE => {
                self.complete_auth_token(&output_desc)?;
                false
            }
            error => {
                return Err(Error::custom(
                    "initialize native Windows HTTP security context",
                    windows::core::Error::from_hresult(error),
                ));
            }
        };

        let output_length = usize::try_from(output_buffer.cbBuffer).map_err(|_| {
            Error::new(
                "native HTTP auth output token length does not fit usize",
                GwErrorKind::Connect,
            )
        })?;
        if output_length > output_token.len() {
            return Err(Error::new(
                "native Windows HTTP authentication returned an oversized output token",
                GwErrorKind::Connect,
            ));
        }
        output_token.truncate(output_length);

        if complete && context_attributes & ISC_RET_INTEGRITY == 0 {
            return Err(Error::new(
                "native Windows HTTP security context lacks integrity",
                GwErrorKind::Connect,
            ));
        }

        Ok(InitializeStep {
            token: output_token,
            complete,
        })
    }

    fn complete_auth_token(&self, output: &SecBufferDesc) -> Result<(), Error> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| Error::new("native Windows HTTP security context is missing", GwErrorKind::Connect))?;

        // SAFETY: this context and the output descriptor are valid for the current SSPI exchange.
        unsafe { CompleteAuthToken(context, output) }
            .map_err(|error| Error::custom("complete native Windows HTTP authentication token", error))
    }
}

impl Drop for NativeHttpAuth {
    fn drop(&mut self) {
        if let Some(context) = self.context.as_ref() {
            // SAFETY: this object owns the initialized security context.
            let _ = unsafe { DeleteSecurityContext(context) };
        }
        // SAFETY: this object owns the acquired credential handle.
        let _ = unsafe { FreeCredentialsHandle(&self.credentials) };
    }
}

struct CredentialsBuffers {
    username: Vec<u16>,
    domain: Vec<u16>,
    password: Vec<u16>,
}

impl CredentialsBuffers {
    fn new(username: &str, password: &str) -> Result<Self, Error> {
        let username = Username::parse(username)
            .or_else(|_| Username::new(username, None))
            .map_err(|error| Error::custom("parse gateway username", error))?;
        let (username, domain) = match username.parts() {
            UsernameParts::UserPrincipalName(parts) => (parts.upn(), None),
            UsernameParts::DownLevelLogonName(parts) => (parts.account_name(), parts.netbios_domain()),
        };

        Ok(Self {
            username: username.encode_utf16().chain(core::iter::once(0)).collect(),
            domain: domain
                .unwrap_or_default()
                .encode_utf16()
                .chain(core::iter::once(0))
                .collect(),
            password: password.encode_utf16().chain(core::iter::once(0)).collect(),
        })
    }
}

impl Drop for CredentialsBuffers {
    fn drop(&mut self) {
        for character in &mut self.password {
            // SAFETY: each mutable reference points to an initialized password code unit.
            unsafe { core::ptr::write_volatile(character, 0) };
        }
    }
}

struct NativeAuthIdentity {
    value: *mut core::ffi::c_void,
}

impl NativeAuthIdentity {
    fn new(credentials: &CredentialsBuffers) -> Result<Self, Error> {
        let mut value = core::ptr::null_mut();

        // SAFETY: each credential string is NUL-terminated and remains valid for the call, and `value` is writable.
        let result = unsafe {
            SspiEncodeStringsAsAuthIdentity(
                PCWSTR::from_raw(credentials.username.as_ptr()),
                PCWSTR::from_raw(credentials.domain.as_ptr()),
                PCWSTR::from_raw(credentials.password.as_ptr()),
                &mut value,
            )
        };
        if let Err(error) = result {
            if !value.is_null() {
                // SAFETY: SSPI allocated this identity before returning an error.
                unsafe { SspiFreeAuthIdentity(Some(value.cast_const())) };
            }
            return Err(Error::custom("encode native Windows HTTP credentials", error));
        }

        if value.is_null() {
            return Err(Error::new(
                "encode native Windows HTTP credentials returned no identity",
                GwErrorKind::Connect,
            ));
        }

        Ok(Self { value })
    }

    fn as_ptr(&self) -> *const core::ffi::c_void {
        self.value.cast_const()
    }
}

impl Drop for NativeAuthIdentity {
    fn drop(&mut self) {
        // SAFETY: this object owns the identity allocated by `SspiEncodeStringsAsAuthIdentity`.
        unsafe { SspiFreeAuthIdentity(Some(self.value.cast_const())) };
    }
}
