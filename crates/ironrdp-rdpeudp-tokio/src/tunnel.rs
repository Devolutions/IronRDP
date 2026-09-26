//! Async wrapper for RDPEMT tunnel handshake and data framing.
//!
//! The sans-I/O `RdpemtTunnel` from `ironrdp-rdpemt` handles the
//! protocol state machine. This module provides async I/O helpers
//! to drive it over a TLS stream:
//!
//! - `establish_tunnel()`: perform the CreateRequest/CreateResponse handshake
//! - `read_tunnel_pdu()`: read a complete RDPEMT PDU using the self-framing header
//! - `write_tunnel_pdu()`: write a PDU and flush

use std::io;

use ironrdp_rdpemt::{RdpemtTunnel, TunnelEvent};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::error::{UdpTransportError, UdpTransportErrorExt as _};

/// Read a complete RDPEMT PDU from the stream using self-framing.
///
/// Wire layout:
/// ```text
/// Byte 0:   Action
/// Byte 1-2: PayloadLength (u16 LE)
/// Byte 3:   HeaderLength (u8, >= 4)
/// Bytes 4..HeaderLength:  SubHeaders (optional)
/// Bytes HeaderLength..:   Higher-layer data (PayloadLength bytes)
/// ```
///
/// Total PDU size = HeaderLength + PayloadLength.
///
/// Returns `Ok(None)` only for EOF before any byte of a new PDU has been
/// read: that is the sole case that is a clean shutdown between frames.
/// Once even the first header byte has been consumed, an EOF anywhere
/// later (mid-header, mid-subheader, mid-payload) means the peer closed
/// mid-frame, and is reported as an error rather than folded into the
/// clean-shutdown case, so a truncated PDU cannot be silently mistaken
/// for a graceful close.
pub(crate) async fn read_tunnel_pdu<S>(stream: &mut S) -> Result<Option<Vec<u8>>, UdpTransportError>
where
    S: AsyncRead + Unpin,
{
    // Read the first header byte alone. Both a zero-length read (a plain
    // stream at true EOF) and an UnexpectedEof error (rustls reports EOF
    // without a TLS close_notify this way rather than as Ok(0); this
    // transport's own shutdown() doesn't send one) are legitimate here: no
    // byte of a new PDU has been consumed, so this is a clean shutdown
    // between frames. Any other error kind, or any EOF once this first byte
    // is in hand, means the peer closed mid-frame and is a truncated PDU,
    // not a clean close.
    let mut header = [0u8; 4];
    match stream.read(&mut header[..1]).await {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(UdpTransportError::tls("read tunnel pdu", error)),
    }

    // Any EOF from here on is mid-frame: a truncated PDU, not a clean close.
    stream
        .read_exact(&mut header[1..])
        .await
        .map_err(|error| UdpTransportError::tls("read tunnel pdu", error))?;

    let payload_len = usize::from(u16::from_le_bytes([header[1], header[2]]));
    let header_len = usize::from(header[3]);

    // Sub-headers occupy the space between the fixed 4-byte header
    // and the end of the header region
    let extra_header = header_len.saturating_sub(4);
    let total = header_len + payload_len;

    let mut buf = Vec::with_capacity(total);
    buf.extend_from_slice(&header);

    if extra_header > 0 {
        buf.resize(4 + extra_header, 0);
        stream
            .read_exact(&mut buf[4..4 + extra_header])
            .await
            .map_err(|error| UdpTransportError::tls("read tunnel pdu", error))?;
    }

    if payload_len > 0 {
        let offset = buf.len();
        buf.resize(offset + payload_len, 0);
        stream
            .read_exact(&mut buf[offset..])
            .await
            .map_err(|error| UdpTransportError::tls("read tunnel pdu", error))?;
    }

    Ok(Some(buf))
}

/// Write a complete RDPEMT PDU to the stream and flush.
pub(crate) async fn write_tunnel_pdu<S>(stream: &mut S, pdu: &[u8]) -> Result<(), UdpTransportError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(pdu)
        .await
        .map_err(|error| UdpTransportError::tls("write tunnel pdu", error))?;
    stream
        .flush()
        .await
        .map_err(|error| UdpTransportError::tls("write tunnel pdu", error))?;
    Ok(())
}

