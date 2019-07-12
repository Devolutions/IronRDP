pub mod ts_request;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use rand::{rngs::OsRng, Rng};

use self::ts_request::{TsRequest, NONCE_SIZE};
use crate::{
    encryption::compute_sha256,
    nego::NegotiationRequestFlags,
    ntlm::{Ntlm, NTLM_VERSION_SIZE},
    sspi::{self, CredentialsBuffers, PackageType, Sspi, SspiError, SspiErrorType, SspiOk},
    Credentials, PduParsing,
};

pub const EARLY_USER_AUTH_RESULT_PDU_SIZE: usize = 4;

const HASH_MAGIC_LEN: usize = 38;
const SERVER_CLIENT_HASH_MAGIC: &[u8; HASH_MAGIC_LEN] = b"CredSSP Server-To-Client Binding Hash\0";
const CLIENT_SERVER_HASH_MAGIC: &[u8; HASH_MAGIC_LEN] = b"CredSSP Client-To-Server Binding Hash\0";

/// Provides an interface for implementing proxy credentials structures.
pub trait CredentialsProxy {
    /// A method signature for implementing a behavior of searching and returning
    /// a user password based on a username and a domain provided as arguments.
    ///
    /// # Arguments
    ///
    /// * `username` - the username string
    /// * `domain` - the domain string (optional)
    fn password_by_user(&mut self, username: String, domain: Option<String>) -> io::Result<String>;
}

/// Provides an interface to be implemented by the CredSSP-related structs:
/// [`CredSspServer`](struct.CredSspServer.html) and
/// [`CredSspClient`](struct.CredSspClient.html).
pub trait CredSsp {
    fn process(&mut self, ts_request: TsRequest) -> sspi::Result<CredSspResult>;
}

/// Implements the CredSSP *client*. The client's credentials are to
/// be securely delegated to the server.
///
/// # MSDN
///
/// * [Glossary](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/97e4a826-1112-4ab4-8662-cfa58418b4c1)
pub struct CredSspClient {
    state: CredSspState,
    context: Option<CredSspContext>,
    credentials: Credentials,
    version: Vec<u8>,
    public_key: Vec<u8>,
    nego_flags: NegotiationRequestFlags,
    client_nonce: [u8; NONCE_SIZE],
}

/// Implements the CredSSP *server*. The client's credentials
/// securely delegated to the server for authentication using TLS.
///
/// # MSDN
///
/// * [Glossary](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/97e4a826-1112-4ab4-8662-cfa58418b4c1)
pub struct CredSspServer<C: CredentialsProxy> {
    pub credentials: C,
    state: CredSspState,
    context: Option<CredSspContext>,
    version: Vec<u8>,
    public_key: Vec<u8>,
}

/// The result of a CredSSP client or server processing.
/// The enum may carry a [`TsRequest`](struct.TsRequest.html) or
/// [`Credentials`](struct.Credentials.html).
#[derive(Debug)]
pub enum CredSspResult {
    /// Used as a result of processing of negotiation tokens by the client and server.
    ReplyNeeded(TsRequest),
    /// Used as a result of processing of authentication info by the client.
    FinalMessage(TsRequest),
    /// Used by the server when processing of negotiation tokens resulted in error.
    WithError(TsRequest),
    /// Used as a result of the final state of the client and server.
    Finished,
    /// Used as a result of  processing of authentication info by the server.
    ClientCredentials(Credentials),
}

/// The Early User Authorization Result PDU is sent from server to client
/// and is used to convey authorization information to the client.
/// This PDU is only sent by the server if the client advertised support for it
/// by specifying the ['HYBRID_EX protocol'](struct.SecurityProtocol.htlm)
/// of the RDP Negotiation Request and it MUST be sent immediately
/// after the CredSSP handshake has completed.
#[derive(Debug, FromPrimitive, ToPrimitive)]
#[repr(u32)]
pub enum EarlyUserAuthResult {
    /// The user has permission to access the server.
    Success = 0,
    /// The user does not have permission to access the server.
    AccessDenied = 5,
}

impl PduParsing for EarlyUserAuthResult {
    type Error = io::Error;

