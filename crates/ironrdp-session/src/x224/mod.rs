use ironrdp_bulk::BulkCompressor;
use ironrdp_core::{Decode as _, ReadCursor, WriteBuf, decode};
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DynamicChannelMut, DynamicChannelRef};
use ironrdp_pdu::gcc::{ChannelName, Monitor};
use ironrdp_pdu::mcs::{DisconnectProviderUltimatum, DisconnectReason, McsMessage, SendDataIndicationCtx};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu};
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::headers::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, CompressionFlags, IoChannelPdu, ShareDataCtx, ShareDataPdu,
};
use ironrdp_pdu::rdp::heartbeat::HeartbeatPdu;
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::rdp::session_info::{InfoData, SaveSessionInfoPdu, ServerAutoReconnect};
use ironrdp_pdu::x224::X224;
use ironrdp_svc::{
    StaticChannelSet, SvcMessage, SvcProcessor, SvcProcessorMessages, client_encode_svc_messages_with_max_chunk_len,
};
use tracing::debug;

use crate::{SessionError, SessionErrorExt as _, SessionResult, reason_err};

/// X224 Processor output
#[derive(Debug, Clone)]
pub enum ProcessorOutput {
    /// A buffer with encoded data to send to the server.
    ResponseFrame(Vec<u8>),
    /// A graceful disconnect notification. Client should close the connection upon receiving this.
    Disconnect(DisconnectDescription),
    /// Received a [`ironrdp_pdu::rdp::headers::ServerDeactivateAll`] PDU. Client should execute the
    /// [Deactivation-Reactivation Sequence].
    ///
    /// [Deactivation-Reactivation Sequence]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    DeactivateAll,
    /// Server Save Session Info notification.
    ///
    /// `logon_complete` is only set for PDU variants that unambiguously report a completed
    /// logon; the source PDU is not retained because it can contain user details and
    /// auto-reconnect cookies.
    SaveSessionInfo { logon_complete: bool },
    /// Server Initiate Multitransport Request. The application should establish a
    /// sideband UDP transport using the request ID and security cookie, then send
    /// a [`MultitransportResponsePdu`] back on the message channel.
    ///
    /// See [\[MS-RDPBCGR\] 2.2.15.1].
    ///
    /// [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
    /// [`MultitransportResponsePdu`]: ironrdp_pdu::rdp::multitransport::MultitransportResponsePdu
    MultitransportRequest(MultitransportRequestPdu),
    /// Server Auto-Reconnect Cookie from a Save Session Info PDU
    /// ([\[MS-RDPBCGR\] 2.2.4.2]).
    ///
    /// The client should hold onto this and pass it to
    /// `ClientConnector::with_auto_reconnect_cookie` if the connection drops
    /// ungracefully, which lets the server reattach the session without asking
    /// for credentials again ([\[MS-RDPBCGR\] 1.3.1.5]).
    ///
    /// The server replaces the cookie whenever a client connects and again at
    /// hourly intervals (MS-RDPBCGR 3.3.6.2, Auto-Reconnect Cookie Update), so
    /// this can arrive more than once in a session; keep the most recent and
    /// discard the previous one.
    ///
    /// [\[MS-RDPBCGR\] 2.2.4.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18f4f605-0ee3-4175-8a62-cf8775252547
    /// [\[MS-RDPBCGR\] 1.3.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/15b0d1c9-2891-4adb-a45e-deb4aeeeab7c
    AutoReconnectCookie(ServerAutoReconnect),
    /// Server rejected a Client Auto-Reconnect Packet ([\[MS-RDPBCGR\] 2.2.4.1]).
    ///
    /// The client must discard its cookie and not report the reconnect as successful.
    ///
    /// [\[MS-RDPBCGR\] 2.2.4.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/5073f4ed-1e93-45e1-b039-6e30c385867c
    AutoReconnectFailed,
    /// Auto-detect network characteristics from server ([\[MS-RDPBCGR\] 2.2.14]).
    ///
    /// Currently only surfaces [`AutoDetectRequest::NetworkCharacteristicsResult`].
    /// RTT requests are handled internally with automatic responses.
    ///
    /// [\[MS-RDPBCGR\] 2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38
    AutoDetect(AutoDetectRequest),
    /// Slow-path graphics update ([MS-RDPBCGR] 2.2.9.1.1.3).
    /// Raw update payload starting with `updateType(u16)`.
    GraphicsUpdate(Vec<u8>),
    /// Slow-path pointer update ([MS-RDPBCGR] 2.2.9.1.1.4).
    /// Raw pointer payload starting with `messageType(u16) + pad(u16)`.
    PointerUpdate(Vec<u8>),
    /// Server-reported remote monitor layout ([MS-RDPBCGR] 2.2.12.1).
    ///
    /// The server may send this after activation when the client advertises
    /// `RNS_UD_CS_SUPPORT_MONITOR_LAYOUT_PDU`.
    MonitorLayout(Vec<Monitor>),
}

