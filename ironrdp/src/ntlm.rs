mod messages;
#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, WriteBytesExt};

use self::messages::{client, server};
use crate::{
    encryption::{compute_hmac_md5, rc4::Rc4, HASH_SIZE},
    sspi::{self, CredentialsBuffers, PackageType, Sspi, SspiError, SspiErrorType},
    Credentials,
};

pub const NTLM_VERSION_SIZE: usize = 8;
pub const ENCRYPTED_RANDOM_SESSION_KEY_SIZE: usize = 16;

const SIGNATURE_SIZE: usize =
    SIGNATURE_VERSION_SIZE + SIGNATURE_CHECKSUM_SIZE + SIGNATURE_SEQ_NUM_SIZE;
const CHALLENGE_SIZE: usize = 8;
const SESSION_KEY_SIZE: usize = 16;
const MESSAGE_INTEGRITY_CHECK_SIZE: usize = 16;
const LM_CHALLENGE_RESPONSE_BUFFER_SIZE: usize = HASH_SIZE + CHALLENGE_SIZE;

const SIGNATURE_VERSION_SIZE: usize = 4;
const SIGNATURE_SEQ_NUM_SIZE: usize = 4;
const SIGNATURE_CHECKSUM_SIZE: usize = 8;
const MESSAGES_VERSION: u32 = 1;

#[derive(Copy, Clone, PartialEq, Debug)]
enum NtlmState {
    Initial,
    Negotiate,
    Challenge,
    Authenticate,
    Completion,
    Final,
}

pub struct Ntlm {
    negotiate_message: Option<NegotiateMessage>,
    challenge_message: Option<ChallengeMessage>,
    authenticate_message: Option<AuthenticateMessage>,

    state: NtlmState,
    flags: NegotiateFlags,
    identity: Option<CredentialsBuffers>,
    version: [u8; NTLM_VERSION_SIZE],

    send_single_host_data: bool,

    send_signing_key: [u8; HASH_SIZE],
    recv_signing_key: [u8; HASH_SIZE],
    send_sealing_key: Option<Rc4>,
    recv_sealing_key: Option<Rc4>,
}

#[derive(Clone)]
struct Mic {
    value: [u8; MESSAGE_INTEGRITY_CHECK_SIZE],
    offset: u8,
}

struct NegotiateMessage {
    message: Vec<u8>,
}

struct ChallengeMessage {
    message: Vec<u8>,
    target_info: Vec<u8>,
    server_challenge: [u8; CHALLENGE_SIZE],
    timestamp: u64,
}

struct AuthenticateMessage {
    message: Vec<u8>,
    mic: Option<Mic>,
    target_info: Vec<u8>,
    client_challenge: [u8; CHALLENGE_SIZE],
    encrypted_random_session_key: [u8; ENCRYPTED_RANDOM_SESSION_KEY_SIZE],
}

impl Ntlm {
    pub fn new(credentials: Option<Credentials>, version: [u8; NTLM_VERSION_SIZE]) -> Self {
        Self {
            negotiate_message: None,
            challenge_message: None,
            authenticate_message: None,

            state: NtlmState::Initial,
            flags: NegotiateFlags::empty(),
            identity: credentials.map(std::convert::Into::into),
            version,

            send_single_host_data: false,

            send_signing_key: [0x00; HASH_SIZE],
            recv_signing_key: [0x00; HASH_SIZE],
            send_sealing_key: None,
            recv_sealing_key: None,
        }
    }
}