    fn from_buffer(mut stream: impl std::io::Read) -> Result<Self, Self::Error> {
        let result = stream.read_u32::<LittleEndian>()?;

        EarlyUserAuthResult::from_u32(result).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Got invalid Early User Authorization Result",
            )
        })
    }
    fn to_buffer(&self, mut stream: impl std::io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.to_u32().unwrap())
    }
    fn buffer_length(&self) -> usize {
        EARLY_USER_AUTH_RESULT_PDU_SIZE
    }
}

#[derive(Copy, Clone, PartialEq)]
enum CredSspState {
    Initial,
    NegoToken,
    AuthInfo,
    Final,
}

#[derive(PartialEq)]
enum EndpointType {
    Client,
    Server,
}

struct CredSspContext {
    peer_version: Option<u32>,
    sspi_context: SspiProvider,
    send_seq_num: u32,
    recv_seq_num: u32,
}

enum SspiProvider {
    NtlmContext(Ntlm),
}

impl CredSspClient {
    pub fn new(
        public_key: Vec<u8>,
        credentials: Credentials,
        version: Vec<u8>,
        nego_flags: NegotiationRequestFlags,
    ) -> sspi::Result<Self> {
        Ok(Self {
            state: CredSspState::Initial,
            context: None,
            credentials,
            version,
            public_key,
            nego_flags,
            client_nonce: OsRng::new()?.gen::<[u8; NONCE_SIZE]>(),
        })
    }
}

impl<C: CredentialsProxy> CredSspServer<C> {
    pub fn new(public_key: Vec<u8>, credentials: C, version: Vec<u8>) -> sspi::Result<Self> {
        Ok(Self {
            state: CredSspState::Initial,
            context: None,
            credentials,
            version,
            public_key,
        })
    }
}

impl SspiProvider {
    pub fn new_ntlm(credentials: Option<Credentials>, version: Vec<u8>) -> Self {
        let mut ntlm_version = [0x00; NTLM_VERSION_SIZE];
        ntlm_version.clone_from_slice(version.as_ref());

        SspiProvider::NtlmContext(Ntlm::new(credentials, ntlm_version))
    }
}

impl CredSsp for CredSspClient {
    fn process(&mut self, mut ts_request: TsRequest) -> sspi::Result<CredSspResult> {
        ts_request.check_error()?;
        if let Some(ref mut context) = self.context {
            context.check_peer_version(&ts_request)?;
        }

        loop {
            match self.state {
                CredSspState::Initial => {
                    self.context = Some(CredSspContext::new(SspiProvider::new_ntlm(
                        Some(self.credentials.clone()),
                        self.version.clone(),
                    )));

                    self.state = CredSspState::NegoToken;
                }
                CredSspState::NegoToken => {
                    let input = ts_request.nego_tokens.take().unwrap_or_default();
                    let mut output = Vec::new();
                    let status = self
                        .context
                        .as_mut()
                        .unwrap()
                        .sspi_context
                        .initialize_security_context(input.as_slice(), &mut output)?;
                    ts_request.nego_tokens = Some(output);
                    if status == SspiOk::CompleteNeeded {
                        let peer_version = self.context.as_ref().unwrap().peer_version.expect(
                            "An encrypt public key client function cannot be fired without any incoming TSRequest",
                        );
                        ts_request.pub_key_auth =
                            Some(self.context.as_mut().unwrap().encrypt_public_key(
                                self.public_key.as_ref(),
                                EndpointType::Client,
                                &Some(self.client_nonce),
                                peer_version,
                            )?);
                        ts_request.client_nonce = Some(self.client_nonce);
                        self.state = CredSspState::AuthInfo;
                    }

                    return Ok(CredSspResult::ReplyNeeded(ts_request));
                }
                CredSspState::AuthInfo => {
                    ts_request.nego_tokens = None;

                    let pub_key_auth = ts_request.pub_key_auth.take().ok_or_else(|| {
                        SspiError::new(
                            SspiErrorType::InvalidToken,
                            String::from("Expected an encrypted public key"),
                        )
                    })?;
                    let peer_version =
                        self.context.as_ref().unwrap().peer_version.expect(
                            "An decrypt public key client function cannot be fired without any incoming TSRequest",
                        );
                    self.context.as_mut().unwrap().decrypt_public_key(
                        self.public_key.as_ref(),
                        pub_key_auth.as_ref(),
                        EndpointType::Client,
                        &Some(self.client_nonce),
                        peer_version,
                    )?;

                    ts_request.auth_info = Some(
                        self.context
                            .as_mut()
                            .unwrap()
                            .encrypt_ts_credentials(self.nego_flags)?,
                    );

                    self.state = CredSspState::Final;

                    return Ok(CredSspResult::FinalMessage(ts_request));
                }
                CredSspState::Final => return Ok(CredSspResult::Finished),
            }
        }
    }
}