#[derive(Debug, Clone)]
pub enum DisconnectDescription {
    /// Includes the reason from the MCS Disconnect Provider Ultimatum.
    /// This is the least-specific disconnect reason and is only used
    /// when a more specific disconnect code is not available.
    McsDisconnect(DisconnectReason),

    /// Includes the error information sent by the RDP server when there
    /// is a connection or disconnection failure.
    ErrorInfo(ErrorInfo),
}

pub struct Processor {
    static_channels: StaticChannelSet,
    user_channel_id: u16,
    io_channel_id: u16,
    message_channel_id: Option<u16>,
    share_id: u32,
}

impl Processor {
    pub fn new(
        static_channels: StaticChannelSet,
        user_channel_id: u16,
        io_channel_id: u16,
        message_channel_id: Option<u16>,
        share_id: u32,
    ) -> Self {
        Self {
            static_channels,
            user_channel_id,
            io_channel_id,
            message_channel_id,
            share_id,
        }
    }

    pub fn set_share_id(&mut self, share_id: u32) {
        self.share_id = share_id;
    }

    /// Updates the negotiated maximum payload length of outgoing static virtual channel chunks.
    pub fn set_static_channel_chunk_size(&mut self, maximum_chunk_size: usize) -> bool {
        self.static_channels.set_maximum_chunk_size(maximum_chunk_size)
    }

    /// Returns the negotiated maximum payload length of outgoing static virtual channel chunks.
    pub fn static_channel_chunk_size(&self) -> usize {
        self.static_channels.maximum_chunk_size()
    }

