use std::fs::File;
use std::path::Path;

use core::net::IpAddr;
use pcap_parser::pcapng::{Block, SecretsType};
use pcap_parser::traits::{PcapNGPacketBlock as _, PcapReaderIterator as _};
use pcap_parser::{PcapBlockOwned, PcapError, PcapNGReader};
use zeroize::Zeroize as _;

use crate::CaptureError;

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
#[derive(Clone)]
pub struct Capture {
    /// Direct TCP RDP flow selected from the pcapng input.
    pub flow: Flow,
    /// Other TLS flows to TCP port 443, tried when the primary flow is not a
    /// complete RD Gateway tunnel (one WebSocket flow, or paired RPCH IN+OUT).
    pub gateway_alternates: Vec<Flow>,
    /// NSS-compatible TLS key-log entries embedded in the capture.
    pub tls_key_log: TlsKeyLog,
}

impl Capture {
    /// Merge additional NSS-compatible TLS key-log entries.
    ///
    /// Capture exports include secrets only for TLS flows the exporter sees.
    /// Supply the original key log for a TLS session unwrapped from a gateway.
    pub fn add_tls_key_log(&mut self, key_log: &str) {
        if key_log.is_empty() {
            return;
        }
        if !self.tls_key_log.0.is_empty() && !self.tls_key_log.0.ends_with('\n') {
            self.tls_key_log.0.push('\n');
        }
        self.tls_key_log.0.push_str(key_log);
        if !key_log.ends_with('\n') {
            self.tls_key_log.0.push('\n');
        }
    }
}

impl core::fmt::Debug for Capture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Capture")
            .field("flow", &self.flow)
            .field("tls_key_log", &"<redacted>")
            .finish()
    }
}

/// TLS key-log material retained for decrypting a capture.
#[derive(Clone)]
pub struct TlsKeyLog(String);

