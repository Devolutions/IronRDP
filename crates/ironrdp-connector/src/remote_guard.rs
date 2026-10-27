use ironrdp_core::{Encode as _, ReadCursor, encode_vec};
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpear::ndr::{self, KerberosRequest as Request, KerberosResponseCall as Response};
use ironrdp_rdpear::{AuthRedirectionBackend, AuthRedirectionClient, RemoteGuardInnerPacket};
use sspi::rcg as rg;
use sspi::{Error, ErrorKind, Secret};
use tracing::warn;

use crate::{ConnectorResult, custom_err, general_err};

pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
pub const WRAP_OVERHEAD: usize = 16 /* Header */ + 16 /* Confounder */ + 16 /* EncryptedHeader */ + 12 /* Checksum */;
pub const KERBEROS: &[u8] = b"K\0e\0r\0b\0e\0r\0o\0s\0";
pub const NTLM: &[u8] = b"N\0T\0L\0M\0";
pub const PROFILE: ndr::NdrProfile = ndr::NdrProfile::WindowsNative;
pub const STATUS_NOT_SUPPORTED: i32 = 0xc000_00bbu32.cast_signed();
pub const STATUS_INVALID_PARAMETER: i32 = 0xc000_000du32.cast_signed();
pub const STATUS_INVALID_HANDLE: i32 = 0xc000_0008u32.cast_signed();
pub const STATUS_ACCESS_DENIED: i32 = 0xc000_0022u32.cast_signed();
pub const STATUS_UNSUCCESSFUL: i32 = 0xc000_0001u32.cast_signed();

pub(crate) fn channel(session: rg::RemoteGuardSession, allow_kerberos: bool) -> ConnectorResult<AuthRedirectionClient> {
    AuthRedirectionClient::new(
        Box::new(Backend {
            context: Some(session),
            negotiated: false,
            ntlm_negotiated: false,
            kerberos_allowed: allow_kerberos,
        }),
        u32::try_from(MAX_MESSAGE_SIZE).map_err(|_| general_err!("RCG message limit exceeds u32"))?,
    )
    .map_err(|e| custom_err!("create RCG channel", e))
}

// Keeps protocol policy testable without manufacturing a live authenticated SSPI context.
pub trait Context: Send {
    fn unwrap(&mut self, token: &[u8]) -> sspi::Result<Secret<Vec<u8>>>;
    fn wrap(&mut self, plaintext: &[u8]) -> sspi::Result<Vec<u8>>;
    fn execute(&mut self, request: &Request) -> sspi::Result<Response>;
    fn ntlm_available(&self) -> bool;
    fn execute_ntlm(&mut self, request: &ndr::NtlmRequest) -> sspi::Result<ndr::NtlmResponseCall>;
}

pub struct Backend<C> {
    pub context: Option<C>,
    pub negotiated: bool,
    pub ntlm_negotiated: bool,
    pub kerberos_allowed: bool,
}

impl<C: Context> AuthRedirectionBackend for Backend<C> {
    fn process(&mut self, encrypted_payload: &[u8], max_payload_size: usize) -> PduResult<Vec<u8>> {
        let result = self.process_inner(encrypted_payload, max_payload_size);
        if result.is_err() {
            self.close();
        }
        result
    }

    fn close(&mut self) {
        self.context = None;
        self.negotiated = false;
        self.ntlm_negotiated = false;
    }
}

