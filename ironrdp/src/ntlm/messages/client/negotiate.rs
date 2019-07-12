use std::io;

use byteorder::{LittleEndian, WriteBytesExt};

use crate::{
    ntlm::{
        messages::{MessageFields, MessageTypes, NTLM_SIGNATURE, NTLM_VERSION_SIZE},
        NegotiateFlags, NegotiateMessage, Ntlm, NtlmState,
    },
    sspi::{self, SspiError, SspiErrorType},
};

const HEADER_SIZE: usize = 32;
const NEGO_MESSAGE_OFFSET: usize = HEADER_SIZE + NTLM_VERSION_SIZE;

struct NegotiateMessageFields {
    domain_name: MessageFields,
    workstation: MessageFields,
}

impl NegotiateMessageFields {
    pub fn new(offset: u32) -> Self {
        let mut domain_name = MessageFields::new();
        let mut workstation = MessageFields::new();

        domain_name.buffer_offset = offset;
        workstation.buffer_offset = domain_name.buffer_offset + domain_name.buffer.len() as u32;

        NegotiateMessageFields {
            domain_name,
            workstation,
        }
    }

    pub fn data_len(&self) -> usize {
        self.workstation.buffer_offset as usize + self.workstation.buffer.len()
    }
}

fn check_state(state: NtlmState) -> sspi::Result<()> {
    if state != NtlmState::Negotiate {
        Err(SspiError::new(
            SspiErrorType::OutOfSequence,
            String::from("Write negotiate was fired but the state is not a Negotiate"),
        ))
    } else {
        Ok(())
    }
}

pub fn write_negotiate(context: &mut Ntlm, mut transport: impl io::Write) -> sspi::SspiResult {
    check_state(context.state)?;

    let negotiate_flags = get_flags();
    let message_fields = NegotiateMessageFields::new(NEGO_MESSAGE_OFFSET as u32);

    let mut buffer = Vec::with_capacity(message_fields.data_len());

    write_header(
        negotiate_flags,
        context.version.as_ref(),
        &message_fields,
        &mut buffer,
    )?;
    write_payload(&message_fields, &mut buffer)?;
    context.flags = negotiate_flags;

    let message = buffer;

    transport.write_all(message.as_slice())?;
    transport.flush()?;

    context.negotiate_message = Some(NegotiateMessage::new(message));
    context.state = NtlmState::Challenge;

    Ok(sspi::SspiOk::ContinueNeeded)
}

fn get_flags() -> NegotiateFlags {
    // NTLMv2
    NegotiateFlags::NTLM_SSP_NEGOTIATE56
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_LM_KEY
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_OEM
    // ASC_REQ_CONFIDENTIALITY, ISC_REQ_CONFIDENTIALITY always set in the nla
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_SEAL
    // other flags
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_KEY_EXCH
        | NegotiateFlags::NTLM_SSP_NEGOTIATE128
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_EXTENDED_SESSION_SECURITY
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_ALWAYS_SIGN
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_NTLM
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_SIGN
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_REQUEST_TARGET
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_UNICODE
        | NegotiateFlags::NTLM_SSP_NEGOTIATE_VERSION
}

fn write_header(
    negotiate_flags: NegotiateFlags,
    version: &[u8],
    message_fields: &NegotiateMessageFields,
    mut buffer: impl io::Write,
) -> io::Result<()> {
    buffer.write_all(NTLM_SIGNATURE)?; // signature 8 bytes
    buffer.write_u32::<LittleEndian>(MessageTypes::Negotiate as u32)?; // message type 4 bytes
    buffer.write_u32::<LittleEndian>(negotiate_flags.bits())?; // negotiate flags 4 bytes
    message_fields.domain_name.write_to(&mut buffer)?; // domain name 8 bytes
    message_fields.workstation.write_to(&mut buffer)?; // workstation 8 bytes
    buffer.write_all(version)?;

    Ok(())
}

fn write_payload(
    message_fields: &NegotiateMessageFields,
    mut buffer: impl io::Write,
) -> io::Result<()> {
    message_fields.domain_name.write_buffer_to(&mut buffer)?;
    message_fields.workstation.write_buffer_to(&mut buffer)?;

    Ok(())
}
