#[cfg(test)]
mod test;

use std::io::{self, Read};

use crate::{
    ber,
    nego::NegotiationRequestFlags,
    sspi::{self, CredentialsBuffers, SspiError, SspiErrorType},
};

pub const NONCE_SIZE: usize = 32;
const NLA_VERSION: u32 = 6;
const NONCE_FIELD_LEN: u16 = 36;

/// Used for communication in the CredSSP [client](struct.CredSspServer.html)
/// and [server](struct.CredSspServer.html). It's a top-most structure that
/// they use.
///
/// # MSDN
///
/// * [TSRequest](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/6aac4dea-08ef-47a6-8747-22ea7f6d8685)
#[derive(Debug, Clone, Default)]
pub struct TsRequest {
    /// Specifies the supported version of the CredSSP protocol.
    pub peer_version: Option<u32>,
    /// Contains the SPNEGO tokens or NTLM messages that are passed between the client
    /// and server.
    pub nego_tokens: Option<Vec<u8>>,
    /// Contains the user's credentials that are delegated to the server.
    pub auth_info: Option<Vec<u8>>,
    /// Used to assure that the public key that is used by the server during
    /// the TLS handshake belongs to the target server and not to a man-in-the-middle.
    pub pub_key_auth: Option<Vec<u8>>,
    /// If the SPNEGO exchange fails on the server, this field is used to send
    /// the failure code to the client.
    pub error_code: Option<u32>,
    /// An array of cryptographically random bytes used to provide sufficient
    /// entropy during hash computation.
    pub client_nonce: Option<[u8; NONCE_SIZE]>,
}