impl<C: CredentialsProxy> CredSsp for CredSspServer<C> {
    fn process(&mut self, mut ts_request: TsRequest) -> sspi::Result<CredSspResult> {
        if let Some(ref mut context) = self.context {
            context.check_peer_version(&ts_request)?;
        }

        loop {
            match self.state {
                CredSspState::Initial => {
                    self.context = Some(CredSspContext::new(SspiProvider::new_ntlm(
                        None,
                        self.version.clone(),
                    )));

                    self.state = CredSspState::NegoToken;
                }
                CredSspState::NegoToken => {
                    let input = ts_request.nego_tokens.take().ok_or_else(|| {
                        SspiError::new(
                            SspiErrorType::InvalidToken,
                            String::from("Got empty nego_tokens field"),
                        )
                    })?;
                    let mut output = Vec::new();
                    match self
                        .context
                        .as_mut()
                        .unwrap()
                        .sspi_context
                        .accept_security_context(input.as_slice(), &mut output)
                    {
                        Ok(SspiOk::ContinueNeeded) => {
                            ts_request.nego_tokens = Some(output);
                        }
                        Ok(SspiOk::CompleteNeeded) => {
                            match self.context.as_ref().unwrap().sspi_context.package_type() {
                                PackageType::Ntlm => {
                                    let mut credentials: Credentials = self
                                        .context
                                        .as_ref()
                                        .unwrap()
                                        .sspi_context
                                        .identity()
                                        .expect(
                                            "Identity must be set from NTLM authenticate message",
                                        )
                                        .into();

                                    credentials.password = self.credentials.password_by_user(
                                        credentials.username.clone(),
                                        credentials.domain.clone(),
                                    )?;;

                                    self.context
                                        .as_mut()
                                        .unwrap()
                                        .sspi_context
                                        .update_identity(Some(credentials.into()));
                                }
                            };

                            self.context
                                .as_mut()
                                .unwrap()
                                .sspi_context
                                .complete_auth_token()?;
                            ts_request.nego_tokens = None;

                            let pub_key_auth = ts_request.pub_key_auth.take().ok_or_else(|| {
                                SspiError::new(
                                    SspiErrorType::InvalidToken,
                                    String::from("Expected an encrypted public key"),
                                )
                            })?;
                            let peer_version = self.context.as_ref().unwrap().peer_version.expect(
                                "An decrypt public key server function cannot be fired without any incoming TSRequest",
                            );
                            self.context.as_mut().unwrap().decrypt_public_key(
                                self.public_key.as_ref(),
                                pub_key_auth.as_ref(),
                                EndpointType::Server,
                                &ts_request.client_nonce,
                                peer_version,
                            )?;
                            ts_request.pub_key_auth =
                                Some(self.context.as_mut().unwrap().encrypt_public_key(
                                    self.public_key.as_ref(),
                                    EndpointType::Server,
                                    &ts_request.client_nonce,
                                    peer_version,
                                )?);

                            self.state = CredSspState::AuthInfo;
                        }
                        Err(e) => {
                            ts_request.error_code = Some(
                                ((e.error_type as i64 & 0x0000_FFFF) | (0x7 << 16) | 0xC000_0000)
                                    as u32,
                            );

                            return Ok(CredSspResult::WithError(ts_request));
                        }
                    };

                    return Ok(CredSspResult::ReplyNeeded(ts_request));
                }
                CredSspState::AuthInfo => {
                    let auth_info = ts_request.auth_info.take().ok_or_else(|| {
                        SspiError::new(
                            SspiErrorType::InvalidToken,
                            String::from("Expected an encrypted ts credentials"),
                        )
                    })?;
                    self.state = CredSspState::Final;

                    let read_credentials = self
                        .context
                        .as_mut()
                        .unwrap()
                        .decrypt_ts_credentials(&auth_info)?
                        .into();

                    return Ok(CredSspResult::ClientCredentials(read_credentials));
                }
                CredSspState::Final => return Ok(CredSspResult::Finished),
            }
        }
    }
}

