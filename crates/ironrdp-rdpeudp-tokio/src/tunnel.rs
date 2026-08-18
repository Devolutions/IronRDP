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
) -> Result<(), UdpTransportError>
where
    S: AsyncRead + Unpin,
{
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
                // `sub_headers` (e.g. auto-detect bandwidth measurement, MS-RDPBCGR
                // 2.2.14) are not consumed here; this driver only wires the DVC
                // payload through. A future auto-detect integration would need to
                // dispatch them instead of discarding them.
                TunnelEvent::Data { data, .. } => {
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

#[cfg(test)]
mod tests {
    use super::*;

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