/// Read RDPEMT data PDUs from the TLS stream and forward higher-layer
/// data to the application via a channel.
///
/// This runs in the same task as the data forwarding loop after the
/// tunnel is established. It reads tunnel PDUs, processes them through
/// the sans-I/O state machine, and sends `TunnelEvent::Data` payloads
/// to the application.
pub(crate) async fn tunnel_data_loop<S>(
    stream: &mut S,
    tunnel: &mut RdpemtTunnel,
    data_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    outgoing_tx: &tokio::sync::mpsc::Sender<Outgoing>,
) -> Result<(), UdpTransportError>
where
    S: AsyncRead + Unpin,
{
    let mut auto_detect = AutoDetectResponder::default();
    loop {
        let pdu = match read_tunnel_pdu(stream).await {
            Ok(Some(pdu)) => pdu,
            // Clean shutdown: EOF before any byte of a new PDU.
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };

        tunnel
            .handle_pdu(&pdu)
            .map_err(|error| UdpTransportError::rdpemt("tunnel data loop", error))?;

        while let Some(event) = tunnel.poll_event() {
            match event {
                // MS-RDPEMT 2.2.1.1.1: sub-headers carry the server's auto-detect requests
                // for this transport (MS-RDPBCGR 2.2.14). Their responses go back on the
                // tunnel ahead of any higher-layer data queued after this point.
                TunnelEvent::Data { data, sub_headers } => {
                    for response in auto_detect.handle(&sub_headers, data.len()) {
                        if outgoing_tx.send(Outgoing::Encoded(response)).await.is_err() {
                            // The write pump is gone
                            return Ok(());
                        }
                    }
                    if data_tx.send(data).await.is_err() {
                        // Application dropped the receiver
                        return Ok(());
                    }
                }
                TunnelEvent::Established => {
                    // Already established, ignore duplicate
                }
                TunnelEvent::Failed { hr_response } => {
                    return Err(UdpTransportError::tunnel_rejected("tunnel data loop", hr_response));
                }
                // `TunnelEvent` is `#[non_exhaustive]`; a variant this driver
                // doesn't know about yet is ignored rather than treated as an
                // error, matching how `Established` is handled above.
                _ => {}
            }
        }
    }
}

/// What the write pump puts on the tunnel.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outgoing {
    /// Higher-layer data, wrapped in a `TunnelData` PDU by the write pump.
    Data(Vec<u8>),
    /// A complete, already encoded tunnel PDU, such as an auto-detect response.
    Encoded(Vec<u8>),
}

/// Answers the auto-detect requests the server sends in tunnel sub-headers
/// ([\[MS-RDPBCGR\] 2.2.14]): RTT requests get an RTT response, and a bandwidth
/// measurement is timed and byte-counted from its Start to its Stop.
///
/// A continuous measurement on a tunnel carries no payload PDUs: the server
/// brackets ordinary channel traffic with Start and Stop and expects the
/// client to report the bytes it received in between. MS-RDPBCGR 3.2.5.14
/// counts only the data that follows the tunnel PDU header, so the
/// higher-layer data of every tunnel PDU is counted while a window is open.
///
/// A tunnel sub-header *is* the auto-detect structure (MS-RDPEMT 2.2.1.1.1):
/// its `SubHeaderLength` and `SubHeaderType` bytes are the structure's
/// `headerLength` and `headerTypeId`, and [`TunnelSubHeader::data`] holds only
/// what follows them.
///
/// [\[MS-RDPBCGR\] 2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38
/// [`TunnelSubHeader::data`]: ironrdp_rdpemt::TunnelSubHeader::data
#[derive(Default)]
struct AutoDetectResponder {
    bandwidth_started_at: Option<std::time::Instant>,
    bandwidth_bytes: u64,
}

impl AutoDetectResponder {
    /// Size of the `headerLength` and `headerTypeId` fields that a tunnel sub-header
    /// carries as its `SubHeaderLength` and `SubHeaderType`.
    const HEADER_SIZE: usize = 1 /* headerLength */ + 1 /* headerTypeId */;