    pub fn get_svc_processor<T: SvcProcessor + 'static>(&self) -> Option<&T> {
        self.static_channels
            .get_by_type::<T>()
            .and_then(|svc| svc.channel_processor_downcast_ref())
    }

    pub fn get_svc_processor_mut<T: SvcProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.static_channels
            .get_by_type_mut::<T>()
            .and_then(|svc| svc.channel_processor_downcast_mut())
    }

    /// Completes user's SVC request with data, required to sent it over the network and returns
    /// a buffer with encoded data.
    pub fn process_svc_processor_messages<C: SvcProcessor + 'static>(
        &self,
        messages: SvcProcessorMessages<C>,
    ) -> SessionResult<Vec<u8>> {
        let channel_id = self
            .static_channels
            .get_channel_id_by_type::<C>()
            .ok_or_else(|| reason_err!("SVC", "channel not found"))?;

        process_svc_messages(
            messages.into(),
            channel_id,
            self.user_channel_id,
            self.static_channels.maximum_chunk_size(),
        )
    }

    /// Completes an SVC request for a runtime-defined channel name.
    pub fn process_svc_messages_by_name(
        &self,
        channel_name: &ChannelName,
        messages: Vec<SvcMessage>,
    ) -> SessionResult<Vec<u8>> {
        let channel_id = self
            .static_channels
            .get_channel_id_by_channel_name(channel_name)
            .ok_or_else(|| reason_err!("SVC", "channel not found"))?;

        process_svc_messages(
            messages,
            channel_id,
            self.user_channel_id,
            self.static_channels.maximum_chunk_size(),
        )
    }

    pub fn get_dvc<T: DvcClientProcessor + 'static>(&self) -> Option<DynamicChannelRef<'_, T>> {
        self.get_svc_processor::<DrdynvcClient>()?.get_dvc::<T>()
    }

    pub fn get_dvc_mut<T: DvcClientProcessor + 'static>(&mut self) -> Option<DynamicChannelMut<'_, T>> {
        self.get_svc_processor_mut::<DrdynvcClient>()?.get_dvc_mut::<T>()
    }

    pub fn get_dvc_by_channel_id<T: DvcClientProcessor + 'static>(
        &self,
        channel_id: u32,
    ) -> Option<DynamicChannelRef<'_, T>> {
        self.get_svc_processor::<DrdynvcClient>()?
            .get_dvc_by_channel_id(channel_id)
    }

    /// Processes a received PDU. Returns a vector of [`ProcessorOutput`] that must be processed
    /// in the returned order.
    pub fn process(
        &mut self,
        frame: &[u8],
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<ProcessorOutput>> {
        let data_ctx: SendDataIndicationCtx<'_> = match ironrdp_pdu::mcs::decode_send_data_indication(frame) {
            Ok(data_ctx) => data_ctx,
            Err(error) => {
                // Some servers (xrdp) end the session with a plain MCS Disconnect Provider Ultimatum.
                if let Ok(X224(McsMessage::DisconnectProviderUltimatum(ultimatum))) =
                    decode::<X224<McsMessage<'_>>>(frame)
                {
                    debug!(reason = ?ultimatum.reason, "Received Disconnect Provider Ultimatum, session will be closed");

                    return Ok(vec![ProcessorOutput::Disconnect(DisconnectDescription::McsDisconnect(
                        ultimatum.reason,
                    ))]);
                }

                return Err(SessionError::decode(error));
            }
        };
        let channel_id = data_ctx.channel_id;

        if channel_id == self.io_channel_id {
            self.process_io_channel_data_indication(data_ctx, bulk_decompressor)
        } else if self.message_channel_id == Some(channel_id) {
            self.process_message_channel(data_ctx)
        } else {
            let maximum_chunk_size = self.static_channels.maximum_chunk_size();
            if let Some(svc) = self.static_channels.get_by_channel_id_mut(channel_id) {
                let response_pdus = svc.process(data_ctx.user_data).map_err(SessionError::pdu)?;
                process_svc_messages(response_pdus, channel_id, data_ctx.initiator_id, maximum_chunk_size)
                    .map(|data| vec![ProcessorOutput::ResponseFrame(data)])
            } else {
                Err(reason_err!("X224", "unexpected channel received: ID {channel_id}"))
            }
        }
    }

    fn process_io_channel_data_indication(
        &mut self,
        data_ctx: SendDataIndicationCtx<'_>,
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<ProcessorOutput>> {
        debug_assert_eq!(data_ctx.channel_id, self.io_channel_id);

        // Multitransport PDUs use BasicSecurityHeader, so the first two bytes are flags
        // rather than Share Control totalLength. Delegate before walking concatenated PDUs.
        if matches!(
            ironrdp_pdu::rdp::headers::decode_io_channel(data_ctx),
            Ok(IoChannelPdu::MultitransportRequest(_))
        ) {
            return self.process_io_channel(data_ctx, bulk_decompressor);
        }

        let mut outputs = Vec::new();
        let mut offset = 0usize;
        let data = data_ctx.user_data;
        while offset < data.len() {
            if offset + 2 > data.len() {
                return Err(reason_err!("X224", "truncated Share Control PDU length"));
            }

            let total_length = usize::from(u16::from_le_bytes([data[offset], data[offset + 1]]));
            if total_length == 0 || offset + total_length > data.len() {
                if offset == 0 {
                    return self.process_io_channel(data_ctx, bulk_decompressor);
                }
                return Err(reason_err!(
                    "X224",
                    "invalid concatenated Share Control PDU length: {total_length}"
                ));
            }

            let part_ctx = SendDataIndicationCtx {
                initiator_id: data_ctx.initiator_id,
                channel_id: data_ctx.channel_id,
                user_data: &data[offset..offset + total_length],
            };
            outputs.extend(self.process_io_channel(part_ctx, bulk_decompressor)?);
            offset += total_length;
        }

        Ok(outputs)
    }

    fn process_io_channel(
        &mut self,
        data_ctx: SendDataIndicationCtx<'_>,
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<ProcessorOutput>> {
        debug_assert_eq!(data_ctx.channel_id, self.io_channel_id);

        let io_channel = ironrdp_pdu::rdp::headers::decode_io_channel(data_ctx).map_err(SessionError::decode)?;

        match io_channel {
            IoChannelPdu::Data(ctx) => Self::process_share_data(ctx, bulk_decompressor),
            IoChannelPdu::MultitransportRequest(pdu) => {
                debug!(
                    request_id = pdu.request_id,
                    "Ignoring Initiate Multitransport Request received outside the MCS message channel"
                );
                Ok(Vec::new())
            }
            IoChannelPdu::DeactivateAll(_) => Ok(vec![ProcessorOutput::DeactivateAll]),
        }
    }

    fn process_share_data(
        ctx: ShareDataCtx,
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<ProcessorOutput>> {
        let ShareDataCtx {
            compression_flags,
            compression_type,
            pdu,
            ..
        } = ctx;
        let (pdu, compression_flags) = match pdu {
            ShareDataPdu::Compressed { pdu_type, data } => {
                let data = Self::decompress_share_data(data, compression_flags, compression_type, bulk_decompressor)?;
                (
                    ShareDataPdu::decode_with_type(&data, pdu_type).map_err(SessionError::decode)?,
                    CompressionFlags::empty(),
                )
            }
            pdu => (pdu, compression_flags),
        };

        match pdu {
            ShareDataPdu::SaveSessionInfo(session_info) => {
                debug!("Got Session Save Info PDU: {session_info:?}");
                let mut outputs = vec![ProcessorOutput::SaveSessionInfo {
                    logon_complete: is_logon_complete(&session_info),
                }];

                // Surface the auto-reconnect cookie alongside the logon status so
                // the consumer can keep it for a later reconnect. Both come out of
                // this one PDU and neither supersedes the other.
                if let InfoData::LogonExtended(extended) = &session_info.info_data {
                    if let Some(cookie) = &extended.auto_reconnect {
                        outputs.push(ProcessorOutput::AutoReconnectCookie(cookie.clone()));
                    }
                }

                Ok(outputs)
            }
            ShareDataPdu::ArcStatusPdu(status) => {
                if status != [0; 4] {
                    return Err(reason_err!("IO channel", "invalid auto-reconnect status PDU"));
                }

                Ok(vec![ProcessorOutput::AutoReconnectFailed])
            }
            // FIXME: workaround fix to not terminate the session on "unhandled PDU: Set Keyboard Indicators PDU"
            ShareDataPdu::SetKeyboardIndicators(data) => {
                debug!("Got Keyboard Indicators PDU: {data:?}");
                Ok(Vec::new())
            }
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                ProtocolIndependentCode::None,
            ))) => {
                debug!("Received None server error");
                Ok(Vec::new())
            }
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e)) => {
                // This is a part of server-side graceful disconnect procedure defined
                // in [MS-RDPBCGR].
                //
                // [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/149070b0-ecec-4c20-af03-934bbc48adb8
                let desc = DisconnectDescription::ErrorInfo(e);
                Ok(vec![ProcessorOutput::Disconnect(desc)])
            }
            ShareDataPdu::ShutdownDenied => {
                debug!("ShutdownDenied received, session will be closed");

                // As defined in [MS-RDPBCGR], when `ShareDataPdu::ShutdownDenied` is received, we
                // need to send a disconnect ultimatum to the server if we want to proceed with the
                // session shutdown.
                //
                // [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
                let ultimatum = McsMessage::DisconnectProviderUltimatum(DisconnectProviderUltimatum::from_reason(
                    DisconnectReason::UserRequested,
                ));

                let encoded_pdu = ironrdp_core::encode_vec(&X224(ultimatum)).map_err(SessionError::encode);

                Ok(vec![
                    ProcessorOutput::ResponseFrame(encoded_pdu?),
                    ProcessorOutput::Disconnect(DisconnectDescription::McsDisconnect(DisconnectReason::UserRequested)),
                ])
            }
            ShareDataPdu::Update(data) => {
                let data = Self::decompress_share_data(data, compression_flags, compression_type, bulk_decompressor)?;
                debug!("Got slow-path graphics update ({} bytes)", data.len());
                Ok(vec![ProcessorOutput::GraphicsUpdate(data)])
            }
            ShareDataPdu::Pointer(data) => {
                let data = Self::decompress_share_data(data, compression_flags, compression_type, bulk_decompressor)?;
                debug!("Got slow-path pointer update ({} bytes)", data.len());
                Ok(vec![ProcessorOutput::PointerUpdate(data)])
            }
            ShareDataPdu::MonitorLayout(monitor_layout) => {
                Ok(vec![ProcessorOutput::MonitorLayout(monitor_layout.monitors)])
            }
            pdu => Err(reason_err!("IO channel", "unhandled PDU: {:?}", pdu.as_short_name())),
        }
    }

    fn decompress_share_data(
        data: Vec<u8>,
        compression_flags: CompressionFlags,
        compression_type: CompressionType,
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<u8>> {
        if compression_flags.is_empty() {
            return Ok(data);
        }

        let decompressor = bulk_decompressor
            .as_mut()
            .ok_or_else(|| reason_err!("slow-path", "received compressed share data without a decompressor"))?;
        let flags = u32::from(compression_flags.bits()) | u32::from(compression_type.as_u8());
        let decompressed = decompressor
            .decompress(&data, flags)
            .map_err(|error| reason_err!("slow-path", "bulk decompression failed: {error}"))?
            .to_vec();
        debug!(
            compressed_size = data.len(),
            decompressed_size = decompressed.len(),
            ?compression_type,
            "Decompressed slow-path share data"
        );
        Ok(decompressed)
    }

    /// Process a PDU received on the MCS message channel: auto-detect
    /// ([MS-RDPBCGR] 2.2.14), multitransport ([MS-RDPBCGR] 2.2.15), or
    /// Heartbeat ([MS-RDPBCGR] 2.2.16.1).
    ///
    /// The PDU families share the channel and are told apart by the
    /// `BasicSecurityHeader` flags (masking off `SEC_RESET_SEQNO`/
    /// `SEC_IGNORE_SEQNO`, which the spec says MUST be ignored, before
    /// comparing), peeked here before committing to either decode. Any other
    /// flag combination is logged and ignored rather than treated as a
    /// session-fatal decode error: this channel is forward-safe for future
    /// message-channel PDU types the same way the connect-time demux
    /// (`ironrdp-connector`) already is.
    fn process_message_channel(&self, data_ctx: SendDataIndicationCtx<'_>) -> SessionResult<Vec<ProcessorOutput>> {
        let Some(message_channel_id) = self.message_channel_id else {
            return Err(reason_err!("message channel", "no message channel negotiated"));
        };

        let mut peek = ReadCursor::new(data_ctx.user_data);
        let security_header = BasicSecurityHeader::decode(&mut peek).map_err(SessionError::decode)?;
        let flags = security_header
            .flags
            .difference(BasicSecurityHeaderFlags::RESET_SEQNO | BasicSecurityHeaderFlags::IGNORE_SEQNO);

        if flags == BasicSecurityHeaderFlags::HEARTBEAT {
            let heartbeat = decode::<HeartbeatPdu>(data_ctx.user_data).map_err(SessionError::decode)?;
            debug!(
                period = heartbeat.period,
                count1 = heartbeat.count1,
                count2 = heartbeat.count2,
                "Received Heartbeat PDU"
            );
            return Ok(Vec::new());
        }

        if flags == BasicSecurityHeaderFlags::TRANSPORT_REQ {
            let request = decode::<MultitransportRequestPdu>(data_ctx.user_data).map_err(SessionError::decode)?;
            debug!(
                request_id = request.request_id,
                "Received Initiate Multitransport Request"
            );
            return Ok(vec![ProcessorOutput::MultitransportRequest(request)]);
        }

        if flags != BasicSecurityHeaderFlags::AUTODETECT_REQ {
            debug!(flags = ?security_header.flags, "Unrecognized message-channel PDU, ignoring");
            return Ok(Vec::new());
        }

        let req = decode::<AutoDetectReqPdu>(data_ctx.user_data).map_err(SessionError::decode)?;

        match req.request {
            AutoDetectRequest::RttRequest { sequence_number, .. } => {
                let response = AutoDetectRspPdu::new(AutoDetectResponse::RttResponse { sequence_number });
                let mut frame = WriteBuf::new();
                ironrdp_pdu::mcs::encode_send_data_request(
                    self.user_channel_id,
                    message_channel_id,
                    &response,
                    &mut frame,
                )
                .map_err(SessionError::encode)?;
                debug!(sequence_number, "Responded to auto-detect RTT request");
                Ok(vec![ProcessorOutput::ResponseFrame(frame.into_inner())])
            }
            req @ AutoDetectRequest::NetworkCharacteristicsResult { .. } => {
                debug!(?req, "Received network characteristics from server");
                Ok(vec![ProcessorOutput::AutoDetect(req)])
            }
            req => {
                debug!(?req, "Auto-detect request not yet implemented");
                Ok(Vec::new())
            }
        }
    }

    /// Encodes an Initiate Multitransport Response on the MCS message channel.
    ///
    /// See [\[MS-RDPBCGR\] 2.2.15.2].
    ///
    /// [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
    pub fn encode_multitransport_response(&self, response: &MultitransportResponsePdu) -> SessionResult<Vec<u8>> {
        let message_channel_id = self
            .message_channel_id
            .ok_or_else(|| reason_err!("message channel", "no message channel negotiated"))?;
        let mut frame = WriteBuf::new();
        ironrdp_pdu::mcs::encode_send_data_request(self.user_channel_id, message_channel_id, response, &mut frame)
            .map_err(SessionError::encode)?;
        Ok(frame.into_inner())
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn encode_static(&self, output: &mut WriteBuf, pdu: ShareDataPdu) -> SessionResult<usize> {
        let written = ironrdp_pdu::rdp::headers::encode_share_data(
            self.user_channel_id,
            self.io_channel_id,
            self.share_id,
            pdu,
            output,
        )
        .map_err(SessionError::encode)?;
        Ok(written)
    }
}

fn is_logon_complete(session_info: &SaveSessionInfoPdu) -> bool {
    matches!(
        session_info.info_data,
        InfoData::LogonInfoV1(_) | InfoData::LogonInfoV2(_) | InfoData::PlainNotify
    )
}

/// Processes a vector of [`SvcMessage`] in preparation for sending them to the server on the `channel_id` channel.
///
/// This includes chunkifying the messages, adding MCS, x224, and tpkt headers, and encoding them into a buffer.
/// The messages returned here are ready to be sent to the server.
///
/// The caller is responsible for ensuring that the `channel_id` corresponds to the correct channel.
fn process_svc_messages(
    messages: Vec<SvcMessage>,
    channel_id: u16,
    initiator_id: u16,
    maximum_chunk_size: usize,
) -> SessionResult<Vec<u8>> {
    client_encode_svc_messages_with_max_chunk_len(messages, channel_id, initiator_id, maximum_chunk_size)
        .map_err(SessionError::encode)
}

#[cfg(test)]
mod tests {
    use ironrdp_bulk::{CompressionType as BulkCompressionType, flags};
    use ironrdp_core::encode_vec;
    use ironrdp_pdu::gcc::MonitorFlags;
    use ironrdp_pdu::rdp::finalization_messages::MonitorLayoutPdu;
    use ironrdp_pdu::rdp::headers::ShareDataPduType;
    use ironrdp_pdu::rdp::multitransport::RequestedProtocol;
    use ironrdp_pdu::rdp::session_info::{InfoType, LogonExFlags, LogonInfoExtended};

    use super::*;

    fn multitransport_request() -> MultitransportRequestPdu {
        MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: 42,
            requested_protocol: RequestedProtocol::UdpFecR,
            security_cookie: [0xAB; 16],
        }
    }

    #[test]
    fn processor_decompresses_slow_path_share_data() {
        let source = vec![b'A'; 1024];
        let mut compressor = BulkCompressor::new(BulkCompressionType::Rdp5);
        let (compressed_size, flags) = compressor.compress(&source).expect("source should compress");
        assert_ne!(flags & flags::PACKET_COMPRESSED, 0, "test data must be compressed");
        let compressed = compressor.compressed_data(compressed_size).to_vec();
        let mut bulk_decompressor = Some(BulkCompressor::new(BulkCompressionType::Rdp5));
        let compression_flags = CompressionFlags::from_bits_retain(
            u8::try_from(flags & !flags::COMPRESSION_TYPE_MASK).expect("bulk flags should fit in a byte"),
        );

        assert_eq!(
            Processor::decompress_share_data(
                compressed,
                compression_flags,
                CompressionType::K64,
                &mut bulk_decompressor
            )
            .expect("compressed slow-path data should decompress"),
            source
        );
    }

    #[test]
    fn processor_rejects_compressed_slow_path_data_without_a_decompressor() {
        let mut bulk_decompressor = None;

        assert!(
            Processor::decompress_share_data(
                vec![0],
                CompressionFlags::COMPRESSED,
                CompressionType::K64,
                &mut bulk_decompressor
            )
            .is_err()
        );
    }

    #[test]
    fn processor_surfaces_multitransport_request_on_message_channel() {
        let request = multitransport_request();
        let encoded = encode_vec(&request).expect("encode multitransport request");
        let processor = Processor::new(StaticChannelSet::new(), 1002, 1003, Some(1004), 0);

        let outputs = processor
            .process_message_channel(SendDataIndicationCtx {
                initiator_id: 1002,
                channel_id: 1004,
                user_data: &encoded,
            })
            .expect("surface multitransport request");

        assert!(matches!(
            outputs.as_slice(),
            [ProcessorOutput::MultitransportRequest(decoded)] if decoded == &request
        ));
    }

    #[test]
    fn processor_ignores_multitransport_request_on_io_channel() {
        let request = multitransport_request();
        let encoded = encode_vec(&request).expect("encode multitransport request");
        let mut processor = Processor::new(StaticChannelSet::new(), 1002, 1003, Some(1004), 0);

        let outputs = processor
            .process_io_channel_data_indication(
                SendDataIndicationCtx {
                    initiator_id: 1002,
                    channel_id: 1003,
                    user_data: &encoded,
                },
                &mut None,
            )
            .expect("ignore a misrouted optional multitransport request");

        assert!(outputs.is_empty());
    }

    #[test]
    fn processor_encodes_multitransport_response_on_message_channel() {
        let processor = Processor::new(StaticChannelSet::new(), 1002, 1003, Some(1004), 0);
        let response = MultitransportResponsePdu::success(42);

        let frame = processor
            .encode_multitransport_response(&response)
            .expect("encode multitransport response");
        let X224(McsMessage::SendDataRequest(request)) =
            decode::<X224<McsMessage<'_>>>(&frame).expect("decode MCS Send Data Request")
        else {
            panic!("expected MCS Send Data Request");
        };

        assert_eq!(request.initiator_id, 1002);
        assert_eq!(request.channel_id, 1004);
        assert_eq!(
            decode::<MultitransportResponsePdu>(&request.user_data).expect("decode multitransport response"),
            response
        );
    }

    #[test]
    fn processor_rejects_multitransport_response_without_message_channel() {
        let processor = Processor::new(StaticChannelSet::new(), 1002, 1003, None, 0);

        let error = processor
            .encode_multitransport_response(&MultitransportResponsePdu::success(42))
            .expect_err("message channel is required");

        assert!(error.to_string().contains("no message channel negotiated"));
    }

    #[test]
    fn processor_decompresses_compressed_save_session_info() {
        let session_info = SaveSessionInfoPdu {
            info_type: InfoType::PlainNotify,
            info_data: InfoData::PlainNotify,
        };
        let source = encode_vec(&session_info).expect("encode save session info");
        let mut compressor = BulkCompressor::new(BulkCompressionType::Rdp5);
        let (compressed_size, flags) = compressor.compress(&source).expect("source should compress");
        assert_ne!(flags & flags::PACKET_COMPRESSED, 0, "test data must be compressed");
        let compressed = compressor.compressed_data(compressed_size).to_vec();
        let mut bulk_decompressor = Some(BulkCompressor::new(BulkCompressionType::Rdp5));
        let compression_flags = CompressionFlags::from_bits_retain(
            u8::try_from(flags & !flags::COMPRESSION_TYPE_MASK).expect("bulk flags should fit in a byte"),
        );
        let outputs = Processor::process_share_data(
            ShareDataCtx {
                initiator_id: 0,
                channel_id: 0,
                share_id: 0,
                pdu_source: 0,
                compression_flags,
                compression_type: CompressionType::K64,
                pdu: ShareDataPdu::Compressed {
                    pdu_type: ShareDataPduType::SaveSessionInfo,
                    data: compressed,
                },
            },
            &mut bulk_decompressor,
        )
        .expect("compressed save session info should be processed");

        assert!(matches!(
            outputs.as_slice(),
            [ProcessorOutput::SaveSessionInfo { logon_complete: true }]
        ));
    }

    #[test]
    fn extended_session_info_does_not_signal_login_completion() {
        let session_info = SaveSessionInfoPdu {
            info_type: InfoType::LogonExtended,
            info_data: InfoData::LogonExtended(LogonInfoExtended {
                present_fields_flags: LogonExFlags::AUTO_RECONNECT_COOKIE,
                auto_reconnect: None,
                errors_info: None,
            }),
        };

        assert!(!is_logon_complete(&session_info));
    }

    #[test]
    fn processor_gracefully_disconnects_on_provider_ultimatum() {
        let frame = encode_vec(&X224(McsMessage::DisconnectProviderUltimatum(
            DisconnectProviderUltimatum::from_reason(DisconnectReason::ProviderInitiated),
        )))
        .expect("encode disconnect provider ultimatum");
        let mut processor = Processor::new(StaticChannelSet::new(), 1002, 1003, None, 0);

        let outputs = processor
            .process(&frame, &mut None)
            .expect("disconnect provider ultimatum should not be a protocol error");

        assert!(matches!(
            outputs.as_slice(),
            [ProcessorOutput::Disconnect(DisconnectDescription::McsDisconnect(
                DisconnectReason::ProviderInitiated
            ))]
        ));
    }

    #[test]
    fn plain_notify_signals_login_completion() {
        let session_info = SaveSessionInfoPdu {
            info_type: InfoType::PlainNotify,
            info_data: InfoData::PlainNotify,
        };

        assert!(is_logon_complete(&session_info));
    }

    #[test]
    fn processor_surfaces_valid_auto_reconnect_status() {
        let mut bulk_decompressor = None;
        let outputs = Processor::process_share_data(
            ShareDataCtx {
                initiator_id: 0,
                channel_id: 0,
                share_id: 0,
                pdu_source: 0,
                compression_flags: CompressionFlags::empty(),
                compression_type: CompressionType::K64,
                pdu: ShareDataPdu::ArcStatusPdu(vec![0; 4]),
            },
            &mut bulk_decompressor,
        )
        .expect("valid auto-reconnect status PDU should be processed");

        assert!(matches!(outputs.as_slice(), [ProcessorOutput::AutoReconnectFailed]));
    }

    #[test]
    fn processor_surfaces_monitor_layout() {
        let monitors = vec![Monitor {
            left: 0,
            top: 0,
            right: 799,
            bottom: 599,
            flags: MonitorFlags::PRIMARY,
        }];
        let mut bulk_decompressor = None;
        let outputs = Processor::process_share_data(
            ShareDataCtx {
                initiator_id: 0,
                channel_id: 0,
                share_id: 0,
                pdu_source: 0,
                compression_flags: CompressionFlags::empty(),
                compression_type: CompressionType::K64,
                pdu: ShareDataPdu::MonitorLayout(MonitorLayoutPdu {
                    monitors: monitors.clone(),
                }),
            },
            &mut bulk_decompressor,
        )
        .expect("monitor layout PDU should be processed");

        let [ProcessorOutput::MonitorLayout(actual)] = outputs.as_slice() else {
            panic!("expected a monitor layout output");
        };
        assert_eq!(actual, &monitors);
    }

    #[test]
    fn processor_rejects_invalid_auto_reconnect_status() {
        let mut bulk_decompressor = None;

        assert!(
            Processor::process_share_data(
                ShareDataCtx {
                    initiator_id: 0,
                    channel_id: 0,
                    share_id: 0,
                    pdu_source: 0,
                    compression_flags: CompressionFlags::empty(),
                    compression_type: CompressionType::K64,
                    pdu: ShareDataPdu::ArcStatusPdu(vec![0, 0, 0]),
                },
                &mut bulk_decompressor,
            )
            .is_err()
        );
    }

    fn header_only_deactivate_all() -> [u8; 6] {
        [
            0x06, 0x00, // totalLength
            0x16, 0x00, // pduType (Deactivate All) + protocolVersion
            0xE9, 0x03, // pduSource
        ]
    }

    #[test]
    fn processor_splits_concatenated_share_control_pdus() {
        let pdu = header_only_deactivate_all();
        let mut user_data = Vec::from(pdu);
        user_data.extend_from_slice(&pdu);
        let mut processor = Processor::new(StaticChannelSet::new(), 1002, 1003, None, 0);

        let outputs = processor
            .process_io_channel_data_indication(
                SendDataIndicationCtx {
                    initiator_id: 1002,
                    channel_id: 1003,
                    user_data: &user_data,
                },
                &mut None,
            )
            .expect("concatenated Share Control PDUs should be split");

        assert!(matches!(
            outputs.as_slice(),
            [ProcessorOutput::DeactivateAll, ProcessorOutput::DeactivateAll]
        ));
    }

    #[test]
    fn processor_rejects_truncated_share_control_pdu_length() {
        let mut processor = Processor::new(StaticChannelSet::new(), 1002, 1003, None, 0);

        let error = processor
            .process_io_channel_data_indication(
                SendDataIndicationCtx {
                    initiator_id: 1002,
                    channel_id: 1003,
                    user_data: &[0x06],
                },
                &mut None,
            )
            .expect_err("a truncated totalLength field is invalid");

        assert!(error.to_string().contains("truncated Share Control PDU length"));
    }

    #[test]
    fn processor_rejects_invalid_concatenated_share_control_pdu_length() {
        let mut user_data = Vec::from(header_only_deactivate_all());
        user_data.extend_from_slice(&[0x10, 0x00]);
        let mut processor = Processor::new(StaticChannelSet::new(), 1002, 1003, None, 0);

        let error = processor
            .process_io_channel_data_indication(
                SendDataIndicationCtx {
                    initiator_id: 1002,
                    channel_id: 1003,
                    user_data: &user_data,
                },
                &mut None,
            )
            .expect_err("an overrunning concatenated totalLength is invalid");

        assert!(
            error
                .to_string()
                .contains("invalid concatenated Share Control PDU length: 16")
        );
    }

    #[test]
    fn processor_falls_back_for_invalid_first_share_control_pdu_length() {
        let mut user_data = Vec::from(header_only_deactivate_all());
        user_data[0] = 0x64;
        user_data[1] = 0x00;
        let mut processor = Processor::new(StaticChannelSet::new(), 1002, 1003, None, 0);

        let outputs = processor
            .process_io_channel_data_indication(
                SendDataIndicationCtx {
                    initiator_id: 1002,
                    channel_id: 1003,
                    user_data: &user_data,
                },
                &mut None,
            )
            .expect("invalid first totalLength should fall back to whole-buffer decode");

        assert!(matches!(outputs.as_slice(), [ProcessorOutput::DeactivateAll]));
    }
}