impl CredSspContext {
    fn new(sspi_context: SspiProvider) -> Self {
        Self {
            peer_version: None,
            send_seq_num: 0,
            recv_seq_num: 0,
            sspi_context,
        }
    }

    fn check_peer_version(&mut self, ts_request: &TsRequest) -> sspi::Result<()> {
        match (self.peer_version, ts_request.peer_version) {
            (Some(peer_version), Some(other_peer_version)) => {
                if peer_version != other_peer_version {
                    Err(SspiError::new(
                        SspiErrorType::MessageAltered,
                        format!(
                            "CredSSP peer changed protocol version from {} to {}",
                            peer_version, other_peer_version
                        ),
                    ))
                } else {
                    Ok(())
                }
            }
            (None, Some(other_peer_version)) => {
                self.peer_version = Some(other_peer_version);

                Ok(())
            }
            _ => Err(SspiError::new(
                SspiErrorType::InvalidToken,
                String::from("CredSSP peer did not provide the version"),
            )),
        }
    }

    fn encrypt_public_key(
        &mut self,
        public_key: &[u8],
        endpoint: EndpointType,
        client_nonce: &Option<[u8; NONCE_SIZE]>,
        peer_version: u32,
    ) -> sspi::Result<Vec<u8>> {
        let hash_magic = match endpoint {
            EndpointType::Client => CLIENT_SERVER_HASH_MAGIC,
            EndpointType::Server => SERVER_CLIENT_HASH_MAGIC,
        };

        if peer_version < 5 {
            self.encrypt_public_key_echo(public_key, endpoint)
        } else {
            self.encrypt_public_key_hash(
                public_key,
                hash_magic,
                &client_nonce.ok_or(SspiError::new(
                    SspiErrorType::InvalidToken,
                    String::from(
                        "client nonce from the TSRequest is empty, but a peer version is >= 5",
                    ),
                ))?,
            )
        }
    }

    fn decrypt_public_key(
        &mut self,
        public_key: &[u8],
        encrypted_public_key: &[u8],
        endpoint: EndpointType,
        client_nonce: &Option<[u8; NONCE_SIZE]>,
        peer_version: u32,
    ) -> sspi::Result<()> {
        let hash_magic = match endpoint {
            EndpointType::Client => SERVER_CLIENT_HASH_MAGIC,
            EndpointType::Server => CLIENT_SERVER_HASH_MAGIC,
        };

        if peer_version < 5 {
            self.decrypt_public_key_echo(public_key, encrypted_public_key, endpoint)
        } else {
            self.decrypt_public_key_hash(
                public_key,
                encrypted_public_key,
                hash_magic,
                &client_nonce.ok_or(SspiError::new(
                    SspiErrorType::InvalidToken,
                    String::from(
                        "client nonce from the TSRequest is empty, but a peer version is >= 5",
                    ),
                ))?,
            )
        }
    }

    fn encrypt_public_key_echo(
        &mut self,
        public_key: &[u8],
        endpoint: EndpointType,
    ) -> sspi::Result<Vec<u8>> {
        let mut public_key = public_key.to_vec();

        match self.sspi_context.package_type() {
            PackageType::Ntlm => {
                if endpoint == EndpointType::Server {
                    integer_increment_le(&mut public_key);
                }
            }
        };

        self.encrypt_message(&public_key)
    }

    fn encrypt_public_key_hash(
        &mut self,
        public_key: &[u8],
        hash_magic: &[u8],
        client_nonce: &[u8],
    ) -> sspi::Result<Vec<u8>> {
        let mut data = hash_magic.to_vec();
        data.extend(client_nonce);
        data.extend(public_key);
        let encrypted_public_key = compute_sha256(&data);

        self.encrypt_message(&encrypted_public_key)
    }

    fn decrypt_public_key_echo(
        &mut self,
        public_key: &[u8],
        encrypted_public_key: &[u8],
        endpoint: EndpointType,
    ) -> sspi::Result<()> {
        let mut decrypted_public_key = self.decrypt_message(encrypted_public_key)?;
        if endpoint == EndpointType::Client {
            integer_decrement_le(&mut decrypted_public_key);
        }

        if public_key != decrypted_public_key.as_slice() {
            return Err(SspiError::new(
                SspiErrorType::MessageAltered,
                String::from("Could not verify a public key echo"),
            ));
        }

        Ok(())
    }