impl<C: Context> Backend<C> {
    fn process_inner(&mut self, encrypted_payload: &[u8], max_payload_size: usize) -> PduResult<Vec<u8>> {
        let limit = max_payload_size.min(MAX_MESSAGE_SIZE - ironrdp_rdpear::AuthRedirectionPdu::HEADER_SIZE);
        if encrypted_payload.len() > limit || limit < WRAP_OVERHEAD {
            return Err(pdu_other_err!("RCG payload exceeds the message limit"));
        }
        let context = self
            .context
            .as_mut()
            .ok_or_else(|| pdu_other_err!("RCG context is closed"))?;
        let plaintext = context
            .unwrap(encrypted_payload)
            .map_err(|_| pdu_other_err!("RCG message authentication failed"))?;
        let inner = RemoteGuardInnerPacket::decode_with_limit(&mut ReadCursor::new(plaintext.as_ref()), limit)
            .map_err(|_| pdu_other_err!("invalid authenticated RCG inner packet"))?;
        let limits = ndr::NdrLimits {
            max_message_size: limit,
            ..ndr::NdrLimits::default()
        };
        let response = if inner.package_name == KERBEROS {
            let request = Request::decode_with_profile(inner.package_call, limits, PROFILE)
                .map_err(|_| pdu_other_err!("invalid RCG Kerberos request"))?;
            let result = if !self.kerberos_allowed {
                Err(Error::new(
                    ErrorKind::UnsupportedFunction,
                    "RCG Kerberos redirection is disabled",
                ))
            } else {
                match &request {
                    Request::NegotiateVersion(version) => {
                        if version.version != 0 {
                            Err(Error::new(
                                ErrorKind::UnsupportedFunction,
                                "unsupported RCG package version",
                            ))
                        } else {
                            self.negotiated = true;
                            Ok(Response::NegotiateVersion(ndr::NegotiateVersion { version: 0 }))
                        }
                    }
                    _ if !self.negotiated => Err(Error::new(
                        ErrorKind::OutOfSequence,
                        "RCG package version not negotiated",
                    )),
                    _ => context.execute(&request),
                }
            };
            let (status, call) = match result {
                Ok(call) => (0, call),
                Err(error) => {
                    warn!(call_id = request.call_id(), kind = ?error.error_type, "RCG operation rejected");
                    (operation_status(error.error_type), empty_response(&request))
                }
            };
            ndr::KerberosResponse { status, call }.encode_with_profile(ndr::Endianness::Little, limits, PROFILE)
        } else if inner.package_name == NTLM {
            let request = ndr::NtlmRequest::decode_with_profile(inner.package_call, limits, PROFILE)
                .map_err(|_| pdu_other_err!("invalid RCG NTLM request"))?;
            let result = if !context.ntlm_available() {
                Err(Error::new(
                    ErrorKind::UnsupportedFunction,
                    "RCG NTLM credentials are unavailable",
                ))
            } else {
                match &request {
                    ndr::NtlmRequest::NegotiateVersion(version) => {
                        if version.version != 0 {
                            Err(Error::new(
                                ErrorKind::UnsupportedFunction,
                                "unsupported RCG NTLM package version",
                            ))
                        } else {
                            self.ntlm_negotiated = true;
                            Ok(ndr::NtlmResponseCall::NegotiateVersion(ndr::NegotiateVersion {
                                version: 0,
                            }))
                        }
                    }
                    _ if !self.ntlm_negotiated => Err(Error::new(
                        ErrorKind::OutOfSequence,
                        "RCG NTLM package version not negotiated",
                    )),
                    ndr::NtlmRequest::CalculateNtResponse(_) | ndr::NtlmRequest::CalculateUserSessionKeyNt(_) => Err(
                        Error::new(ErrorKind::UnsupportedFunction, "RCG NTLMv1 operations are disabled"),
                    ),
                    _ => context.execute_ntlm(&request),
                }
            };
            let (status, call) = match result {
                Ok(call) => (0, call),
                Err(error) => {
                    warn!(call_id = request.call_id(), kind = ?error.error_type, "RCG NTLM operation rejected");
                    (operation_status(error.error_type), empty_ntlm_response(&request))
                }
            };
            ndr::NtlmResponse { status, call }.encode_with_profile(ndr::Endianness::Little, limits, PROFILE)
        } else {
            return Err(pdu_other_err!("unauthorized RCG security package"));
        }
        .map_err(|_| pdu_other_err!("RCG response encoding failed"))?;
        let response = Secret::new(response);
        let inner = RemoteGuardInnerPacket {
            package_name: inner.package_name,
            package_call: response.as_ref(),
        };
        if inner.size() > limit - WRAP_OVERHEAD {
            return Err(pdu_other_err!("RCG response exceeds the message limit"));
        }
        let encoded = Secret::new(encode_vec(&inner).map_err(|_| pdu_other_err!("RCG inner encoding failed"))?);
        let sealed = context
            .wrap(encoded.as_ref())
            .map_err(|_| pdu_other_err!("RCG response protection failed"))?;
        if sealed.is_empty() || sealed.len() > limit {
            return Err(pdu_other_err!("RCG protected response exceeds the message limit"));
        }
        Ok(sealed)
    }
}

