use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use core::net::IpAddr;
use pcap_parser::pcapng::{Block, SecretsType};
use pcap_parser::traits::{PcapNGPacketBlock as _, PcapReaderIterator as _};
use pcap_parser::{PcapBlockOwned, PcapError, PcapNGReader};

use crate::ReplayError;

const ETHERNET_LINKTYPE: i32 = 1;

/// One endpoint of a captured TCP flow.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Endpoint {
    /// IP address of the endpoint.
    pub address: IpAddr,
    /// TCP port of the endpoint.
    pub port: u16,
}

/// Reassembled TCP bytes associated with their source packet number.
pub type PacketStream = Vec<(usize, Vec<u8>)>;

/// A direct TCP RDP flow reconstructed from a capture.
#[derive(Clone, Debug)]
pub struct Flow {
    /// Client endpoint identified by the TCP handshake.
    pub client: Endpoint,
    /// Server endpoint identified by the TCP handshake.
    pub server: Endpoint,
    /// Reassembled client-to-server TCP stream.
    pub client_stream: PacketStream,
    /// Reassembled server-to-client TCP stream.
    pub server_stream: PacketStream,
}

/// Capture data used by later replay stages.
#[derive(Clone, Debug)]
pub struct Capture {
    /// Direct TCP RDP flow selected from the pcapng input.
    pub flow: Flow,
    /// NSS-compatible TLS key-log entries embedded in the capture.
    ///
    /// The entries are retained only in memory and must not be logged or persisted.
    pub tls_key_log: String,
}

#[derive(Clone, Debug)]
struct Segment {
    packet: usize,
    source: Endpoint,
    destination: Endpoint,
    sequence: u32,
    syn: bool,
    ack: bool,
    data: Vec<u8>,
}

/// Read an Ethernet direct TCP RDP flow from a pcapng capture.
pub fn read_capture(path: &Path) -> Result<Capture, ReplayError> {
    let file = File::open(path).map_err(ReplayError::Io)?;
    let mut reader = PcapNGReader::new(1024 * 1024, file).map_err(|error| ReplayError::Pcap(error.to_string()))?;
    let mut packet = 0;
    let mut linktypes = Vec::new();
    let mut segments = Vec::new();
    let mut tls_key_log = String::new();

    loop {
        match reader.next() {
            Ok((offset, block)) => {
                packet += 1;
                match block {
                    PcapBlockOwned::NG(Block::InterfaceDescription(interface)) => {
                        linktypes.push(interface.linktype.0);
                    }
                    PcapBlockOwned::NG(Block::DecryptionSecrets(secrets))
                        if secrets.secrets_type == SecretsType::TlsKeyLog =>
                    {
                        let length = usize::try_from(secrets.secrets_len)
                            .map_err(|_| ReplayError::Pcap("TLS secret block is too large".to_owned()))?;
                        let data = secrets
                            .data
                            .get(..length)
                            .ok_or_else(|| ReplayError::Pcap("truncated TLS secret block".to_owned()))?;
                        let text = core::str::from_utf8(data)
                            .map_err(|_| ReplayError::Pcap("TLS secret block is not UTF-8".to_owned()))?;
                        tls_key_log.push_str(text);
                    }
                    PcapBlockOwned::NG(Block::EnhancedPacket(packet_block)) => {
                        let interface = usize::try_from(packet_block.if_id)
                            .map_err(|_| ReplayError::Pcap("interface ID is too large".to_owned()))?;
                        if linktypes.get(interface) == Some(&ETHERNET_LINKTYPE) && !packet_block.truncated() {
                            if let Some(segment) = parse_tcp_packet(packet, packet_block.packet_data()) {
                                segments.push(segment);
                            }
                        }
                    }
                    _ => {}
                }
                reader.consume(offset);
            }
            Err(PcapError::Eof) => break,
            Err(PcapError::Incomplete(_)) => reader
                .refill()
                .map_err(|_| ReplayError::Pcap("truncated pcapng block".to_owned()))?,
            Err(error) => return Err(ReplayError::Pcap(error.to_string())),
        }
    }

    if !linktypes.contains(&ETHERNET_LINKTYPE) {
        return Err(ReplayError::UnsupportedTransport);
    }

    Ok(Capture {
        flow: assemble_flow(segments)?,
        tls_key_log,
    })
}

