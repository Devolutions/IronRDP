use bytes::BytesMut;
use futures::StreamExt;
use ironrdp::{nego, PduParsing};
use log::debug;
use sspi::internal::credssp;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::Framed;

use crate::{codecs::RdpFrameCodec, RdpError};

const MAX_TS_REQUEST_LENGTH_BUFFER_SIZE: usize = 4;

#[derive(Default)]
pub struct TsRequestTransport;

impl TsRequestTransport {
    pub async fn encode(
        &mut self,
        ts_request: credssp::TsRequest,
        mut stream: impl AsyncWrite + Unpin,
    ) -> Result<(), RdpError> {
        let mut buf = BytesMut::with_capacity(ts_request.buffer_len() as usize);
        buf.resize(ts_request.buffer_len() as usize, 0x00);

        ts_request
            .encode_ts_request(buf.as_mut())
            .map_err(RdpError::TsRequestError)?;

        stream.write_all(buf.as_ref()).await?;
        stream.flush().await?;

        Ok(())
    }

    pub async fn decode(&mut self, mut stream: impl AsyncRead + Unpin) -> Result<credssp::TsRequest, RdpError> {
        let mut buf = BytesMut::with_capacity(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE);
        buf.resize(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE, 0x00);
        stream.read_exact(&mut buf).await?;

        let ts_request_buffer_length = credssp::TsRequest::read_length(buf.as_ref())?;
        buf.resize(ts_request_buffer_length, 0x00);
        stream.read_exact(&mut buf[MAX_TS_REQUEST_LENGTH_BUFFER_SIZE..]).await?;

        let ts_request = credssp::TsRequest::from_buffer(buf.as_ref()).map_err(RdpError::TsRequestError)?;

        Ok(ts_request)
    }
}

pub struct EarlyUserAuthResult;

impl EarlyUserAuthResult {
    pub async fn read(mut stream: impl AsyncRead + Unpin) -> Result<credssp::EarlyUserAuthResult, RdpError> {
        let mut buf = BytesMut::with_capacity(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE);
        buf.resize(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE, 0x00);
        stream.read_exact(&mut buf).await?;
        let early_user_auth_result =
            credssp::EarlyUserAuthResult::from_buffer(buf.as_ref()).map_err(RdpError::EarlyUserAuthResultError)?;

        Ok(early_user_auth_result)
    }
}

pub async fn connect(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    security_protocol: nego::SecurityProtocol,
    username: String,
) -> Result<nego::SecurityProtocol, RdpError> {
    let selected_protocol = process_negotiation(
        &mut stream,
        Some(nego::NegoData::Cookie(username)),
        security_protocol,
        nego::RequestFlags::empty(),
        0,
    )
    .await?;

    Ok(selected_protocol)
}

async fn process_negotiation(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    nego_data: Option<nego::NegoData>,
    protocol: nego::SecurityProtocol,
    flags: nego::RequestFlags,
    src_ref: u16,
) -> Result<nego::SecurityProtocol, RdpError> {
    let connection_request = nego::Request {
        nego_data,
        flags,
        protocol,
        src_ref,
    };
    debug!("Send X.224 Connection Request PDU: {:?}", connection_request);
    let mut buffer = Vec::new();
    connection_request.to_buffer(&mut buffer)?;
    stream.write_all(buffer.as_slice()).await?;
    stream.flush().await?;

    let mut framed = Framed::new(&mut stream, RdpFrameCodec::default());

    let data = framed.next().await.ok_or(RdpError::AccessDenied)??;
    let connection_response = nego::Response::from_buffer(data.as_ref())?;
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