fn operation_status(kind: ErrorKind) -> i32 {
    match kind {
        ErrorKind::UnsupportedFunction => STATUS_NOT_SUPPORTED,
        ErrorKind::InvalidParameter | ErrorKind::InvalidToken | ErrorKind::OutOfSequence => STATUS_INVALID_PARAMETER,
        ErrorKind::InvalidHandle => STATUS_INVALID_HANDLE,
        ErrorKind::NoCredentials | ErrorKind::LogonDenied => STATUS_ACCESS_DENIED,
        _ => STATUS_UNSUCCESSFUL,
    }
}

fn empty_response(request: &Request) -> Response {
    match request {
        Request::NegotiateVersion(_) => Response::NegotiateVersion(Default::default()),
        Request::BuildAsReqAuthenticator(_) => Response::BuildAsReqAuthenticator(Default::default()),
        Request::VerifyServiceTicket(_) => Response::VerifyServiceTicket(Default::default()),
        Request::CreateApReqAuthenticator(_) => Response::CreateApReqAuthenticator(Default::default()),
        Request::DecryptApReply(_) => Response::DecryptApReply(Default::default()),
        Request::UnpackKdcReplyBody(_) => Response::UnpackKdcReplyBody(Default::default()),
        Request::ComputeTgsChecksum(_) => Response::ComputeTgsChecksum(Default::default()),
        Request::BuildEncryptedAuthData(_) => Response::BuildEncryptedAuthData(Default::default()),
        Request::PackApReply(_) => Response::PackApReply(Default::default()),
        Request::HashS4UPreauth(_) => Response::HashS4UPreauth(Default::default()),
        Request::SignS4UPreauthData(_) => Response::SignS4UPreauthData(Default::default()),
        Request::VerifyChecksum(_) => Response::VerifyChecksum(Default::default()),
        Request::DecryptPacCredentials(_) => Response::DecryptPacCredentials(Default::default()),
        Request::CreateEcdhKeyAgreement(_) => Response::CreateEcdhKeyAgreement(Default::default()),
        Request::CreateDhKeyAgreement(_) => Response::CreateDhKeyAgreement(Default::default()),
        Request::DestroyKeyAgreement(_) => Response::DestroyKeyAgreement(Default::default()),
        Request::KeyAgreementGenerateNonce(_) => Response::KeyAgreementGenerateNonce(Default::default()),
        Request::FinalizeKeyAgreement(_) => Response::FinalizeKeyAgreement(Default::default()),
    }
}

pub fn empty_ntlm_response(request: &ndr::NtlmRequest) -> ndr::NtlmResponseCall {
    match request {
        ndr::NtlmRequest::NegotiateVersion(_) => ndr::NtlmResponseCall::NegotiateVersion(Default::default()),
        ndr::NtlmRequest::Lm20GetNtlm3ChallengeResponse(_) => {
            ndr::NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(Default::default())
        }
        ndr::NtlmRequest::CalculateNtResponse(_) => ndr::NtlmResponseCall::CalculateNtResponse(Default::default()),
        ndr::NtlmRequest::CalculateUserSessionKeyNt(_) => {
            ndr::NtlmResponseCall::CalculateUserSessionKeyNt(Default::default())
        }
        ndr::NtlmRequest::CompareCredentials(_) => ndr::NtlmResponseCall::CompareCredentials(Default::default()),
    }
}

fn invalid_parameter() -> Error {
    Error::new(ErrorKind::InvalidParameter, "invalid RCG operation argument")
}

fn required<T>(value: &Option<T>) -> sspi::Result<&T> {
    value.as_ref().ok_or_else(invalid_parameter)
}

pub fn bytes(value: &ndr::OctetString) -> sspi::Result<&[u8]> {
    let bytes = required(&value.value)?;
    if usize::try_from(value.length).ok() != Some(bytes.len()) {
        return Err(invalid_parameter());
    }
    Ok(bytes)
}

fn asn1(value: &ndr::Asn1Data) -> sspi::Result<&[u8]> {
    // Pdu is reserved and ignored as specified by MS-RDPEAR 2.2.1.2.4.
    bytes(&value.data)
}

pub fn key(value: &ndr::EncryptionKey) -> sspi::Result<rg::KeyReference<'_>> {
    Ok(rg::KeyReference {
        encryption_type: value.reserved2,
        value: bytes(&value.reserved3)?,
    })
}