fn parse_tcp_packet(packet: usize, bytes: &[u8]) -> Option<Segment> {
    let ethernet_type = u16::from_be_bytes(bytes.get(12..14)?.try_into().ok()?);
    let (network_offset, ethernet_type) = if ethernet_type == 0x8100 {
        (18, u16::from_be_bytes(bytes.get(16..18)?.try_into().ok()?))
    } else {
        (14, ethernet_type)
    };

    let (source, destination, tcp_offset) = match ethernet_type {
        0x0800 => {
            let header = bytes.get(network_offset..)?;
            let header_len = usize::from(header.first()? & 0x0f) * 4;
            if header_len < 20 || *header.get(9)? != 6 {
                return None;
            }
            let source = IpAddr::from(<[u8; 4]>::try_from(header.get(12..16)?).ok()?);
            let destination = IpAddr::from(<[u8; 4]>::try_from(header.get(16..20)?).ok()?);
            (source, destination, network_offset + header_len)
        }
        0x86dd => {
            let header = bytes.get(network_offset..network_offset + 40)?;
            if header[6] != 6 {
                return None;
            }
            let source = IpAddr::from(<[u8; 16]>::try_from(&header[8..24]).ok()?);
            let destination = IpAddr::from(<[u8; 16]>::try_from(&header[24..40]).ok()?);
            (source, destination, network_offset + 40)
        }
        _ => return None,
    };

    let tcp = bytes.get(tcp_offset..)?;
    let header_len = usize::from(tcp.get(12)? >> 4) * 4;
    if header_len < 20 || tcp.len() < header_len {
        return None;
    }
    let flags = *tcp.get(13)?;

    Some(Segment {
        packet,
        source: Endpoint {
            address: source,
            port: u16::from_be_bytes(tcp.get(0..2)?.try_into().ok()?),
        },
        destination: Endpoint {
            address: destination,
            port: u16::from_be_bytes(tcp.get(2..4)?.try_into().ok()?),
        },
        sequence: u32::from_be_bytes(tcp.get(4..8)?.try_into().ok()?),
        syn: flags & 0x02 != 0,
        ack: flags & 0x10 != 0,
        data: tcp[header_len..].to_vec(),
    })
}

fn assemble_flow(mut segments: Vec<Segment>) -> Result<Flow, ReplayError> {
    segments.retain(|segment| !segment.data.is_empty() || segment.syn);
    for syn in segments.iter().filter(|segment| segment.syn && !segment.ack) {
        let client = syn.source.clone();
        let server = syn.destination.clone();
        let mut client_segments = Vec::new();
        let mut server_segments = Vec::new();

        for segment in &segments {
            if segment.source == client && segment.destination == server {
                client_segments.push(segment.clone());
            } else if segment.source == server && segment.destination == client {
                server_segments.push(segment.clone());
            }
        }

        let client_stream = reassemble(client_segments)?;
        let server_stream = reassemble(server_segments)?;
        if !client_stream.is_empty() && !server_stream.is_empty() && has_x224_connection_request(&client_stream) {
            return Ok(Flow {
                client,
                server,
                client_stream,
                server_stream,
            });
        }
    }

    Err(ReplayError::UnsupportedTransport)
}

fn has_x224_connection_request(stream: &PacketStream) -> bool {
    let bytes = flatten(stream);
    tpkt_frames(&bytes).any(|frame| frame.get(5) == Some(&0xe0))
}

fn reassemble(segments: Vec<Segment>) -> Result<PacketStream, ReplayError> {
    let mut bytes = BTreeMap::new();
    for segment in segments {
        for (offset, byte) in segment.data.into_iter().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| ReplayError::MissingTcpFlow)?;
            let sequence = segment
                .sequence
                .checked_add(offset)
                .ok_or(ReplayError::MissingTcpFlow)?;
            match bytes.entry(sequence) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((segment.packet, byte));
                }
                std::collections::btree_map::Entry::Occupied(entry) if entry.get().1 == byte => {}
                std::collections::btree_map::Entry::Occupied(_) => return Err(ReplayError::MissingTcpFlow),
            }
        }
    }

    let Some((&first, _)) = bytes.first_key_value() else {
        return Ok(Vec::new());
    };

    let mut sequence = first;
    let mut stream = PacketStream::new();
    while let Some((packet, byte)) = bytes.remove(&sequence) {
        if let Some((previous_packet, chunk)) = stream.last_mut()
            && *previous_packet == packet
        {
            chunk.push(byte);
        } else {
            stream.push((packet, vec![byte]));
        }
        sequence = sequence.checked_add(1).ok_or(ReplayError::MissingTcpFlow)?;
    }
    if !bytes.is_empty() {
        return Err(ReplayError::MissingTcpFlow);
    }

    Ok(stream)
}

fn flatten(stream: &PacketStream) -> Vec<u8> {
    stream.iter().flat_map(|(_, bytes)| bytes).copied().collect()
}

