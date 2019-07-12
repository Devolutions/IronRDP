#[cfg(test)]
mod test;

use std::io::{self, Read, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::{DateTime, TimeZone, Utc};
use lazy_static::lazy_static;
use rand::{rngs::OsRng, Rng};

use crate::{
    encryption::{compute_hmac_md5, compute_md4, compute_md5, HASH_SIZE},
    ntlm::{
        messages::av_pair::*, CHALLENGE_SIZE, LM_CHALLENGE_RESPONSE_BUFFER_SIZE,
        MESSAGE_INTEGRITY_CHECK_SIZE,
    },
    sspi::{self, CredentialsBuffers, SspiError, SspiErrorType},
    utils,
};

pub const SSPI_CREDENTIALS_HASH_LENGTH_OFFSET: usize = 512;
pub const SINGLE_HOST_DATA_SIZE: usize = 48;

const NT_V2_RESPONSE_BASE_SIZE: usize = 28;

// The Single_Host_Data structure allows a client to send machine-specific information
// within an authentication exchange to services on the same machine. The client can
// produce additional information to be processed in an implementation-specific way when
// the client and server are on the same host. If the server and client platforms are
// different or if they are on different hosts, then the information MUST be ignored.
// Any fields after the MachineID field MUST be ignored on receipt.
lazy_static! {
    pub static ref SINGLE_HOST_DATA: [u8; SINGLE_HOST_DATA_SIZE] = {
        let mut result = [0x00; SINGLE_HOST_DATA_SIZE];
        let mut buffer = io::Cursor::new(result.as_mut());

        buffer.write_u32::<LittleEndian>(SINGLE_HOST_DATA_SIZE as u32).unwrap(); //size
        buffer.write_u32::<LittleEndian>(0).unwrap(); //z4
        buffer.write_u32::<LittleEndian>(1).unwrap(); //data present
        buffer.write_u32::<LittleEndian>(0x2000).unwrap(); //custom_data
        buffer.write_all([0xaa; 32].as_ref()).unwrap(); //machine_id

        result
    };
}

pub fn get_system_time_as_file_time<T>(
    start_date: DateTime<T>,
    end_date: DateTime<T>,
) -> sspi::Result<u64>
where
    T: TimeZone,
{
    if start_date > end_date {
        Err(SspiError::new(
            SspiErrorType::InternalError,
            format!(
                "Failed to convert system time to file time, where the start date: {:?}, end date: {:?}",
                start_date, end_date
            ),
        ))
    } else {
        Ok(end_date
            .signed_duration_since(start_date)
            .num_microseconds()
            .expect("System time does not fit to i64") as u64
            * 10)
    }
}

pub fn get_challenge_target_info(timestamp: u64) -> sspi::Result<Vec<u8>> {
    let mut av_pairs: Vec<AvPair> = Vec::with_capacity(6);

    // Windows requires _DomainName, _ComputerName fields, but does not care what they are contain
    av_pairs.push(AvPair::NbDomainName(Vec::new()));
    av_pairs.push(AvPair::NbComputerName(Vec::new()));
    av_pairs.push(AvPair::DnsDomainName(Vec::new()));
    av_pairs.push(AvPair::DnsComputerName(Vec::new()));
    av_pairs.push(AvPair::Timestamp(timestamp));
    av_pairs.push(AvPair::EOL);

    Ok(AvPair::list_to_buffer(&av_pairs)?)
}

pub fn get_authenticate_target_info(
    target_info: &[u8],
    send_single_host_data: bool,
) -> sspi::Result<Vec<u8>> {
    let mut av_pairs = AvPair::buffer_to_av_pairs(&target_info)?;

    av_pairs.retain(|av_pair| av_pair.as_u16() != AV_PAIR_EOL);

    // use_mic always true, when ntlm_v2 is true
    let flags_av_pair = AvPair::Flags(MsvAvFlags::MESSAGE_INTEGRITY_CHECK.bits());
    av_pairs.push(flags_av_pair);

    if send_single_host_data {
        let single_host_av_pair = AvPair::SingleHost(*SINGLE_HOST_DATA);
        av_pairs.push(single_host_av_pair);
    }

    // will not check suppress_extended_protection and
    // will not add channel bindings and service principal name
    // because it is not used anywhere

    let mut authenticate_target_info = AvPair::list_to_buffer(&av_pairs)?;

    // NTLMv2
    // unknown 8-byte padding: AvEOL ([0x00; 4]) + reserved ([0x00; 4])
    authenticate_target_info.write_u64::<LittleEndian>(0x00)?;

    Ok(authenticate_target_info)
}

pub fn generate_challenge() -> Result<[u8; CHALLENGE_SIZE], rand::Error> {
    Ok(OsRng::new()?.gen::<[u8; CHALLENGE_SIZE]>())
}

pub fn generate_timestamp() -> sspi::Result<u64> {
    get_system_time_as_file_time(Utc.ymd(1601, 1, 1).and_hms(0, 1, 1), Utc::now())
}

pub fn generate_signing_key(exported_session_key: &[u8], sign_magic: &[u8]) -> [u8; HASH_SIZE] {
    let mut value = exported_session_key.to_vec();
    value.extend_from_slice(sign_magic);
    compute_md5(value.as_ref())
}

pub fn compute_message_integrity_check(
    negotiate_message: &[u8],
    challenge_message: &[u8],
    authenticate_message: &[u8],
    exported_session_key: &[u8],
) -> io::Result<[u8; MESSAGE_INTEGRITY_CHECK_SIZE]> {
    let mut message_integrity_check = negotiate_message.to_vec();
    message_integrity_check.extend_from_slice(challenge_message);
    message_integrity_check.extend_from_slice(authenticate_message);

    compute_hmac_md5(exported_session_key, message_integrity_check.as_ref())
}

pub fn convert_password_hash(identity_password: &[u8]) -> sspi::Result<[u8; HASH_SIZE]> {
    if identity_password.len() >= SSPI_CREDENTIALS_HASH_LENGTH_OFFSET + HASH_SIZE * 2 {
        let mut result = [0x00; HASH_SIZE];
        let password_hash = &identity_password
            [0..identity_password.len() - SSPI_CREDENTIALS_HASH_LENGTH_OFFSET]
            .to_ascii_uppercase();

        let magic_transform = |elem: u8| {
            if elem > b'9' {
                elem + 10 - b'A'
            } else {
                elem.wrapping_sub(b'0')
            }
        };

        for (hash_items, res) in password_hash.chunks(2).zip(result.iter_mut()) {
            let hn = magic_transform(*hash_items.first().unwrap());
            let ln = magic_transform(*hash_items.last().unwrap());
            *res = (hn << 4) | ln;
        }

        Ok(result)
    } else {
        Err(SspiError::new(
            SspiErrorType::InvalidToken,
            format!(
                "Got password with a small length: {}",
                identity_password.len()
            ),
        ))
    }
}

pub fn compute_ntlm_v2_hash(identity: &CredentialsBuffers) -> sspi::Result<[u8; HASH_SIZE]> {
    if !identity.is_empty() {
        let hmac_key = if identity.password.len() > SSPI_CREDENTIALS_HASH_LENGTH_OFFSET {
            convert_password_hash(&identity.password)?
        } else {
            compute_md4(&identity.password)
        };

        let user_utf16 = utils::bytes_to_utf16_string(identity.user.as_ref());
        let mut user_uppercase_with_domain =
            utils::string_to_utf16(user_utf16.to_uppercase().as_str());
        user_uppercase_with_domain.extend(&identity.domain);

        Ok(compute_hmac_md5(&hmac_key, &user_uppercase_with_domain)?)
    } else {
        Err(SspiError::new(
            SspiErrorType::InvalidToken,
            String::from("Got empty identity"),
        ))
    }
    // hash by the callback is not implemented because the callback never sets
}

pub fn compute_lm_v2_response(
    client_challenge: &[u8],
    server_challenge: &[u8],
    ntlm_v2_hash: &[u8],
) -> sspi::Result<[u8; LM_CHALLENGE_RESPONSE_BUFFER_SIZE]> {
    let mut lm_challenge_data = [0x00; CHALLENGE_SIZE * 2];
    lm_challenge_data[0..CHALLENGE_SIZE].clone_from_slice(server_challenge);
    lm_challenge_data[CHALLENGE_SIZE..].clone_from_slice(client_challenge);

    let mut lm_challenge_response = [0x00; LM_CHALLENGE_RESPONSE_BUFFER_SIZE];
    lm_challenge_response[0..HASH_SIZE]
        .clone_from_slice(compute_hmac_md5(ntlm_v2_hash, &lm_challenge_data)?.as_ref());
    lm_challenge_response[HASH_SIZE..].clone_from_slice(client_challenge);
    Ok(lm_challenge_response)
}

pub fn compute_ntlm_v2_response(
    client_challenge: &[u8],
    server_challenge: &[u8],
    target_info: &[u8],
    ntlm_v2_hash: &[u8],
    timestamp: u64,
) -> sspi::Result<(Vec<u8>, [u8; HASH_SIZE])> {
    let mut ntlm_v2_temp = Vec::with_capacity(NT_V2_RESPONSE_BASE_SIZE);
    ntlm_v2_temp.write_u8(1)?; // RespType 1 byte
    ntlm_v2_temp.write_u8(1)?; // HighRespType 1 byte
    ntlm_v2_temp.write_u16::<LittleEndian>(0)?; // Reserved1 2 bytes
    ntlm_v2_temp.write_u32::<LittleEndian>(0)?; // Reserved2 4 bytes
    ntlm_v2_temp.write_u64::<LittleEndian>(timestamp)?; // Timestamp 8 bytes
    ntlm_v2_temp.extend(client_challenge); // ClientChallenge 8 bytes
    ntlm_v2_temp.write_u32::<LittleEndian>(0)?; // Reserved3 4 bytes
    ntlm_v2_temp.extend(target_info); // TargetInfo

    let mut nt_proof_input = server_challenge.to_vec();
    nt_proof_input.extend(ntlm_v2_temp.as_slice());
    let nt_proof = compute_hmac_md5(ntlm_v2_hash, nt_proof_input.as_ref())?;

    let mut nt_challenge_response = nt_proof.to_vec();
    nt_challenge_response.append(ntlm_v2_temp.as_mut());

    let key_exchange_key = compute_hmac_md5(ntlm_v2_hash, nt_proof.as_ref())?;

    Ok((nt_challenge_response, key_exchange_key))
}

pub fn read_ntlm_v2_response(
    mut challenge_response: &[u8],
) -> io::Result<(Vec<u8>, [u8; CHALLENGE_SIZE])> {
    let mut response = [0x00; HASH_SIZE];
    challenge_response.read_exact(response.as_mut())?;
    let _resp_type = challenge_response.read_u8()?;
    let _hi_resp_type = challenge_response.read_u8()?;
    let _reserved1 = challenge_response.read_u16::<LittleEndian>()?;
    let _reserved2 = challenge_response.read_u32::<LittleEndian>()?;
    let _timestamp = challenge_response.read_u64::<LittleEndian>()?;

    let mut client_challenge = [0x00; CHALLENGE_SIZE];
    challenge_response.read_exact(client_challenge.as_mut())?;
    let _reserved3 = challenge_response.read_u32::<LittleEndian>()?;

    let mut av_pairs = Vec::with_capacity(challenge_response.len());
    challenge_response.read_to_end(&mut av_pairs)?;

    Ok((av_pairs, client_challenge))
}

pub fn get_av_flags_from_response(target_info: &[u8]) -> io::Result<MsvAvFlags> {
    let av_pairs = AvPair::buffer_to_av_pairs(target_info)?;

    if let Some(AvPair::Flags(value)) = av_pairs
        .iter()
        .find(|&av_pair| av_pair.as_u16() == AV_PAIR_FLAGS)
    {
        Ok(MsvAvFlags::from_bits(*value).unwrap_or_else(MsvAvFlags::empty))
    } else {
        Ok(MsvAvFlags::empty())
    }
}

pub fn get_challenge_timestamp_from_response(target_info: &[u8]) -> sspi::Result<u64> {
    let av_pairs = AvPair::buffer_to_av_pairs(target_info)?;

    if let Some(AvPair::Timestamp(value)) = av_pairs
        .iter()
        .find(|&av_pair| av_pair.as_u16() == AV_PAIR_TIMESTAMP)
    {
        Ok(*value)
    } else {
        generate_timestamp()
    }
}