impl Sspi for Ntlm {
    fn package_type(&self) -> PackageType {
        PackageType::Ntlm
    }
    fn identity(&self) -> Option<CredentialsBuffers> {
        self.identity.clone()
    }
    fn update_identity(&mut self, identity: Option<CredentialsBuffers>) {
        self.identity = identity;
    }
    fn initialize_security_context(
        &mut self,
        input: impl io::Read,
        mut output: impl io::Write,
    ) -> sspi::SspiResult {
        match self.state {
            NtlmState::Initial => {
                self.state = NtlmState::Negotiate;
                client::write_negotiate(self, &mut output)
            }
            NtlmState::Challenge => {
                client::read_challenge(self, input)?;
                client::write_authenticate(self, &mut output)
            }
            _ => Err(SspiError::new(
                SspiErrorType::OutOfSequence,
                format!("Got wrong NTLM state: {:?}", self.state),
            )),
        }
    }
    fn accept_security_context(
        &mut self,
        input: impl io::Read,
        mut output: impl io::Write,
    ) -> sspi::SspiResult {
        match self.state {
            NtlmState::Initial => {
                self.state = NtlmState::Negotiate;
                server::read_negotiate(self, input)?;
                server::write_challenge(self, &mut output)
            }
            NtlmState::Authenticate => server::read_authenticate(self, input),
            _ => Err(SspiError::new(
                SspiErrorType::OutOfSequence,
                format!("got wrong NTLM state: {:?}", self.state),
            )),
        }
    }
    fn complete_auth_token(&mut self) -> sspi::Result<()> {
        server::complete_authenticate(self)
    }
    fn encrypt_message(&mut self, input: &[u8], message_seq_num: u32) -> sspi::Result<Vec<u8>> {
        let digest = compute_digest(&self.send_signing_key, message_seq_num, &input)?;

        let mut data = self.send_sealing_key.as_mut().unwrap().process(input);

        let checksum = self
            .send_sealing_key
            .as_mut()
            .unwrap()
            .process(&digest[0..SIGNATURE_CHECKSUM_SIZE]);
        let mut output = compute_signature(&checksum, message_seq_num).to_vec();
        output.append(&mut data);

        Ok(output)
    }
    fn decrypt_message(&mut self, input: &[u8], message_seq_num: u32) -> sspi::Result<Vec<u8>> {
        let (expected_signature, data) = input.split_at(SIGNATURE_SIZE);

        let decrypted_data = self.recv_sealing_key.as_mut().unwrap().process(data);

        let digest = compute_digest(&self.recv_signing_key, message_seq_num, &decrypted_data)?;
        let checksum = self
            .recv_sealing_key
            .as_mut()
            .unwrap()
            .process(&digest[0..SIGNATURE_CHECKSUM_SIZE]);
        let signature = compute_signature(&checksum, message_seq_num);

        if expected_signature != signature.as_ref() {
            return Err(SspiError::new(
                SspiErrorType::MessageAltered,
                String::from("Signature verification failed, something nasty is going on!"),
            ));
        }

        Ok(decrypted_data)
    }
}

impl NegotiateMessage {
    fn new(message: Vec<u8>) -> Self {
        Self { message }
    }
}

impl ChallengeMessage {
    fn new(
        message: Vec<u8>,
        target_info: Vec<u8>,
        server_challenge: [u8; CHALLENGE_SIZE],
        timestamp: u64,
    ) -> Self {
        Self {
            message,
            target_info,
            server_challenge,
            timestamp,
        }
    }
}

impl AuthenticateMessage {
    fn new(
        message: Vec<u8>,
        mic: Option<Mic>,
        target_info: Vec<u8>,
        client_challenge: [u8; CHALLENGE_SIZE],
        encrypted_random_session_key: [u8; ENCRYPTED_RANDOM_SESSION_KEY_SIZE],
    ) -> Self {
        Self {
            message,
            mic,
            target_info,
            client_challenge,
            encrypted_random_session_key,
        }
    }
}

impl Mic {
    fn new(value: [u8; MESSAGE_INTEGRITY_CHECK_SIZE], offset: u8) -> Self {
        Self { value, offset }
    }
}