fn tpkt_frames(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0;
    core::iter::from_fn(move || {
        while offset + 4 <= bytes.len() {
            if bytes[offset] == 3 && bytes[offset + 1] == 0 {
                let length = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
                let end = offset.checked_add(length)?;
                if length >= 7 && end <= bytes.len() {
                    let frame = &bytes[offset..end];
                    offset = end;
                    return Some(frame);
                }
            }
            offset += 1;
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use core::net::Ipv4Addr;

    use super::*;

    fn endpoint(port: u16) -> Endpoint {
        Endpoint {
            address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        }
    }

    fn segment(sequence: u32, packet: usize, data: &[u8]) -> Segment {
        Segment {
            packet,
            source: endpoint(1),
            destination: endpoint(2),
            sequence,
            syn: false,
            ack: true,
            data: data.to_vec(),
        }
    }

    #[test]
    fn reassembly_deduplicates_retransmissions() {
        let stream = reassemble(vec![
            segment(12, 2, b"world"),
            segment(7, 1, b"hello"),
            segment(7, 3, b"hello"),
        ])
        .unwrap();

        assert_eq!(flatten(&stream), b"helloworld");
        assert_eq!(stream[0].0, 1);
    }

    #[test]
    fn reassembly_rejects_conflicting_overlap() {
        let error = reassemble(vec![segment(1, 1, b"hello"), segment(3, 2, b"XX")]).unwrap_err();

        assert!(matches!(error, ReplayError::MissingTcpFlow));
    }

    #[test]
    fn parses_direct_tcp_flow_from_pcapng() {
        let path = std::env::temp_dir().join(format!("ironrdp-capture-replay-{}.pcapng", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            pcapng([
                ethernet_tcp(1, 2, 100, 0x02, &[]),
                ethernet_tcp(1, 2, 101, 0x10, &[3, 0, 0, 7, 2, 0xe0, 0]),
                ethernet_tcp(2, 1, 200, 0x10, &[0]),
            ]),
        )
        .unwrap();

        let capture = read_capture(&path).unwrap();

        assert_eq!(capture.flow.client.port, 1);
        assert_eq!(capture.flow.server.port, 2);
        assert_eq!(flatten(&capture.flow.client_stream), [3, 0, 0, 7, 2, 0xe0, 0]);
        assert_eq!(flatten(&capture.flow.server_stream), [0]);
        std::fs::remove_file(path).unwrap();
    }

    fn pcapng(frames: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend(block(0x0a0d0d0a, {
            let mut body = Vec::new();
            body.extend(0x1a2b3c4du32.to_le_bytes());
            body.extend(1u16.to_le_bytes());
            body.extend(0u16.to_le_bytes());
            body.extend((-1i64).to_le_bytes());
            body
        }));
        output.extend(block(1, {
            let mut body = Vec::new();
            body.extend(1u16.to_le_bytes());
            body.extend(0u16.to_le_bytes());
            body.extend(u32::MAX.to_le_bytes());
            body
        }));
        for frame in frames {
            let mut body = Vec::new();
            body.extend(0u32.to_le_bytes());
            body.extend(0u32.to_le_bytes());
            body.extend(0u32.to_le_bytes());
            body.extend(u32::try_from(frame.len()).unwrap().to_le_bytes());
            body.extend(u32::try_from(frame.len()).unwrap().to_le_bytes());
            body.extend(frame);
            body.resize((body.len() + 3) & !3, 0);
            output.extend(block(6, body));
        }
        output
    }

    fn block(kind: u32, body: Vec<u8>) -> Vec<u8> {
        let length = u32::try_from(body.len() + 12).unwrap();
        let mut output = Vec::new();
        output.extend(kind.to_le_bytes());
        output.extend(length.to_le_bytes());
        output.extend(body);
        output.extend(length.to_le_bytes());
        output
    }

    fn ethernet_tcp(source_port: u16, destination_port: u16, sequence: u32, flags: u8, data: &[u8]) -> Vec<u8> {
        let mut packet = vec![0; 14 + 20 + 20];
        packet[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        packet[14] = 0x45;
        let length = u16::try_from(20 + 20 + data.len()).unwrap();
        packet[16..18].copy_from_slice(&length.to_be_bytes());
        packet[22] = 64;
        packet[23] = 6;
        let source_address = if source_port == 1 { 1 } else { 2 };
        let destination_address = if destination_port == 1 { 1 } else { 2 };
        packet[26..30].copy_from_slice(&[127, 0, 0, source_address]);
        packet[30..34].copy_from_slice(&[127, 0, 0, destination_address]);
        let tcp = &mut packet[34..];
        tcp[0..2].copy_from_slice(&source_port.to_be_bytes());
        tcp[2..4].copy_from_slice(&destination_port.to_be_bytes());
        tcp[4..8].copy_from_slice(&sequence.to_be_bytes());
        tcp[12] = 0x50;
        tcp[13] = flags;
        packet.extend(data);
        packet
    }
}
