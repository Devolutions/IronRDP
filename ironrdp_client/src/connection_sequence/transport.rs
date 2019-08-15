use std::io;

use bytes::BytesMut;

use crate::{connection_sequence::SERVER_CHANNEL_ID, RdpError, RdpResult};
use ironrdp::PduParsing;

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

fn read_tpkt_tpdu_buffer(mut stream: impl io::Read) -> io::Result<BytesMut> {
    let mut buf = BytesMut::with_capacity(ironrdp::TPKT_HEADER_LENGTH);
    buf.resize(ironrdp::TPKT_HEADER_LENGTH, 0x00);
    stream.read_exact(&mut buf)?;

    let tpkt_header_length = ironrdp::read_tpkt_len(buf.as_ref())? as usize;

    buf.resize(tpkt_header_length, 0x00);
    stream.read_exact(&mut buf[ironrdp::TPKT_HEADER_LENGTH..])?;

    Ok(buf)
}

#[derive(Debug)]
pub struct ConnectionRequest {
    nego_data: Option<ironrdp::NegoData>,
    protocol: ironrdp::SecurityProtocol,
    flags: ironrdp::NegotiationRequestFlags,
}

impl ConnectionRequest {
    pub fn new(
        nego_data: Option<ironrdp::NegoData>,
        protocol: ironrdp::SecurityProtocol,
        flags: ironrdp::NegotiationRequestFlags,
    ) -> Self {
        Self {
            nego_data,
            protocol,
            flags,
        }
    }
    pub fn write(&self, mut stream: impl io::Write) -> RdpResult<()> {
        let cookie = if let Some(ironrdp::NegoData::Cookie(ref cookie)) = self.nego_data {
            cookie
        } else {
            unimplemented!()
        };

        let mut negotiation_request =
            BytesMut::with_capacity(ironrdp::NEGOTIATION_REQUEST_LEN + cookie.len());
        negotiation_request.resize(ironrdp::NEGOTIATION_REQUEST_LEN + cookie.len(), 0x00);
        ironrdp::write_negotiation_request(
            negotiation_request.as_mut(),
            cookie,
            self.protocol,
            self.flags,
        )
        .map_err(RdpError::NegotiationError)?;

        let tpdu_len = ironrdp::tpdu_header_length(ironrdp::X224TPDUType::ConnectionRequest);

        let mut tpdu_buf = BytesMut::with_capacity(tpdu_len);
        tpdu_buf.resize(tpdu_len, 0x00);
        ironrdp::encode_x224(
            ironrdp::X224TPDUType::ConnectionRequest,
            negotiation_request,
            &mut tpdu_buf,
        )
        .map_err(RdpError::X224Error)?;

        stream.write_all(tpdu_buf.as_ref())?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct ConnectionConfirm {
    pub protocol: ironrdp::SecurityProtocol,
    flags: ironrdp::NegotiationResponseFlags,
}

impl ConnectionConfirm {
    pub fn read(mut stream: impl io::Read) -> RdpResult<Self> {
        let mut tpkt_tpdu_buf = read_tpkt_tpdu_buffer(&mut stream)?;
        let (tpdu_type, data) =
            ironrdp::decode_x224(&mut tpkt_tpdu_buf).map_err(RdpError::NegotiationError)?;

        let (protocol, flags) = ironrdp::parse_negotiation_response(tpdu_type, data.as_ref())
            .map_err(|e| RdpError::NegotiationError(io::Error::new(io::ErrorKind::Other, e)))?;

        Ok(Self { protocol, flags })
    }
}

#[derive(Default)]
pub struct DataTransport;

impl Encoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn encode(&mut self, data: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let tpdu_len = ironrdp::tpdu_header_length(ironrdp::X224TPDUType::Data);

        let mut tpdu = BytesMut::with_capacity(tpdu_len);
        tpdu.resize(tpdu_len, 0x00);
        ironrdp::encode_x224(ironrdp::X224TPDUType::Data, data, &mut tpdu)
            .map_err(RdpError::X224Error)?;

        stream.write_all(tpdu.as_ref())?;

        Ok(())
    }
}

impl Decoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let mut tpkt_tpdu_buf = read_tpkt_tpdu_buffer(&mut stream)?;
        let (_tpdu_type, data) =
            ironrdp::decode_x224(&mut tpkt_tpdu_buf).map_err(RdpError::X224Error)?;

        Ok(data)
    }
}

#[derive(Default)]
pub struct TsRequestTransport;

impl Encoder for TsRequestTransport {
    type Item = sspi::TsRequest;
    type Error = RdpError;

    fn encode(&mut self, ts_request: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let mut buf = BytesMut::with_capacity(ts_request.buffer_len() as usize);
        buf.resize(ts_request.buffer_len() as usize, 0x00);

        ts_request
            .encode_ts_request(buf.as_mut())
            .map_err(RdpError::TsRequestError)?;

        stream.write_all(buf.as_ref())?;

        Ok(())
    }
}

impl Decoder for TsRequestTransport {
    type Item = sspi::TsRequest;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let mut buf = BytesMut::with_capacity(sspi::MAX_TS_REQUEST_LENGTH_BUFFER_SIZE);
        buf.resize(sspi::MAX_TS_REQUEST_LENGTH_BUFFER_SIZE, 0x00);
        stream.read_exact(&mut buf)?;

        let ts_request_buffer_length = sspi::TsRequest::read_length(buf.as_ref())?;
        buf.resize(ts_request_buffer_length, 0x00);
        stream.read_exact(&mut buf[sspi::MAX_TS_REQUEST_LENGTH_BUFFER_SIZE..])?;

        let ts_request =
            sspi::TsRequest::from_buffer(buf.as_ref()).map_err(RdpError::TsRequestError)?;

        Ok(ts_request)
    }
}

pub struct EarlyUserAuthResult;

impl EarlyUserAuthResult {
    pub fn read(mut stream: impl io::Read) -> RdpResult<sspi::EarlyUserAuthResult> {
        let mut buf = BytesMut::with_capacity(sspi::EARLY_USER_AUTH_RESULT_PDU_SIZE);
        buf.resize(sspi::EARLY_USER_AUTH_RESULT_PDU_SIZE, 0x00);
        stream.read_exact(&mut buf)?;
        let early_user_auth_result = sspi::EarlyUserAuthResult::from_buffer(buf.as_ref())
            .map_err(RdpError::EarlyUserAuthResultError)?;

        Ok(early_user_auth_result)
    }
}

pub struct McsConnectInitial(pub ironrdp::ConnectInitial);

impl McsConnectInitial {
    pub fn write(&self, mut stream: impl io::Write) -> RdpResult<()> {
        let mut connect_initial_buf = BytesMut::with_capacity(self.0.buffer_length() as usize);
        connect_initial_buf.resize(self.0.buffer_length(), 0x00);
        self.0
            .to_buffer(connect_initial_buf.as_mut())
            .map_err(RdpError::McsConnectError)?;

        DataTransport::default().encode(connect_initial_buf, &mut stream)
    }
}

pub struct McsConnectResponse(pub ironrdp::ConnectResponse);

impl McsConnectResponse {
    pub fn read(mut stream: impl io::Read) -> RdpResult<Self> {
        let data = DataTransport::default().decode(&mut stream)?;

        let connect_response = ironrdp::ConnectResponse::from_buffer(data.as_ref())
            .map_err(RdpError::McsConnectError)?;

        Ok(Self(connect_response))
    }
}

#[derive(Default)]
pub struct McsTransport(DataTransport);

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