impl TsRequest {
    /// Creates a `TsRequest` structure from a raw array.
    ///
    /// # Arguments
    ///
    /// * `buffer` - the array of bytess
    pub fn from_buffer(buffer: &[u8]) -> io::Result<TsRequest> {
        let mut stream = io::Cursor::new(buffer);

        let ts_request_len = ber::read_sequence_tag(&mut stream)
            .map_err(|e| io::Error::new(io::ErrorKind::UnexpectedEof, e))?;
        if buffer.len() < ber::sizeof_sequence(ts_request_len) as usize {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Incomplete buffer",
            ));
        }

        ber::read_contextual_tag(&mut stream, 0, ber::Pc::Construct)?;

        let peer_version = ber::read_integer(&mut stream)? as u32;

        let nego_tokens =
            if ber::read_contextual_tag_or_unwind(&mut stream, 1, ber::Pc::Construct)?.is_some() {
                ber::read_sequence_tag(&mut stream)?;
                ber::read_sequence_tag(&mut stream)?;
                ber::read_contextual_tag(&mut stream, 0, ber::Pc::Construct)?;
                let length = ber::read_octet_string_tag(&mut stream)?;
                let mut nego_tokens = vec![0x00; length as usize];
                stream.read_exact(&mut nego_tokens)?;

                Some(nego_tokens)
            } else {
                None
            };

        let auth_info =
            if ber::read_contextual_tag_or_unwind(&mut stream, 2, ber::Pc::Construct)?.is_some() {
                let length = ber::read_octet_string_tag(&mut stream)?;
                let mut auth_info = vec![0x00; length as usize];
                stream.read_exact(&mut auth_info)?;

                Some(auth_info)
            } else {
                None
            };

        let pub_key_auth =
            if ber::read_contextual_tag_or_unwind(&mut stream, 3, ber::Pc::Construct)?.is_some() {
                let length = ber::read_octet_string_tag(&mut stream)?;
                let mut pub_key_auth = vec![0x00; length as usize];
                stream.read_exact(&mut pub_key_auth)?;

                Some(pub_key_auth)
            } else {
                None
            };

        let error_code = if peer_version >= 3
            && ber::read_contextual_tag_or_unwind(&mut stream, 4, ber::Pc::Construct)?.is_some()
        {
            let read_error_code = ber::read_integer(&mut stream)?;
            let error_code = read_error_code as u32;

            Some(error_code)
        } else {
            None
        };

        let client_nonce = if peer_version >= 5
            && ber::read_contextual_tag_or_unwind(&mut stream, 5, ber::Pc::Construct)?.is_some()
        {
            let length = ber::read_octet_string_tag(&mut stream)?;
            if length != NONCE_SIZE as u16 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Got ClientNonce with invalid length: {}", length),
                ));
            }

            let mut client_nonce = [0x00; NONCE_SIZE];
            stream.read_exact(&mut client_nonce)?;

            Some(client_nonce)
        } else {
            None
        };

        Ok(TsRequest {
            nego_tokens,
            auth_info,
            pub_key_auth,
            error_code,
            client_nonce,
            peer_version: Some(peer_version),
        })
    }

    /// Encodes the `TsRequest` to be ready to be sent to the TLS stream.
    ///
    /// # Arguments
    ///
    /// * `buffer` - an output buffer
    pub fn encode_ts_request(&self, mut buffer: impl io::Write) -> io::Result<()> {
        let len = self.ts_request_len();

        ber::write_sequence_tag(&mut buffer, len)?;
        /* [0] version */
        ber::write_contextual_tag(&mut buffer, 0, 3, ber::Pc::Construct)?;
        ber::write_integer(&mut buffer, NLA_VERSION)?;

        /* [1] negoTokens (NegoData) */
        if let Some(ref nego_tokens) = self.nego_tokens {
            ber::write_contextual_tag(
                &mut buffer,
                1,
                ber::sizeof_sequence(ber::sizeof_sequence(ber::sizeof_sequence_octet_string(
                    nego_tokens.len() as u16,
                ))),
                ber::Pc::Construct,
            )?;
            ber::write_sequence_tag(
                &mut buffer,
                ber::sizeof_sequence(ber::sizeof_sequence_octet_string(nego_tokens.len() as u16)),
            )?; /* SEQUENCE OF NegoDataItem */
            ber::write_sequence_tag(
                &mut buffer,
                ber::sizeof_sequence_octet_string(nego_tokens.len() as u16),
            )?; /* NegoDataItem */
            ber::write_sequence_octet_string(&mut buffer, 0, &nego_tokens)?; /* OCTET STRING */
        }

        /* [2] authInfo (OCTET STRING) */
        if let Some(ref auth_info) = self.auth_info {
            ber::write_sequence_octet_string(&mut buffer, 2, auth_info)?;
        }

        /* [3] pubKeyAuth (OCTET STRING) */
        if let Some(ref pub_key_auth) = self.pub_key_auth {
            ber::write_sequence_octet_string(&mut buffer, 3, pub_key_auth)?;
        }

        /* [4] errorCode (INTEGER) */
        if self.peer_version.is_some()
            && self.peer_version.unwrap() >= 3
            && self.error_code.is_some()
        {
            let (error_code_len, _) =
                get_error_code_len(self.peer_version.unwrap(), self.error_code);
            ber::write_contextual_tag(&mut buffer, 4, error_code_len, ber::Pc::Construct)?;
            ber::write_integer(&mut buffer, self.error_code.unwrap())?;
        }

        /* [5] clientNonce (OCTET STRING) */
        if self.peer_version.is_some()
            && self.peer_version.unwrap() >= 5
            && self.client_nonce.is_some()
        {
            ber::write_sequence_octet_string(&mut buffer, 5, self.client_nonce.as_ref().unwrap())?;
        }

        Ok(())
    }

    pub fn buffer_len(&self) -> u16 {
        ber::sizeof_sequence(self.ts_request_len())
    }

    pub fn check_error(&self) -> sspi::Result<()> {
        match self.error_code {
            Some(error_code) if error_code != 0 => Err(SspiError::new(
                SspiErrorType::InvalidToken,
                format!("Server has returned an error: 0x{:x}", error_code),
            )),
            _ => Ok(()),
        }
    }

    fn ts_request_len(&self) -> u16 {
        let (error_code_len, error_code_context_len) = match self.peer_version {
            Some(peer_version) => get_error_code_len(peer_version, self.error_code),
            None => (0, 0),
        };
        let client_nonce_len = if self.client_nonce.is_some()
            && self.peer_version.is_some()
            && self.peer_version.unwrap() >= 5
        {
            NONCE_FIELD_LEN
        } else {
            0
        };
        let fields_len = get_nego_tokens_len(&self.nego_tokens)
            + get_field_len(&self.pub_key_auth)
            + get_field_len(&self.auth_info)
            + client_nonce_len
            + error_code_context_len
            + error_code_len;

        fields_len + ber::sizeof_integer(2) + ber::sizeof_contextual_tag(3)
    }
}