bitflags! {
    struct NegotiateFlags: u32 {
        /// W-bit
        /// requests 56-bit encryption
        const NTLM_SSP_NEGOTIATE56 = 0x8000_0000;

        /// V-bit
        /// requests explicit key exchange
        const NTLM_SSP_NEGOTIATE_KEY_EXCH = 0x4000_0000;

        /// U-bit
        /// requests an 128 bit session key
        const NTLM_SSP_NEGOTIATE128 = 0x2000_0000;

        /// r1
        const NTLM_SSP_NEGOTIATE_RESERVED1  = 0x1000_0000;

        /// r2
        const NTLM_SSP_NEGOTIATE_RESERVED2 = 0x0800_0000;

        /// r3
        const NTLM_SSP_NEGOTIATE_RESERVED3 = 0x0400_0000;

        /// r6
        const NTLM_SSP_NEGOTIATE_VERSION = 0x0200_0000;

        /// r4
        const NTLM_SSP_NEGOTIATE_RESERVED4 = 0x0100_0000;

        /// S-bit
        const NTLM_SSP_NEGOTIATE_TARGET_INFO = 0x0080_0000;

        /// R
        const NTLM_SSP_NEGOTIATE_REQUEST_NON_NT_SESSION_KEY = 0x0040_0000;

        /// r5
        const NTLM_SSP_NEGOTIATE_RESERVED5 = 0x0020_0000;

        /// Q
        const NTLM_SSP_NEGOTIATE_IDENTIFY = 0x0010_0000;

        /// P-bit
        /// NTLMv2 Session Security
        const NTLM_SSP_NEGOTIATE_EXTENDED_SESSION_SECURITY = 0x0008_0000;

        /// r6
        const NTLM_SSP_NEGOTIATE_RESERVED6 = 0x0004_0000;

        /// O
        const NTLM_SSP_NEGOTIATE_TARGET_TYPE_SERVER = 0x0002_0000;

        /// N
        const NTLM_SSP_NEGOTIATE_TARGET_TYPE_DOMAIN = 0x0001_0000;

        /// M-bit
        /// requests a signature block
        const NTLM_SSP_NEGOTIATE_ALWAYS_SIGN = 0x0000_8000;

        /// r7
        const NTLM_SSP_NEGOTIATE_RESERVED7 = 0x0000_4000;

        /// L-bit
        const NTLM_SSP_NEGOTIATE_WORKSTATION_SUPPLIED = 0x0000_2000;

        /// K-bit
        const NTLM_SSP_NEGOTIATE_DOMAIN_SUPPLIED = 0x0000_1000;

        /// J
        const NTLM_SSP_NEGOTIATE_ANONYMOUS = 0x0000_0800;

        /// r8
        const NTLM_SSP_NEGOTIATE_RESERVED8 = 0x0000_0400;

        /// H-bit
        /// NTLMv1 Session Security, deprecated, insecure and not supported by us
        const NTLM_SSP_NEGOTIATE_NTLM = 0x0000_0200;

        /// r9
        const NTLM_SSP_NEGOTIATE_RESERVED9 = 0x0000_0100;

        /// G-bit
        /// LM Session Security, deprecated, insecure and not supported by us
        const NTLM_SSP_NEGOTIATE_LM_KEY = 0x0000_0080;

        /// F
        const NTLM_SSP_NEGOTIATE_DATAGRAM = 0x0000_0040;

        /// E-bit
        /// session key negotiation with message confidentiality
        const NTLM_SSP_NEGOTIATE_SEAL = 0x0000_0020;

        /// D-bit
        const NTLM_SSP_NEGOTIATE_SIGN = 0x0000_0010;

        /// r10
        const NTLM_SSP_NEGOTIATE_SIGN_RESERVED10 = 0x0000_0008;

        /// C-bit
        const NTLM_SSP_NEGOTIATE_REQUEST_TARGET = 0x0000_0004;

        /// B-bit
        const NTLM_SSP_NEGOTIATE_OEM = 0x0000_0002;

        /// A-bit
        const NTLM_SSP_NEGOTIATE_UNICODE = 0x0000_0001;
    }
}

fn compute_digest(key: &[u8], seq_num: u32, data: &[u8]) -> io::Result<[u8; 16]> {
    let mut digest_data = Vec::with_capacity(SIGNATURE_SEQ_NUM_SIZE + data.len());
    digest_data.write_u32::<LittleEndian>(seq_num)?;
    digest_data.extend_from_slice(data);

    compute_hmac_md5(key, &digest_data)
}

fn compute_signature(checksum: &[u8], seq_num: u32) -> [u8; SIGNATURE_SIZE] {
    let mut signature = [0x00; SIGNATURE_SIZE];
    signature[..SIGNATURE_VERSION_SIZE].clone_from_slice(&MESSAGES_VERSION.to_le_bytes());
    signature[SIGNATURE_VERSION_SIZE..SIGNATURE_VERSION_SIZE + SIGNATURE_CHECKSUM_SIZE]
        .clone_from_slice(&checksum);
    signature[SIGNATURE_VERSION_SIZE + SIGNATURE_CHECKSUM_SIZE..]
        .clone_from_slice(&seq_num.to_le_bytes());

    signature
}