impl TlsKeyLog {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    /// Get the key log required to decrypt the recorded TLS stream.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for TlsKeyLog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Drop for TlsKeyLog {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug)]
struct Segment {
    packet: usize,
    source: Endpoint,
    destination: Endpoint,
    sequence: u32,
    syn: bool,
    ack: bool,
    fin: bool,
    rst: bool,
    data: Vec<u8>,
}

/// Read an Ethernet direct TCP RDP flow from a pcapng capture.
pub fn read_capture(path: &Path) -> Result<Capture, CaptureError> {
    let file = File::open(path).map_err(CaptureError::Io)?;
    let mut reader = PcapNGReader::new(1024 * 1024, file).map_err(|error| CaptureError::Pcap(error.to_string()))?;
    let mut packet = 0;
    let mut linktypes = Vec::new();
    let mut segments = Vec::new();
    let mut tls_key_log = String::new();

    loop {
        match reader.next() {
            Ok((offset, block)) => {
                match block {
                    PcapBlockOwned::NG(Block::SectionHeader(_)) => {
                        linktypes.clear();
                    }
                    PcapBlockOwned::NG(Block::InterfaceDescription(interface)) => {
                        linktypes.push(interface.linktype.0);
                    }
                    PcapBlockOwned::NG(Block::DecryptionSecrets(secrets))
                        if secrets.secrets_type == SecretsType::TlsKeyLog =>
                    {
                        let length = usize::try_from(secrets.secrets_len)
                            .map_err(|_| CaptureError::Pcap("TLS secret block is too large".to_owned()))?;
                        let data = secrets
                            .data
                            .get(..length)
                            .ok_or_else(|| CaptureError::Pcap("truncated TLS secret block".to_owned()))?;
                        let text = core::str::from_utf8(data)
                            .map_err(|_| CaptureError::Pcap("TLS secret block is not UTF-8".to_owned()))?;
                        tls_key_log.push_str(text);
                    }
                    PcapBlockOwned::NG(Block::EnhancedPacket(packet_block)) => {
                        packet += 1;
                        let interface = usize::try_from(packet_block.if_id)
                            .map_err(|_| CaptureError::Pcap("interface ID is too large".to_owned()))?;
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
            Err(PcapError::Incomplete(_)) => reader.refill().map_err(|error| CaptureError::Pcap(error.to_string()))?,
            Err(error) => return Err(CaptureError::Pcap(error.to_string())),
        }
    }

    let (flow, gateway_alternates) = assemble_flow(segments)?;
    Ok(Capture {
        flow,
        gateway_alternates,
        tls_key_log: TlsKeyLog::new(tls_key_log),
    })
}

fn parse_tcp_packet(packet: usize, bytes: &[u8]) -> Option<Segment> {
    let ethernet_type = u16::from_be_bytes(bytes.get(12..14)?.try_into().ok()?);
    let (network_offset, ethernet_type) = if ethernet_type == 0x8100 {
        (18, u16::from_be_bytes(bytes.get(16..18)?.try_into().ok()?))
    } else {
        (14, ethernet_type)
    };

    let (source, destination, tcp_offset, network_end) = match ethernet_type {
        0x0800 => {
            let header = bytes.get(network_offset..)?;
            let header_len = usize::from(header.first()? & 0x0f) * 4;
            if header_len < 20 || *header.get(9)? != 6 {
                return None;
            }
            let total_len = usize::from(u16::from_be_bytes(header.get(2..4)?.try_into().ok()?));
            if total_len != 0 && total_len < header_len {
                return None;
            }
            // Some NIC offload captures leave the IPv4 total length unset.
            // In that case, bound parsing to the captured frame extent.
            let network_end = if total_len == 0 {
                // A minimum Ethernet payload can be link-layer padding, not TCP data.
                let minimum_frame_len = network_offset.checked_add(46)?;
                if bytes.len() == minimum_frame_len || bytes.len() == minimum_frame_len.checked_add(4)? {
                    return None;
                }
                bytes.len()
            } else {
                network_offset.checked_add(total_len)?
            };
            bytes.get(network_offset..network_end)?;
            let source = IpAddr::from(<[u8; 4]>::try_from(header.get(12..16)?).ok()?);
            let destination = IpAddr::from(<[u8; 4]>::try_from(header.get(16..20)?).ok()?);
            (source, destination, network_offset + header_len, network_end)
        }
        0x86dd => {
            let header = bytes.get(network_offset..network_offset + 40)?;
            let payload_len = usize::from(u16::from_be_bytes(header.get(4..6)?.try_into().ok()?));
            let network_end = network_offset.checked_add(40 + payload_len)?;
            bytes.get(network_offset..network_end)?;
            let source = IpAddr::from(<[u8; 16]>::try_from(&header[8..24]).ok()?);
            let destination = IpAddr::from(<[u8; 16]>::try_from(&header[24..40]).ok()?);
            let mut next_header = header[6];
            let mut tcp_offset = network_offset + 40;
            while matches!(next_header, 0 | 43 | 60) {
                let extension = bytes.get(tcp_offset..network_end)?;
                let length = (usize::from(*extension.get(1)?) + 1) * 8;
                next_header = *extension.first()?;
                tcp_offset = tcp_offset.checked_add(length)?;
            }
            if next_header != 6 {
                return None;
            }
            (source, destination, tcp_offset, network_end)
        }
        _ => return None,
    };

    let tcp = bytes.get(tcp_offset..network_end)?;
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
        fin: flags & 0x01 != 0,
        rst: flags & 0x04 != 0,
        data: tcp[header_len..].to_vec(),
    })
}

fn assemble_flow(mut segments: Vec<Segment>) -> Result<(Flow, Vec<Flow>), CaptureError> {
    segments.retain(|segment| !segment.data.is_empty() || segment.syn || segment.fin || segment.rst);
    let mut missing_tcp_flow = false;
    let mut gateways = Vec::new();
    for (start, syn) in segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| segment.syn && !segment.ack)
    {
        let client = syn.source.clone();
        let server = syn.destination.clone();
        let mut client_segments = Vec::new();
        let mut server_segments = Vec::new();
        let mut client_closed = false;
        let mut server_closed = false;

        for segment in segments.iter().skip(start) {
            if segment.syn
                && !segment.ack
                && segment.packet != syn.packet
                && segment.source == client
                && segment.destination == server
            {
                break;
            }
            if segment.source == client && segment.destination == server {
                if !client_closed {
                    client_segments.push(segment.clone());
                    client_closed |= segment.fin;
                }
            } else if segment.source == server && segment.destination == client {
                if !server_closed {
                    server_segments.push(segment.clone());
                    server_closed |= segment.fin;
                }
            } else {
                continue;
            }
            if segment.rst || (client_closed && server_closed) {
                break;
            }
        }

        let client_origin = syn.sequence.wrapping_add(1);
        let server_origin = server_segments
            .iter()
            .find(|segment| segment.syn)
            .map(|segment| segment.sequence.wrapping_add(1))
            .or_else(|| {
                server_segments
                    .iter()
                    .find(|segment| !segment.data.is_empty())
                    .map(|segment| segment.sequence)
            })
            .unwrap_or(0);
        let Ok(client_stream) = reassemble(client_segments, client_origin) else {
            missing_tcp_flow = true;
            continue;
        };
        let Ok(server_stream) = reassemble(server_segments, server_origin) else {
            missing_tcp_flow = true;
            continue;
        };
        if has_x224_connection_initiation(&client_stream, &server_stream) {
            return Ok((
                Flow {
                    client,
                    server,
                    client_stream,
                    server_stream,
                },
                Vec::new(),
            ));
        }
        if server.port == 443 {
            gateways.push(Flow {
                client,
                server,
                client_stream,
                server_stream,
            });
        }
    }

    let mut gateways = gateways.into_iter();
    if let Some(flow) = gateways.next() {
        return Ok((flow, gateways.collect()));
    }

    Err(if missing_tcp_flow {
        CaptureError::MissingTcpFlow
    } else {
        CaptureError::UnsupportedTransport
    })
}

fn has_x224_connection_initiation(client_stream: &PacketStream, server_stream: &PacketStream) -> bool {
    x224_connection_tpdu_end(&flatten(client_stream), 0xe0).is_some()
        && x224_connection_tpdu_end(&flatten(server_stream), 0xd0).is_some()
}

pub(crate) fn x224_connection_tpdu_end(bytes: &[u8], code: u8) -> Option<usize> {
    let header = bytes.get(..4)?;
    if header[0] != 3 || header[1] != 0 {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let frame = bytes.get(..length)?;

    // MS-RDPBCGR 2.2.1.1 and 2.2.1.2 include optional connection fields in LI.
    let li = usize::from(*frame.get(4)?);
    (6 <= li
        && li < usize::from(u8::MAX)
        && li.checked_add(5)? == frame.len()
        && frame.get(5) == Some(&code)
        && frame.get(10) == Some(&0))
    .then_some(length)
}

fn reassemble(segments: Vec<Segment>, origin: u32) -> Result<PacketStream, CaptureError> {
    let mut segments = segments
        .into_iter()
        .filter(|segment| !segment.data.is_empty())
        .map(|segment| {
            (
                segment.packet,
                segment
                    .sequence
                    .wrapping_add(u32::from(segment.syn))
                    .wrapping_sub(origin),
                segment.data,
            )
        })
        .collect::<Vec<_>>();
    segments.sort_by_key(|(_, sequence, _)| *sequence);
    let mut segments = segments.into_iter();
    let Some((packet, start_sequence, data)) = segments.next() else {
        return Ok(Vec::new());
    };

    let mut bytes = data.clone();
    let mut stream = vec![(packet, data)];
    // INVARIANT: segments are sorted by non-decreasing sequence number, so each sequence is at least start_sequence.
    for (packet, sequence, data) in segments {
        let data_end = sequence
            .checked_add(u32::try_from(data.len()).map_err(|_| CaptureError::MissingTcpFlow)?)
            .ok_or(CaptureError::MissingTcpFlow)?;
        let stream_end = start_sequence
            .checked_add(u32::try_from(bytes.len()).map_err(|_| CaptureError::MissingTcpFlow)?)
            .ok_or(CaptureError::MissingTcpFlow)?;
        if stream_end < sequence {
            return Err(CaptureError::MissingTcpFlow);
        }
        // The sequence ordering invariant above guarantees this subtraction does not underflow.
        let offset = usize::try_from(sequence - start_sequence).map_err(|_| CaptureError::MissingTcpFlow)?;
        let overlap = usize::try_from(stream_end - sequence)
            .map_err(|_| CaptureError::MissingTcpFlow)?
            .min(data.len());
        if bytes[offset..offset + overlap] != data[..overlap] {
            return Err(CaptureError::MissingTcpFlow);
        }
        if stream_end < data_end {
            let data = data[overlap..].to_vec();
            bytes.extend_from_slice(&data);
            if let Some((previous_packet, previous_data)) = stream.last_mut()
                && *previous_packet == packet
            {
                previous_data.extend(data);
            } else {
                stream.push((packet, data));
            }
        }
    }

    Ok(stream)
}

/// Concatenate the chunks in a packet stream.
pub fn flatten(stream: &PacketStream) -> Vec<u8> {
    stream.iter().flat_map(|(_, bytes)| bytes).copied().collect()
}
