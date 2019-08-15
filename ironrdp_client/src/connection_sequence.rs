mod transport;
mod user_info;

pub use transport::{
    ConnectionConfirm, ConnectionRequest, EarlyUserAuthResult, McsConnectInitial,
    McsConnectResponse, McsTransport, SendDataContextTransport, ShareControlHeaderTransport,
    ShareDataHeaderTransport, TsRequestTransport,
};

use std::{collections::HashMap, io, iter};

use ironrdp::PduParsing;
use lazy_static::lazy_static;
use log::debug;
use native_tls::TlsStream;
use sspi::CredSsp;

use crate::connection_sequence::transport::{Decoder, Encoder};
use crate::{config::Config, utils, RdpError, RdpResult};

pub type StaticChannels = HashMap<String, u16>;

lazy_static! {
    pub static ref GLOBAL_CHANNEL_NAME: String = String::from("GLOBAL");
    pub static ref USER_CHANNEL_NAME: String = String::from("USER");
}

const SERVER_CHANNEL_ID: u16 = 0x03ea;

pub fn process_negotiation<S>(
    mut stream: &mut S,
    cookie: String,
    request_protocols: ironrdp::SecurityProtocol,
    request_flags: ironrdp::NegotiationRequestFlags,
) -> RdpResult<ironrdp::SecurityProtocol>
where
    S: io::Read + io::Write,
{
    let connection_request = ConnectionRequest::new(
        Some(ironrdp::NegoData::Cookie(cookie)),
        request_protocols,
        request_flags,
    );
    debug!(
        "Send X.224 Connection Request PDU: {:?}",
        connection_request
    );
    connection_request.write(&mut stream)?;

    let connection_confirm = ConnectionConfirm::read(&mut stream)?;
    debug!("Got X.224 Connection Confirm PDU: {:?}", connection_confirm);
    let selected_protocol = connection_confirm.protocol;

    if request_protocols.contains(selected_protocol) {
        Ok(selected_protocol)
    } else {
        Err(RdpError::InvalidResponse(format!(
            "Got unexpected security protocol: {:?} while was expected one of {:?}",
            selected_protocol, request_protocols
        )))
    }
}

pub fn process_cred_ssp<S>(
    mut tls_stream: &mut TlsStream<S>,
    credentials: sspi::Credentials,
) -> RdpResult<()>
where
    S: io::Read + io::Write,
{
    let server_tls_pubkey = utils::get_tls_peer_pubkey(&tls_stream)?;
    let mut transport = TsRequestTransport::default();

    let mut cred_ssp_client = sspi::CredSspClient::with_default_version(
        server_tls_pubkey,
        credentials,
        sspi::CredSspMode::WithCredentials,
    )
    .map_err(RdpError::CredSspError)?;
    let mut next_ts_request = sspi::TsRequest::default();

    loop {
        let result = cred_ssp_client
            .process(next_ts_request.clone())
            .map_err(RdpError::CredSspError)?;
        debug!("Got CredSSP TSRequest: {:x?}", result);

        match result {
            sspi::CredSspResult::ReplyNeeded(ts_request) => {
                debug!("Send CredSSP TSRequest: {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream)?;

                next_ts_request = transport.decode(&mut tls_stream)?;
            }
            sspi::CredSspResult::FinalMessage(ts_request) => {
                debug!("Send CredSSP TSRequest: {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream)?;

                break;
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}