pub fn unicode(value: &ndr::UnicodeString) -> sspi::Result<String> {
    let buffer = required(&value.buffer)?;
    if usize::from(value.length) != buffer.len() * 2 {
        return Err(invalid_parameter());
    }
    String::from_utf16(buffer).map_err(|_| invalid_parameter())
}

pub fn unicode_bytes(value: &ndr::UnicodeString) -> sspi::Result<Vec<u8>> {
    let buffer = required(&value.buffer)?;
    if usize::from(value.length) != buffer.len() * 2 {
        return Err(invalid_parameter());
    }
    Ok(buffer.iter().flat_map(|unit| unit.to_le_bytes()).collect())
}

pub fn ntlm_credential(value: &ndr::EncryptedSecrets) -> sspi::Result<rg::NtlmCredentialReference<'_>> {
    Ok(rg::NtlmCredentialReference {
        reserved1: value.reserved1,
        reserved2: value.reserved2,
        reserved3: value.reserved3,
        credential_key_type: match value.reserved4 {
            ndr::CredentialKeyType::Invalid => 0,
            ndr::CredentialKeyType::Ium => 1,
            ndr::CredentialKeyType::DomainUser => 2,
            ndr::CredentialKeyType::LocalUser => 3,
            ndr::CredentialKeyType::ExternallySupplied => 4,
        },
        credential_key: &value.reserved5.0,
        encrypted_secrets: bytes(&value.reserved6)?,
    })
}

pub fn output(bytes: Vec<u8>) -> sspi::Result<ndr::OctetString> {
    Ok(ndr::OctetString {
        length: u32::try_from(bytes.len()).map_err(|_| invalid_parameter())?,
        value: Some(bytes),
    })
}

pub fn asn1_output(bytes: Vec<u8>) -> sspi::Result<ndr::Asn1Data> {
    Ok(ndr::Asn1Data {
        pdu: 0,
        data: output(bytes)?,
    })
}

impl Context for rg::RemoteGuardSession {
    fn unwrap(&mut self, token: &[u8]) -> sspi::Result<Secret<Vec<u8>>> {
        Self::unwrap(self, token)
    }

    fn wrap(&mut self, plaintext: &[u8]) -> sspi::Result<Vec<u8>> {
        Self::wrap(self, plaintext)
    }

    fn ntlm_available(&self) -> bool {
        Self::ntlm_available(self)
    }

    fn execute_ntlm(&mut self, request: &ndr::NtlmRequest) -> sspi::Result<ndr::NtlmResponseCall> {
        match request {
            ndr::NtlmRequest::Lm20GetNtlm3ChallengeResponse(request) => {
                let result = self.lm20_get_ntlm3_challenge_response(&rg::Lm20GetNtlm3ChallengeResponseRequest {
                    credential: ntlm_credential(required(&request.credential)?)?,
                    user_name: &unicode_bytes(required(&request.user_name)?)?,
                    logon_domain_name: &unicode_bytes(required(&request.logon_domain_name)?)?,
                    server_name: &unicode_bytes(required(&request.server_name)?)?,
                    challenge_to_client: request.challenge_to_client.0,
                })?;
                let mut lm3_response = [0; 24];
                lm3_response[..16].copy_from_slice(&result.lm3_response);
                lm3_response[16..].copy_from_slice(&result.challenge_from_client);
                Ok(ndr::NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(
                    ndr::Lm20GetNtlm3ChallengeResponseResponse {
                        ntlm3_response: ndr::NtlmResponseBuffer {
                            length: u16::try_from(result.ntlm3_response.len()).map_err(|_| invalid_parameter())?,
                            value: Some(result.ntlm3_response),
                        },
                        lm3_response: ndr::Opaque(lm3_response),
                        user_session_key: ndr::Opaque(*result.user_session_key.as_ref()),
                        lm_session_key: ndr::Opaque(*result.lm_session_key.as_ref()),
                    },
                ))
            }
            ndr::NtlmRequest::CompareCredentials(request) => {
                let result = self.compare_ntlm_credentials(
                    &ntlm_credential(required(&request.lhs_credential)?)?,
                    &ntlm_credential(required(&request.rhs_credential)?)?,
                )?;
                Ok(ndr::NtlmResponseCall::CompareCredentials(
                    ndr::CompareCredentialsResponse {
                        are_nt_owfs_equal: i32::from(result.are_nt_owfs_equal),
                        are_lm_owfs_equal: i32::from(result.are_lm_owfs_equal),
                        are_sha_owfs_equal: i32::from(result.are_sha_owfs_equal),
                    },
                ))
            }
            _ => Err(Error::new(
                ErrorKind::UnsupportedFunction,
                "RCG NTLM operation is unsupported",
            )),
        }
    }