    fn decrypt_public_key_hash(
        &mut self,
        public_key: &[u8],
        encrypted_public_key: &[u8],
        hash_magic: &[u8],
        client_nonce: &[u8],
    ) -> sspi::Result<()> {
        let decrypted_public_key = self.decrypt_message(encrypted_public_key)?;

        let mut data = hash_magic.to_vec();
        data.extend(client_nonce);
        data.extend(public_key);
        let expected_public_key = compute_sha256(&data);

        if expected_public_key.as_ref() != decrypted_public_key.as_slice() {
            return Err(SspiError::new(
                SspiErrorType::MessageAltered,
                String::from("Could not verify a public key hash"),
            ));
        }

        Ok(())
    }

    fn encrypt_ts_credentials(
        &mut self,
        nego_flags: NegotiationRequestFlags,
    ) -> sspi::Result<Vec<u8>> {
        let ts_credentials = ts_request::write_ts_credentials(
            self.sspi_context
                .identity()
                .as_ref()
                .expect("Identity must be set from authenticate message"),
            nego_flags,
        )?;

        self.encrypt_message(&ts_credentials)
    }

    fn decrypt_ts_credentials(&mut self, auth_info: &[u8]) -> sspi::Result<CredentialsBuffers> {
        let ts_credentials_buffer = self.decrypt_message(&auth_info)?;

        Ok(ts_request::read_ts_credentials(
            ts_credentials_buffer.as_slice(),
        )?)
    }

    fn encrypt_message(&mut self, buffer: &[u8]) -> sspi::Result<Vec<u8>> {
        let send_seq_num = self.send_seq_num;
        let encrypted_buffer = self.sspi_context.encrypt_message(buffer, send_seq_num)?;
        self.send_seq_num += 1;

        // there will be magic transform for the kerberos

        Ok(encrypted_buffer)
    }

    fn decrypt_message(&mut self, buffer: &[u8]) -> sspi::Result<Vec<u8>> {
        let recv_seq_num = self.recv_seq_num;
        let decrypted_buffer = self.sspi_context.decrypt_message(buffer, recv_seq_num)?;
        self.recv_seq_num += 1;

        Ok(decrypted_buffer)
    }
}

impl Sspi for SspiProvider {
    fn package_type(&self) -> PackageType {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.package_type(),
        }
    }
    fn identity(&self) -> Option<CredentialsBuffers> {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.identity(),
        }
    }
    fn update_identity(&mut self, identity: Option<CredentialsBuffers>) {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.update_identity(identity),
        }
    }
    fn initialize_security_context(
        &mut self,
        input: impl std::io::Read,
        output: impl std::io::Write,
    ) -> sspi::SspiResult {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.initialize_security_context(input, output),
        }
    }
    fn accept_security_context(
        &mut self,
        input: impl std::io::Read,
        output: impl std::io::Write,
    ) -> sspi::SspiResult {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.accept_security_context(input, output),
        }
    }
    fn complete_auth_token(&mut self) -> sspi::Result<()> {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.complete_auth_token(),
        }
    }
    fn encrypt_message(&mut self, input: &[u8], message_seq_number: u32) -> sspi::Result<Vec<u8>> {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.encrypt_message(input, message_seq_number),
        }
    }
    fn decrypt_message(&mut self, input: &[u8], message_seq_number: u32) -> sspi::Result<Vec<u8>> {
        match self {
            SspiProvider::NtlmContext(ntlm) => ntlm.decrypt_message(input, message_seq_number),
        }
    }
}

fn integer_decrement_le(buffer: &mut [u8]) {
    for elem in buffer.iter_mut() {
        let (value, overflow) = elem.overflowing_sub(1);
        *elem = value;
        if !overflow {
            break;
        }
    }
}

fn integer_increment_le(buffer: &mut [u8]) {
    for elem in buffer.iter_mut() {
        let (value, overflow) = elem.overflowing_add(1);
        *elem = value;
        if !overflow {
            break;
        }
    }
}
