use std::io;

use bytes::BytesMut;
use ironrdp::{nego, rdp::SERVER_CHANNEL_ID, PduParsing};
use log::{debug, warn};
use sspi::internal::credssp;

use super::{DataTransport, Decoder, Encoder, SendDataContextTransport};
use crate::{RdpError, RdpResult};

const MAX_TS_REQUEST_LENGTH_BUFFER_SIZE: usize = 4;

#[derive(Default)]
pub struct TsRequestTransport;

impl Encoder for TsRequestTransport {
    type Item = credssp::TsRequest;
    type Error = RdpError;

    fn encode(&mut self, ts_request: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let mut buf = BytesMut::with_capacity(ts_request.buffer_len() as usize);
        buf.resize(ts_request.buffer_len() as usize, 0x00);

        ts_request
            .encode_ts_request(buf.as_mut())
            .map_err(RdpError::TsRequestError)?;

        stream.write_all(buf.as_ref())?;
        stream.flush()?;

        Ok(())
    }
}

impl Decoder for TsRequestTransport {
    type Item = credssp::TsRequest;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let mut buf = BytesMut::with_capacity(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE);
        buf.resize(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE, 0x00);
        stream.read_exact(&mut buf)?;

        let ts_request_buffer_length = credssp::TsRequest::read_length(buf.as_ref())?;
        buf.resize(ts_request_buffer_length, 0x00);
        stream.read_exact(&mut buf[MAX_TS_REQUEST_LENGTH_BUFFER_SIZE..])?;

        let ts_request =
            credssp::TsRequest::from_buffer(buf.as_ref()).map_err(RdpError::TsRequestError)?;

        Ok(ts_request)
    }
}

pub struct EarlyUserAuthResult;

impl EarlyUserAuthResult {
    pub fn read(mut stream: impl io::Read) -> RdpResult<credssp::EarlyUserAuthResult> {
        let mut buf = BytesMut::with_capacity(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE);
        buf.resize(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE, 0x00);
        stream.read_exact(&mut buf)?;
        let early_user_auth_result = credssp::EarlyUserAuthResult::from_buffer(buf.as_ref())
            .map_err(RdpError::EarlyUserAuthResultError)?;

        Ok(early_user_auth_result)
    }
}

pub struct ShareControlHeaderTransport {
    global_channel_id: u16,
    share_id: u32,
    pdu_source: u16,
    send_data_context_transport: SendDataContextTransport,
}

impl ShareControlHeaderTransport {
    pub fn new(
        send_data_context_transport: SendDataContextTransport,
        pdu_source: u16,
        global_channel_id: u16,
    ) -> Self {
        Self {
            global_channel_id,
            send_data_context_transport,
            pdu_source,
            share_id: 0,
        }
    }
}

impl Encoder for ShareControlHeaderTransport {
    type Item = ironrdp::ShareControlPdu;
    type Error = RdpError;

    fn encode(
        &mut self,
        share_control_pdu: Self::Item,
        mut stream: impl io::Write,
    ) -> RdpResult<()> {
        let share_control_header =
            ironrdp::ShareControlHeader::new(share_control_pdu, self.pdu_source, self.share_id);

        let mut pdu = Vec::with_capacity(share_control_header.buffer_length());
        share_control_header
            .to_buffer(&mut pdu)
            .map_err(RdpError::ShareControlHeaderError)?;

        self.send_data_context_transport.encode(pdu, &mut stream)
    }
}

impl Decoder for ShareControlHeaderTransport {
    type Item = ironrdp::ShareControlPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let (channel_ids, send_data_context) =
            self.send_data_context_transport.decode(&mut stream)?;
        if channel_ids.channel_id != self.global_channel_id {
            return Err(RdpError::InvalidResponse(format!(
                "Unexpected Send Data Context channel ID ({})",
                channel_ids.channel_id,
            )));
        }

        let share_control_header =
            ironrdp::ShareControlHeader::from_buffer(send_data_context.as_slice())
                .map_err(RdpError::ShareControlHeaderError)?;
        self.share_id = share_control_header.share_id;

        if share_control_header.pdu_source != SERVER_CHANNEL_ID {
            warn!(
                "Invalid Share Control Header pdu source: expected ({}) != actual ({})",
                SERVER_CHANNEL_ID, share_control_header.pdu_source
            );
        }

        Ok(share_control_header.share_control_pdu)
    }
}

pub struct ShareDataHeaderTransport(ShareControlHeaderTransport);

impl ShareDataHeaderTransport {
    pub fn new(transport: ShareControlHeaderTransport) -> Self {
        Self(transport)
    }
}

impl Encoder for ShareDataHeaderTransport {
    type Item = ironrdp::ShareDataPdu;
    type Error = RdpError;

    fn encode(&mut self, share_data_pdu: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let share_data_header = ironrdp::ShareDataHeader::new(
            share_data_pdu,
            ironrdp::rdp::StreamPriority::Medium,
            ironrdp::rdp::CompressionFlags::empty(),
            ironrdp::rdp::CompressionType::K8, // ignored if CompressionFlags::empty()
        );

        self.0.encode(
            ironrdp::ShareControlPdu::Data(share_data_header),
            &mut stream,
        )
    }
}

impl Decoder for ShareDataHeaderTransport {
    type Item = ironrdp::ShareDataPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let share_control_pdu = self.0.decode(&mut stream)?;

        if let ironrdp::ShareControlPdu::Data(share_data_header) = share_control_pdu {
            Ok(share_data_header.share_data_pdu)
        } else {
            Err(RdpError::UnexpectedPdu(format!(
                "Expected Share Data Header, got: {:?}",
                share_control_pdu.as_short_name()
            )))
        }
    }
}

pub fn connect(
    mut stream: impl io::BufRead + io::Write,
    security_protocol: nego::SecurityProtocol,
    username: String,
) -> RdpResult<(DataTransport, nego::SecurityProtocol)> {
    let selected_protocol = process_negotiation(
        &mut stream,
        Some(nego::NegoData::Cookie(username)),
        security_protocol,
        nego::RequestFlags::empty(),
        0,
    )?;

    Ok((DataTransport, selected_protocol))
}

fn process_negotiation(
    mut stream: impl io::BufRead + io::Write,
    nego_data: Option<nego::NegoData>,
    protocol: nego::SecurityProtocol,
    flags: nego::RequestFlags,
    src_ref: u16,
) -> RdpResult<nego::SecurityProtocol> {
    let connection_request = nego::Request {
        nego_data,
        flags,
        protocol,
        src_ref,
    };
    debug!(
        "Send X.224 Connection Request PDU: {:?}",
        connection_request
    );
    connection_request.to_buffer(&mut stream)?;
    stream.flush()?;

    let connection_response = nego::Response::from_buffer(&mut stream)?;
    if let Some(nego::ResponseData::Response {
        flags,
        protocol: selected_protocol,
    }) = connection_response.response
    {
        debug!(
            "Got X.224 Connection Confirm PDU: selected protocol ({:?}), response flags ({:?})",
            selected_protocol, flags
        );

        if protocol.contains(selected_protocol) {
            Ok(selected_protocol)
        } else {
            Err(RdpError::InvalidResponse(format!(
                "Got unexpected security protocol: {:?} while was expected one of {:?}",
                selected_protocol, protocol
            )))
        }
    } else {
        Err(RdpError::InvalidResponse(format!(
            "Got unexpected X.224 Connection Response: {:?}",
            connection_response.response
        )))
    }
}