pub fn write_ts_credentials(
    identity: &CredentialsBuffers,
    nego_flags: NegotiationRequestFlags,
) -> io::Result<Vec<u8>> {
    let empty_identity = CredentialsBuffers::default();
    let identity = if nego_flags.contains(NegotiationRequestFlags::RESTRICTED_ADMIN_MODE_REQUIED) {
        &empty_identity
    } else {
        identity
    };

    let ts_credentials_len = sizeof_ts_credentials(identity);
    let ts_credentials_sequence_len = ber::sizeof_sequence(ts_credentials_len);
    let password_credentials_len = sizeof_ts_password_creds(identity);
    let password_credentials_sequence_len = ber::sizeof_sequence(password_credentials_len);

    let mut buffer = Vec::with_capacity(ts_credentials_sequence_len as usize);

    // TSCredentials (SEQUENCE)
    ber::write_sequence_tag(&mut buffer, ts_credentials_len)?;
    // [0] credType (INTEGER)
    ber::write_contextual_tag(&mut buffer, 0, ber::sizeof_integer(1), ber::Pc::Construct)?;
    ber::write_integer(&mut buffer, 1)?;
    /* [1] credentials (OCTET STRING) */
    ber::write_contextual_tag(
        &mut buffer,
        1,
        ber::sizeof_octet_string(password_credentials_sequence_len),
        ber::Pc::Construct,
    )?;
    ber::write_octet_string_tag(&mut buffer, password_credentials_sequence_len)?;

    /* TSPasswordCreds (SEQUENCE) */
    ber::write_sequence_tag(&mut buffer, password_credentials_len)?;
    /* [0] domainName (OCTET STRING) */
    ber::write_sequence_octet_string(&mut buffer, 0, &identity.domain)?;
    /* [1] userName (OCTET STRING) */
    ber::write_sequence_octet_string(&mut buffer, 1, &identity.user)?;
    /* [2] password (OCTET STRING) */
    ber::write_sequence_octet_string(&mut buffer, 2, &identity.password)?;

    Ok(buffer)
}

pub fn read_ts_credentials(mut buffer: impl io::Read) -> io::Result<CredentialsBuffers> {
    // TSCredentials (SEQUENCE)
    ber::read_sequence_tag(&mut buffer)?;
    // [0] credType (INTEGER)
    ber::read_contextual_tag(&mut buffer, 0, ber::Pc::Construct)?;
    ber::read_integer(&mut buffer)?;
    // [1] credentials (OCTET STRING)
    ber::read_contextual_tag(&mut buffer, 1, ber::Pc::Construct)?;
    ber::read_octet_string_tag(&mut buffer)?;

    // Read TS password credentials
    let _len = ber::read_sequence_tag(&mut buffer)?;

    /* [0] domainName (OCTET STRING) */
    ber::read_contextual_tag(&mut buffer, 0, ber::Pc::Construct)?;
    let length = ber::read_octet_string_tag(&mut buffer)?;
    let mut domain = vec![0x00; length as usize];
    if length > 0 {
        buffer.read_exact(&mut domain)?;
    }

    /* [1] userName (OCTET STRING) */
    ber::read_contextual_tag(&mut buffer, 1, ber::Pc::Construct)?;
    let length = ber::read_octet_string_tag(&mut buffer)?;
    let mut user = vec![0x00; length as usize];
    if length > 0 {
        buffer.read_exact(&mut user)?;
    }

    /* [2] password (OCTET STRING) */
    ber::read_contextual_tag(&mut buffer, 2, ber::Pc::Construct)?;
    let length = ber::read_octet_string_tag(&mut buffer)?;
    let mut password = vec![0x00; length as usize];
    if length > 0 {
        buffer.read_exact(&mut password)?;
    }

    Ok(CredentialsBuffers::new(user, domain, password))
}

fn sizeof_ts_credentials(identity: &CredentialsBuffers) -> u16 {
    ber::sizeof_integer(1)
        + ber::sizeof_contextual_tag(ber::sizeof_integer(1))
        + ber::sizeof_sequence_octet_string(ber::sizeof_sequence(sizeof_ts_password_creds(
            &identity,
        )))
}

fn sizeof_ts_password_creds(identity: &CredentialsBuffers) -> u16 {
    ber::sizeof_sequence_octet_string(identity.domain.len() as u16)
        + ber::sizeof_sequence_octet_string(identity.user.len() as u16)
        + ber::sizeof_sequence_octet_string(identity.password.len() as u16)
}

fn get_nego_tokens_len(nego_tokens: &Option<Vec<u8>>) -> u16 {
    match nego_tokens {
        Some(nego_tokens) => {
            let nego_len = nego_tokens.len() as u16;
            let mut len = ber::sizeof_octet_string(nego_len);
            len += ber::sizeof_contextual_tag(len);
            len += ber::sizeof_sequence_tag(len);
            len += ber::sizeof_sequence_tag(len);
            len += ber::sizeof_contextual_tag(len);

            len
        }
        None => 0,
    }
}

fn get_error_code_len(version: u32, error_code: Option<u32>) -> (u16, u16) {
    if version >= 3 && version != 5 && error_code.is_some() {
        let len = ber::sizeof_integer(error_code.unwrap());
        let context_len = ber::sizeof_contextual_tag(len);

        (len, context_len)
    } else {
        (0, 0)
    }
}

fn get_field_len(field: &Option<Vec<u8>>) -> u16 {
    match field {
        Some(field) => {
            let field_len = field.len() as u16;
            let mut len = ber::sizeof_octet_string(field_len);
            len += ber::sizeof_contextual_tag(len);

            len
        }
        None => 0,
    }
}