    fn execute(&mut self, request: &Request) -> sspi::Result<Response> {
        match request {
            Request::ComputeTgsChecksum(request) => {
                let result = self.compute_tgs_checksum(rg::ComputeTgsChecksumRequest {
                    request_body: asn1(required(&request.request_body)?)?,
                    key: key(required(&request.key)?)?,
                    checksum_type: i32::try_from(request.checksum_type).map_err(|_| invalid_parameter())?,
                })?;
                Ok(Response::ComputeTgsChecksum(asn1_output(result)?))
            }
            Request::BuildEncryptedAuthData(request) => {
                let result = self.build_encrypted_auth_data(rg::BuildEncryptedAuthDataRequest {
                    key_usage: request.key_usage,
                    key: key(required(&request.key)?)?,
                    plain_auth_data: asn1(required(&request.plain_auth_data)?)?,
                })?;
                Ok(Response::BuildEncryptedAuthData(asn1_output(result)?))
            }
            Request::CreateApReqAuthenticator(request) => {
                let name = required(&request.client_name)?;
                let names = required(&name.names)?;
                if usize::from(name.name_count) != names.len() {
                    return Err(invalid_parameter());
                }
                let names = names.iter().map(unicode).collect::<sspi::Result<Vec<_>>>()?;
                let realm = unicode(required(&request.client_realm)?)?;
                // SSPI currently ignores SkewTime; reject unsupported adjustments explicitly.
                if request.skew_time.is_some_and(|skew| skew != 0) {
                    return Err(Error::new(
                        ErrorKind::UnsupportedFunction,
                        "RCG server clock adjustment is unsupported",
                    ));
                }
                let result = self.create_ap_req_authenticator(rg::CreateApReqAuthenticatorRequest {
                    encryption_key: key(required(&request.encryption_key)?)?,
                    sequence_number: request.sequence_number,
                    client_name: &names,
                    client_realm: &realm,
                    skew_time: 0,
                    sub_key: request.sub_key.as_ref().map(key).transpose()?,
                    auth_data: request.auth_data.as_ref().map(asn1).transpose()?,
                    gss_checksum: request.gss_checksum.as_ref().map(asn1).transpose()?,
                    key_usage: request.key_usage,
                })?;
                Ok(Response::CreateApReqAuthenticator(
                    ndr::CreateApReqAuthenticatorResponse {
                        authenticator_time: result.authenticator_time,
                        authenticator: asn1_output(result.authenticator)?,
                        kerb_protocol_error: 0,
                    },
                ))
            }
            Request::UnpackKdcReplyBody(request) => {
                let result = self.unpack_kdc_reply_body(rg::UnpackKdcReplyBodyRequest {
                    encrypted_data: asn1(required(&request.encrypted_data)?)?,
                    key: key(required(&request.key)?)?,
                    strengthen_key: request.strengthen_key.as_ref().map(key).transpose()?,
                    key_usage: request.key_usage,
                })?;
                Ok(Response::UnpackKdcReplyBody(ndr::UnpackKdcReplyBodyResponse {
                    kerb_protocol_error: 0,
                    reply_body: asn1_output(result)?,
                }))
            }
            Request::DecryptApReply(request) => {
                let result = self.decrypt_ap_reply(rg::DecryptApReplyRequest {
                    encrypted_reply: asn1(required(&request.encrypted_reply)?)?,
                    key: key(required(&request.key)?)?,
                })?;
                Ok(Response::DecryptApReply(asn1_output(result.as_ref().clone())?))
            }
            Request::PackApReply(request) => {
                let result = self.pack_ap_reply(rg::PackApReplyRequest {
                    reply: asn1(required(&request.reply)?)?,
                    reply_body: asn1(required(&request.reply_body)?)?,
                    session_key: key(required(&request.session_key)?)?,
                })?;
                Ok(Response::PackApReply(output(result)?))
            }
            _ => Err(Error::new(
                ErrorKind::UnsupportedFunction,
                "RCG operation is unsupported",
            )),
        }
    }
}
