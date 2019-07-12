use std::io::{self, Read};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::sspi::CredentialsBuffers;
use crate::{
    ntlm::{
        messages::{
            av_pair::MsvAvFlags, computations::*, read_ntlm_header, try_read_version,
            MessageFields, MessageTypes,
        },
        AuthenticateMessage, Mic, NegotiateFlags, Ntlm, NtlmState,
        ENCRYPTED_RANDOM_SESSION_KEY_SIZE, MESSAGE_INTEGRITY_CHECK_SIZE,
    },
    sspi::{self, SspiError, SspiErrorType},
};

const HEADER_SIZE: usize = 64;

struct AuthenticateMessageFields {
    workstation: MessageFields,
    domain_name: MessageFields,
    encrypted_random_session_key: MessageFields,
    user_name: MessageFields,
    lm_challenge_response: MessageFields,
    nt_challenge_response: MessageFields,
}

pub fn read_authenticate(mut context: &mut Ntlm, mut stream: impl io::Read) -> sspi::SspiResult {
    check_state(context.state)?;

    let mut buffer = Vec::with_capacity(HEADER_SIZE);
    stream.read_to_end(&mut buffer)?;
    let mut buffer = io::Cursor::new(buffer);

    read_ntlm_header(&mut buffer, MessageTypes::Authenticate)?;
    let (mut message_fields, flags) = read_header(&mut buffer)?;
    context.flags = flags;
    let _version = try_read_version(context.flags, &mut buffer)?;
    let mic = read_payload(flags, &mut message_fields, &mut buffer)?;
    let message = buffer.into_inner();

    let (authenticate_message, updated_identity) =
        process_message_fields(&context.identity, message_fields, mic, message)?;
    context.identity = Some(updated_identity);
    context.authenticate_message = Some(authenticate_message);

    context.state = NtlmState::Completion;

    Ok(sspi::SspiOk::CompleteNeeded)
}

fn check_state(state: NtlmState) -> sspi::Result<()> {
    if state != NtlmState::Authenticate {
        Err(SspiError::new(
            SspiErrorType::OutOfSequence,
            String::from("Read authenticate was fired but the state is not an Authenticate"),
        ))
    } else {
        Ok(())
    }
}

fn read_header(
    mut buffer: impl io::Read,
) -> sspi::Result<(AuthenticateMessageFields, NegotiateFlags)> {
    let mut lm_challenge_response = MessageFields::new();
    let mut nt_challenge_response = MessageFields::new();
    let mut domain_name = MessageFields::new();
    let mut user_name = MessageFields::new();
    let mut workstation = MessageFields::new();
    let mut encrypted_random_session_key = MessageFields::new();

    lm_challenge_response.read_from(&mut buffer)?;
    nt_challenge_response.read_from(&mut buffer)?;
    domain_name.read_from(&mut buffer)?;
    user_name.read_from(&mut buffer)?;
    workstation.read_from(&mut buffer)?;
    encrypted_random_session_key.read_from(&mut buffer)?;
    let negotiate_flags = NegotiateFlags::from_bits(buffer.read_u32::<LittleEndian>()?)
        .unwrap_or_else(NegotiateFlags::empty);

    let negotiate_key_exchange =
        negotiate_flags.contains(NegotiateFlags::NTLM_SSP_NEGOTIATE_KEY_EXCH);
    if negotiate_key_exchange && encrypted_random_session_key.buffer.is_empty()
        || !negotiate_key_exchange && !encrypted_random_session_key.buffer.is_empty()
    {
        return Err(SspiError::new(
            SspiErrorType::InvalidToken,
            String::from(
                "Negotiate key exchange flag is set but encrypted random session key \
                 is empty or the flag is not set but the key is not empty",
            ),
        ));
    }

    if encrypted_random_session_key.buffer.len() != ENCRYPTED_RANDOM_SESSION_KEY_SIZE {
        return Err(SspiError::new(
            SspiErrorType::InvalidToken,
            String::from("Invalid encrypted random session key"),
        ));
    }

    let message_fields = AuthenticateMessageFields {
        workstation,
        domain_name,
        encrypted_random_session_key,
        user_name,
        lm_challenge_response,
        nt_challenge_response,
    };

    Ok((message_fields, negotiate_flags))
}

fn read_payload<T>(
    negotiate_flags: NegotiateFlags,
    message_fields: &mut AuthenticateMessageFields,
    mut buffer: &mut io::Cursor<T>,
) -> sspi::Result<Option<Mic>>
where
    io::Cursor<T>: io::Read + io::Seek,
{
    let mic = if negotiate_flags.contains(NegotiateFlags::NTLM_SSP_NEGOTIATE_TARGET_INFO) {
        let mic_offset = buffer.position() as u8;
        let mut mic_value = [0x00; MESSAGE_INTEGRITY_CHECK_SIZE];
        buffer.read_exact(&mut mic_value)?;
        Some(Mic::new(mic_value, mic_offset))
    } else {
        None
    };

    message_fields.domain_name.read_buffer_from(&mut buffer)?;
    message_fields.user_name.read_buffer_from(&mut buffer)?;
    message_fields.workstation.read_buffer_from(&mut buffer)?;
    message_fields
        .lm_challenge_response
        .read_buffer_from(&mut buffer)?;
    message_fields
        .nt_challenge_response
        .read_buffer_from(&mut buffer)?;
    message_fields
        .encrypted_random_session_key
        .read_buffer_from(&mut buffer)?;

    Ok(mic)
}

fn process_message_fields(
    identity: &Option<CredentialsBuffers>,
    message_fields: AuthenticateMessageFields,
    mic: Option<Mic>,
    authenticate_message: Vec<u8>,
) -> sspi::Result<(AuthenticateMessage, CredentialsBuffers)> {
    if message_fields.nt_challenge_response.buffer.is_empty() {
        return Err(SspiError::new(
            SspiErrorType::InvalidToken,
            String::from("NtChallengeResponse cannot be empty"),
        ));
    }
    let (target_info, client_challenge) =
        read_ntlm_v2_response(message_fields.nt_challenge_response.buffer.as_ref())?;
    let mic = if mic.is_some() {
        let challenge_response_av_flags = get_av_flags_from_response(target_info.as_ref())?;
        if challenge_response_av_flags.contains(MsvAvFlags::MESSAGE_INTEGRITY_CHECK) {
            mic
        } else {
            None
        }
    } else {
        None
    };

    // will not set workstation because it is not used anywhere

    let mut encrypted_random_session_key = [0x00; ENCRYPTED_RANDOM_SESSION_KEY_SIZE];
    encrypted_random_session_key
        .clone_from_slice(message_fields.encrypted_random_session_key.buffer.as_ref());

    let mut identity = if let Some(identity) = identity {
        identity.clone()
    } else {
        CredentialsBuffers::default()
    };

    if !message_fields.user_name.buffer.is_empty() {
        identity.user = message_fields.user_name.buffer.clone();
    }
    if !message_fields.domain_name.buffer.is_empty() {
        identity.domain = message_fields.domain_name.buffer.clone();
    }

    Ok((
        AuthenticateMessage::new(
            authenticate_message,
            mic,
            target_info,
            client_challenge,
            encrypted_random_session_key,
        ),
        identity,
    ))
}