    /// Handles the sub-headers of one tunnel PDU carrying `data_len` bytes of higher-layer
    /// data and returns the encoded tunnel PDUs that answer them.
    ///
    /// The data follows the sub-headers on the wire, so it belongs to a window its PDU
    /// opens and not to one its PDU closes.
    fn handle(&mut self, sub_headers: &[ironrdp_rdpemt::TunnelSubHeader], data_len: usize) -> Vec<Vec<u8>> {
        use ironrdp_pdu::rdp::autodetect::{
            AutoDetectRequest, AutoDetectResponse, BW_RESULTS_CONNECT_TIME, BW_RESULTS_CONTINUOUS,
            BW_STOP_CONNECT_TIME, TYPE_ID_AUTODETECT_REQUEST,
        };
        use ironrdp_rdpemt::{SubHeaderType, TunnelData, TunnelSubHeader};

        let mut responses = Vec::new();
        for sub_header in sub_headers {
            if sub_header.sub_header_type != SubHeaderType::AutoDetectRequest {
                continue;
            }

            // Re-attach the two header bytes so the MS-RDPBCGR 2.2.14.1 decoder sees the
            // layout it expects.
            let request = {
                let header_length = u8::try_from(sub_header.data.len() + Self::HEADER_SIZE).unwrap_or(u8::MAX);
                let mut structure = Vec::with_capacity(sub_header.data.len() + Self::HEADER_SIZE);
                structure.push(header_length);
                structure.push(TYPE_ID_AUTODETECT_REQUEST);
                structure.extend_from_slice(&sub_header.data);
                match ironrdp_core::decode::<AutoDetectRequest>(&structure) {
                    Ok(request) => request,
                    Err(error) => {
                        // Windows sends request types this decoder does not model; skipping
                        // them keeps the tunnel up.
                        tracing::debug!(%error, data = ?sub_header.data, "Ignoring an undecodable auto-detect request on the tunnel");
                        continue;
                    }
                }
            };

            let response = match request {
                AutoDetectRequest::RttRequest { sequence_number, .. } => {
                    AutoDetectResponse::RttResponse { sequence_number }
                }
                AutoDetectRequest::BandwidthMeasureStart { .. } => {
                    // A repeated Start restarts the measurement.
                    self.bandwidth_started_at = Some(std::time::Instant::now());
                    self.bandwidth_bytes = 0;
                    continue;
                }
                // Sub-headers are part of the tunnel PDU header, so a Payload adds nothing.
                AutoDetectRequest::BandwidthMeasurePayload { .. } => continue,
                AutoDetectRequest::BandwidthMeasureStop {
                    sequence_number,
                    request_type,
                    ..
                } => {
                    // The server divides by timeDelta (MS-RDPBCGR 3.3.5.14), and a LAN
                    // measurement can complete within a millisecond, so it is floored at 1.
                    let time_delta_ms = self
                        .bandwidth_started_at
                        .take()
                        .map_or(1, |started_at| {
                            u32::try_from(started_at.elapsed().as_millis()).unwrap_or(u32::MAX)
                        })
                        .max(1);
                    AutoDetectResponse::BandwidthMeasureResults {
                        sequence_number,
                        response_type: if request_type == BW_STOP_CONNECT_TIME {
                            BW_RESULTS_CONNECT_TIME
                        } else {
                            BW_RESULTS_CONTINUOUS
                        },
                        time_delta_ms,
                        byte_count: u32::try_from(core::mem::take(&mut self.bandwidth_bytes)).unwrap_or(u32::MAX),
                    }
                }
                AutoDetectRequest::NetworkCharacteristicsResult {
                    base_rtt_ms,
                    bandwidth_kbps,
                    average_rtt_ms,
                    ..
                } => {
                    tracing::debug!(
                        ?base_rtt_ms,
                        ?bandwidth_kbps,
                        average_rtt_ms,
                        "Received tunnel network characteristics"
                    );
                    continue;
                }
            };

            tracing::debug!(?response, "Answering an auto-detect request on the tunnel");
            // Symmetric to the request: `TunnelSubHeader::encode` writes the header bytes the
            // encoded response starts with, so they are dropped from the sub-header data.
            let pdu = ironrdp_core::encode_vec(&response).and_then(|mut structure| {
                structure.drain(..Self::HEADER_SIZE.min(structure.len()));
                ironrdp_core::encode_vec(&TunnelData {
                    sub_headers: vec![TunnelSubHeader {
                        sub_header_type: SubHeaderType::AutoDetectResponse,
                        data: structure,
                    }],
                    higher_layer_data: Vec::new(),
                })
            });
            match pdu {
                Ok(pdu) => responses.push(pdu),
                Err(error) => tracing::debug!(%error, "Could not encode an auto-detect response"),
            }
        }
        if self.bandwidth_started_at.is_some() {
            self.bandwidth_bytes = self
                .bandwidth_bytes
                .saturating_add(u64::try_from(data_len).unwrap_or(u64::MAX));
        }
        responses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows frames an RTT Measure Request as `06 00 <seq> 01 00`: the sub-header prefix
    /// doubles as the auto-detect header. The response is framed the same way,
    /// `06 01 <seq> 00 00`, with no second header inside the sub-header data.
    #[test]
    fn auto_detect_responder_answers_an_rtt_request_framed_as_a_sub_header() {
        use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};

        let mut responder = AutoDetectResponder::default();
        let sub_header = TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            // sequenceNumber = 0x0007, requestType = RTT_REQUEST_CONTINUOUS (0x0001)
            data: vec![0x07, 0x00, 0x01, 0x00],
        };
        let responses = responder.handle(&[sub_header], 10);
        // TunnelData: action = 2, payloadLength = 0, headerLength = 4 + 6, then the sub-header.
        assert_eq!(
            responses,
            [vec![0x02, 0x00, 0x00, 0x0A, 0x06, 0x01, 0x07, 0x00, 0x00, 0x00]]
        );
    }

    /// A continuous measurement is answered with one Results PDU that counts the higher-layer
    /// data received between the Start and the Stop, and nothing from the tunnel headers.
    #[test]
    fn auto_detect_responder_measures_bandwidth_between_start_and_stop() {
        use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};

        let mut responder = AutoDetectResponder::default();
        let start = TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            // sequenceNumber = 1, requestType = BW_START_RELIABLE_UDP (0x0014)
            data: vec![0x01, 0x00, 0x14, 0x00],
        };
        // The data in the PDU that carries the Start follows it, so it is counted.
        assert!(responder.handle(&[start], 7).is_empty());
        assert!(responder.handle(&[], 100).is_empty());
        let stop = TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            // sequenceNumber = 3, requestType = BW_STOP_RELIABLE_UDP (0x0429)
            data: vec![0x03, 0x00, 0x29, 0x04],
        };
        // The data in the PDU that carries the Stop follows it, so it is not.
        let responses = responder.handle(&[stop], 50);
        let [pdu] = responses.as_slice() else {
            panic!("expected exactly one bandwidth result");
        };
        assert_eq!(&pdu[..4], &[0x02, 0x00, 0x00, 0x12]);
        // Sub-header: length 0x0E, AutoDetectResponse, sequenceNumber 3, BW_RESULTS_CONTINUOUS.
        assert_eq!(&pdu[4..10], &[0x0E, 0x01, 0x03, 0x00, 0x0B, 0x00]);
        let byte_count = u32::from_le_bytes([pdu[14], pdu[15], pdu[16], pdu[17]]);
        assert_eq!(byte_count, 7 + 100);

        // Nothing is counted once the window is closed.
        assert!(responder.handle(&[], 100).is_empty());
        assert_eq!(responder.bandwidth_bytes, 0);
    }

    /// Request types this decoder does not model are skipped rather than failing the tunnel.
    #[test]
    fn auto_detect_responder_skips_undecodable_requests() {
        use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};

        let mut responder = AutoDetectResponder::default();
        let unknown = TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            data: vec![0x01, 0x00, 0xFF, 0x7F],
        };
        assert!(responder.handle(&[unknown], 10).is_empty());
    }

    /// Verify that read_tunnel_pdu correctly reassembles a simple Data PDU.
    #[tokio::test]
    async fn read_tunnel_pdu_simple_data() {
        // Encode a TunnelData PDU: Action=2, PayloadLen=5, HeaderLen=4, "Hello"
        let wire: Vec<u8> = vec![0x02, 0x05, 0x00, 0x04, 0x48, 0x65, 0x6C, 0x6C, 0x6F];

        let mut cursor = io::Cursor::new(wire);
        let pdu = read_tunnel_pdu(&mut cursor).await.expect("read").expect("pdu present");
        assert_eq!(pdu, [0x02, 0x05, 0x00, 0x04, 0x48, 0x65, 0x6C, 0x6C, 0x6F]);
    }

    /// Verify that read_tunnel_pdu handles sub-headers.
    #[tokio::test]
    async fn read_tunnel_pdu_with_subheader() {
        // Action=2, PayloadLen=2, HeaderLen=7 (4 + 3 subheader), subheader=[03, 00, FF], payload=[01, 02]
        let expected: &[u8] = &[0x02, 0x02, 0x00, 0x07, 0x03, 0x00, 0xFF, 0x01, 0x02];

        let mut cursor = io::Cursor::new(expected.to_vec());
        let pdu = read_tunnel_pdu(&mut cursor).await.expect("read").expect("pdu present");
        assert_eq!(pdu, expected);
    }

    /// Verify that read_tunnel_pdu handles empty payload.
    #[tokio::test]
    async fn read_tunnel_pdu_empty_payload() {
        // Action=2, PayloadLen=0, HeaderLen=4
        let expected: &[u8] = &[0x02, 0x00, 0x00, 0x04];

        let mut cursor = io::Cursor::new(expected.to_vec());
        let pdu = read_tunnel_pdu(&mut cursor).await.expect("read").expect("pdu present");
        assert_eq!(pdu, expected);
    }

    /// Verify that write + read roundtrips through an in-memory pipe.
    #[tokio::test]
    async fn write_read_roundtrip() {
        let (mut client, mut server) = tokio::io::duplex(4096);

        let original: Vec<u8> = vec![0x02, 0x03, 0x00, 0x04, 0xAA, 0xBB, 0xCC];

        let write_handle = tokio::spawn(async move {
            write_tunnel_pdu(&mut client, &original).await.expect("write");
        });

        let pdu = read_tunnel_pdu(&mut server).await.expect("read").expect("pdu present");
        write_handle.await.expect("join");

        assert_eq!(pdu, [0x02, 0x03, 0x00, 0x04, 0xAA, 0xBB, 0xCC]);
    }

    /// EOF before any byte of a new PDU is a clean shutdown: `Ok(None)`, not
    /// an error and not confused with a truncated PDU.
    #[tokio::test]
    async fn read_tunnel_pdu_clean_eof_between_frames() {
        let mut cursor = io::Cursor::new(Vec::<u8>::new());
        let pdu = read_tunnel_pdu(&mut cursor).await.expect("read");
        assert!(pdu.is_none());
    }

    /// EOF after the first header byte, but before the PDU is complete, is a
    /// truncated PDU: an error, never `Ok(None)`. Covers all three points a
    /// real peer could die mid-frame: mid-header, mid-subheader, mid-payload.
    #[tokio::test]
    async fn read_tunnel_pdu_mid_frame_eof_is_an_error_not_clean_shutdown() {
        // Mid fixed header: only 2 of the 4 header bytes arrive.
        let mut cursor = io::Cursor::new(vec![0x02, 0x05]);
        assert!(read_tunnel_pdu(&mut cursor).await.is_err());

        // Complete header (HeaderLen=7, so 3 subheader bytes expected) but the
        // stream dies after only 1 of them.
        let mut cursor = io::Cursor::new(vec![0x02, 0x02, 0x00, 0x07, 0x03]);
        assert!(read_tunnel_pdu(&mut cursor).await.is_err());

        // Complete header (PayloadLen=5) but only 2 payload bytes arrive.
        let mut cursor = io::Cursor::new(vec![0x02, 0x05, 0x00, 0x04, 0x48, 0x65]);
        assert!(read_tunnel_pdu(&mut cursor).await.is_err());
    }
}
