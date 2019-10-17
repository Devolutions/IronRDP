use std::io;

use bytes::BytesMut;
use ironrdp::{nego, PduParsing};
use log::{debug, warn};

use crate::{connection_sequence::SERVER_CHANNEL_ID, RdpError, RdpResult};

const MAX_TS_REQUEST_LENGTH_BUFFER_SIZE: usize = 4;

pub trait Encoder {
    type Item;
    type Error;

    fn encode(&mut self, item: Self::Item, stream: impl io::Write) -> Result<(), Self::Error>;
}

pub trait Decoder {
    type Item;
    type Error;

    fn decode(&mut self, stream: impl io::Read) -> Result<Self::Item, Self::Error>;
}

#[derive(Default)]
pub struct DataTransport;

impl DataTransport {
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

        Ok((Self, selected_protocol))
    }
}

impl Encoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn encode(&mut self, data: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        ironrdp::Data::new(data.len()).to_buffer(&mut stream)?;
        stream.write_all(data.as_ref())?;
        stream.flush()?;

        Ok(())
    }
}

impl Decoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let data_pdu = ironrdp::Data::from_buffer(&mut stream)?;

        let mut data = BytesMut::with_capacity(data_pdu.data_length);
        data.resize(data_pdu.data_length, 0x00);
        stream.read_exact(&mut data)?;

        Ok(data)
    }
}

#[derive(Default)]
pub struct TsRequestTransport;

impl Encoder for TsRequestTransport {
    type Item = sspi::internal::TsRequest;
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
    type Item = sspi::internal::TsRequest;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let mut buf = BytesMut::with_capacity(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE);
        buf.resize(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE, 0x00);
        stream.read_exact(&mut buf)?;

        let ts_request_buffer_length = sspi::internal::TsRequest::read_length(buf.as_ref())?;
        buf.resize(ts_request_buffer_length, 0x00);
        stream.read_exact(&mut buf[MAX_TS_REQUEST_LENGTH_BUFFER_SIZE..])?;

        let ts_request = sspi::internal::TsRequest::from_buffer(buf.as_ref())
            .map_err(RdpError::TsRequestError)?;

        Ok(ts_request)
    }
}

pub struct EarlyUserAuthResult;

impl EarlyUserAuthResult {
    pub fn read(mut stream: impl io::Read) -> RdpResult<sspi::internal::EarlyUserAuthResult> {
        let mut buf = BytesMut::with_capacity(sspi::internal::EARLY_USER_AUTH_RESULT_PDU_SIZE);
        buf.resize(sspi::internal::EARLY_USER_AUTH_RESULT_PDU_SIZE, 0x00);
        stream.read_exact(&mut buf)?;
        let early_user_auth_result = sspi::internal::EarlyUserAuthResult::from_buffer(buf.as_ref())
            .map_err(RdpError::EarlyUserAuthResultError)?;

        Ok(early_user_auth_result)
    }
}

pub struct McsTransport(DataTransport);

impl McsTransport {
    pub fn new(transport: DataTransport) -> Self {
        Self(transport)
    }
}

impl Encoder for McsTransport {
    type Item = ironrdp::McsPdu;
    type Error = RdpError;

    fn encode(&mut self, mcs_pdu: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let mut mcs_pdu_buf = BytesMut::with_capacity(mcs_pdu.buffer_length() as usize);
        mcs_pdu_buf.resize(mcs_pdu.buffer_length(), 0x00);
        mcs_pdu
            .to_buffer(mcs_pdu_buf.as_mut())
            .map_err(RdpError::McsError)?;

        self.0.encode(mcs_pdu_buf, &mut stream)
    }
}

impl Decoder for McsTransport {
    type Item = ironrdp::McsPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let data = self.0.decode(&mut stream)?;

        let mcs_pdu = ironrdp::McsPdu::from_buffer(data.as_ref()).map_err(RdpError::McsError)?;

        Ok(mcs_pdu)
    }
}

pub struct SendDataContextTransport {
    mcs_transport: McsTransport,
    initiator_id: u16,
    global_channel_id: u16,
}

impl SendDataContextTransport {
    pub fn new(mcs_transport: McsTransport, initiator_id: u16, global_channel_id: u16) -> Self {
        Self {
            mcs_transport,
            initiator_id,
            global_channel_id,
        }
    }
}

impl Encoder for SendDataContextTransport {
    type Item = Vec<u8>;
    type Error = RdpError;

    fn encode(
        &mut self,
        send_data_context_pdu: Self::Item,
        mut stream: impl io::Write,
    ) -> RdpResult<()> {
        let send_data_context = ironrdp::mcs::SendDataContext::new(
            send_data_context_pdu,
            self.initiator_id,
            self.global_channel_id,
        );
        let send_data_request = ironrdp::McsPdu::SendDataRequest(send_data_context);

        self.mcs_transport.encode(send_data_request, &mut stream)
    }
}

impl Decoder for SendDataContextTransport {
    type Item = Vec<u8>;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let mcs_pdu = self.mcs_transport.decode(&mut stream)?;

        if let ironrdp::McsPdu::SendDataIndication(send_data_context) = mcs_pdu {
            if send_data_context.channel_id == self.global_channel_id {
                Ok(send_data_context.pdu)
            } else {
                Err(RdpError::InvalidResponse(format!(
                    "Unexpected Send Data Context channel ID ({})",
                    send_data_context.channel_id,
                )))
            }
        } else {
            Err(RdpError::UnexpectedPdu(format!(
                "Expected Send Data Context PDU, got {:?}",
                mcs_pdu.as_short_name()
            )))
        }
    }
}

pub struct ShareControlHeaderTransport {
    share_id: u32,
    pdu_source: u16,
    send_data_context_transport: SendDataContextTransport,
}

impl ShareControlHeaderTransport {
    pub fn new(send_data_context_transport: SendDataContextTransport, pdu_source: u16) -> Self {
        Self {
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
        let send_data_context = self.send_data_context_transport.decode(&mut stream)?;

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
