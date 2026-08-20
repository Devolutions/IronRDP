use std::fs::File;
use std::path::Path;

use core::net::IpAddr;
use pcap_parser::pcapng::{Block, SecretsType};
use pcap_parser::traits::{PcapNGPacketBlock as _, PcapReaderIterator as _};
use pcap_parser::{PcapBlockOwned, PcapError, PcapNGReader};
use zeroize::Zeroize as _;

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
#[derive(Clone)]
pub struct Capture {
    /// Direct TCP RDP flow selected from the pcapng input.
    pub flow: Flow,
    /// NSS-compatible TLS key-log entries embedded in the capture.
    pub tls_key_log: TlsKeyLog,
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
pub fn read_capture(path: &Path) -> Result<Capture, ReplayError> {
    let file = File::open(path).map_err(ReplayError::Io)?;
    let mut reader = PcapNGReader::new(1024 * 1024, file).map_err(|error| ReplayError::Pcap(error.to_string()))?;
    let mut packet = 0;
    let mut linktypes = Vec::new();
    let mut has_ethernet = false;
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
                        has_ethernet |= interface.linktype.0 == ETHERNET_LINKTYPE;
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
                        packet += 1;
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
            Err(PcapError::Incomplete(_)) => reader.refill().map_err(|error| ReplayError::Pcap(error.to_string()))?,
            Err(error) => return Err(ReplayError::Pcap(error.to_string())),
        }
    }

    if !has_ethernet {
        return Err(ReplayError::UnsupportedTransport);
    }

    Ok(Capture {
        flow: assemble_flow(segments)?,
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

fn assemble_flow(mut segments: Vec<Segment>) -> Result<Flow, ReplayError> {
    segments.retain(|segment| !segment.data.is_empty() || segment.syn || segment.fin || segment.rst);
    let mut missing_tcp_flow = false;
    let mut gateway = None;
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
            return Ok(Flow {
                client,
                server,
                client_stream,
                server_stream,
            });
        }
        if server.port == 443 && gateway.is_none() {
            gateway = Some(Flow {
                client,
                server,
                client_stream,
                server_stream,
            });
        }
    }

    if let Some(flow) = gateway {
        return Ok(flow);
    }

    Err(if missing_tcp_flow {
        ReplayError::MissingTcpFlow
    } else {
        ReplayError::UnsupportedTransport
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
    (li >= 6
        && li < usize::from(u8::MAX)
        && li.checked_add(5)? == frame.len()
        && frame.get(5) == Some(&code)
        && frame.get(10) == Some(&0))
    .then_some(length)
}

fn reassemble(segments: Vec<Segment>, origin: u32) -> Result<PacketStream, ReplayError> {
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
    for (packet, sequence, data) in segments {
        let data_end = sequence
            .checked_add(u32::try_from(data.len()).map_err(|_| ReplayError::MissingTcpFlow)?)
            .ok_or(ReplayError::MissingTcpFlow)?;
        let stream_end = start_sequence
            .checked_add(u32::try_from(bytes.len()).map_err(|_| ReplayError::MissingTcpFlow)?)
            .ok_or(ReplayError::MissingTcpFlow)?;
        if sequence > stream_end {
            return Err(ReplayError::MissingTcpFlow);
        }
        let offset = usize::try_from(sequence - start_sequence).map_err(|_| ReplayError::MissingTcpFlow)?;
        let overlap = usize::try_from(stream_end - sequence)
            .map_err(|_| ReplayError::MissingTcpFlow)?
            .min(data.len());
        if bytes[offset..offset + overlap] != data[..overlap] {
            return Err(ReplayError::MissingTcpFlow);
        }
        if data_end > stream_end {
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

pub(crate) fn flatten(stream: &PacketStream) -> Vec<u8> {
    stream.iter().flat_map(|(_, bytes)| bytes).copied().collect()
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
        segment_between(endpoint(1), endpoint(2), sequence, packet, false, false, data)
    }

    fn segment_between(
        source: Endpoint,
        destination: Endpoint,
        sequence: u32,
        packet: usize,
        syn: bool,
        ack: bool,
        data: &[u8],
    ) -> Segment {
        Segment {
            packet,
            source,
            destination,
            sequence,
            syn,
            ack,
            fin: false,
            rst: false,
            data: data.to_vec(),
        }
    }

    fn connection_request() -> [u8; 19] {
        [3, 0, 0, 19, 14, 0xe0, 0, 0, 0, 0, 0, 1, 0, 8, 0, 1, 0, 0, 0]
    }

    fn connection_confirm() -> [u8; 19] {
        [3, 0, 0, 19, 14, 0xd0, 0, 0, 0x12, 0x34, 0, 2, 0, 8, 0, 1, 0, 0, 0]
    }

    #[test]
    fn reassembly_deduplicates_retransmissions() {
        let stream = reassemble(
            vec![
                segment(12, 2, b"world"),
                segment(7, 1, b"hello"),
                segment(7, 3, b"hello"),
            ],
            7,
        )
        .unwrap();

        assert_eq!(flatten(&stream), b"helloworld");
        assert_eq!(stream[0].0, 1);
    }

    #[test]
    fn reassembly_rejects_conflicting_overlap() {
        let error = reassemble(vec![segment(1, 1, b"hello"), segment(3, 2, b"XX")], 1).unwrap_err();

        assert!(matches!(error, ReplayError::MissingTcpFlow));
    }

    #[test]
    fn reassembly_handles_wrapped_sequence_numbers() {
        let stream = reassemble(
            vec![segment(u32::MAX - 1, 1, b"abcd"), segment(2, 2, b"ef")],
            u32::MAX - 1,
        )
        .unwrap();

        assert_eq!(flatten(&stream), b"abcdef");
    }

    #[test]
    fn rejects_invalid_x224_connection_initiation() {
        let client = vec![3, 0, 0, 7, 2, 0xe0, 0];
        let server = connection_confirm();

        assert!(!has_x224_connection_initiation(
            &vec![(1, client)],
            &vec![(2, server.to_vec())]
        ));
    }

    #[test]
    fn rejects_x224_connection_request_after_leading_data() {
        let client = [b"not-rdp".as_slice(), connection_request().as_slice()].concat();
        let server = connection_confirm();

        assert!(!has_x224_connection_initiation(
            &vec![(1, client)],
            &vec![(2, server.to_vec())]
        ));
    }

    #[test]
    fn accepts_variable_length_x224_connection_request() {
        let mut request = Vec::from([
            3, 0, 0, 47, 42, 0xe0, 0, 0, 0, 0, 0, b'C', b'o', b'o', b'k', b'i', b'e', b':', b' ', b'm', b's', b't',
            b's', b'h', b'a', b's', b'h', b'=', b's', b'y', b'n', b't', b'h', b'e', b't', b'i', b'c', b'\r', b'\n',
        ]);
        request.extend([1, 0, 8, 0, 3, 0, 0, 0]);

        assert_eq!(x224_connection_tpdu_end(&request, 0xe0), Some(47));
    }

    #[test]
    fn selects_a_gateway_port_flow_without_x224() {
        let client = endpoint(1);
        let server = endpoint(443);
        let flow = assemble_flow(vec![
            segment_between(client.clone(), server.clone(), 100, 1, true, false, &[]),
            segment_between(client.clone(), server.clone(), 101, 2, false, true, b"CLIENT"),
            segment_between(server, client, 200, 3, false, true, b"SERVER"),
        ])
        .unwrap();

        assert_eq!(flow.server.port, 443);
        assert_eq!(flatten(&flow.client_stream), b"CLIENT");
        assert_eq!(flatten(&flow.server_stream), b"SERVER");
    }

    #[test]
    fn prefers_an_x224_flow_over_a_gateway_port_flow() {
        let gateway_client = endpoint(1);
        let gateway_server = endpoint(443);
        let rdp_client = endpoint(3);
        let rdp_server = endpoint(4);
        let flow = assemble_flow(vec![
            segment_between(gateway_client.clone(), gateway_server.clone(), 100, 1, true, false, &[]),
            segment_between(
                gateway_client.clone(),
                gateway_server.clone(),
                101,
                2,
                false,
                true,
                b"HTTPS",
            ),
            segment_between(gateway_server, gateway_client, 200, 3, false, true, b"HTTPS"),
            segment_between(rdp_client.clone(), rdp_server.clone(), 300, 4, true, false, &[]),
            segment_between(
                rdp_client.clone(),
                rdp_server.clone(),
                301,
                5,
                false,
                true,
                &connection_request(),
            ),
            segment_between(rdp_server, rdp_client, 400, 6, false, true, &connection_confirm()),
        ])
        .unwrap();

        assert_eq!(flow.client.port, 3);
        assert_eq!(flow.server.port, 4);
    }

    #[test]
    fn skips_incomplete_flows_before_a_valid_rdp_connection() {
        let first_client = endpoint(1);
        let first_server = endpoint(2);
        let second_client = endpoint(3);
        let second_server = endpoint(4);
        let flow = assemble_flow(vec![
            segment_between(first_client.clone(), first_server.clone(), 100, 1, true, false, &[]),
            segment_between(first_client.clone(), first_server.clone(), 101, 2, false, true, b"bad"),
            segment_between(first_client, first_server, 101, 3, false, true, b"XX"),
            segment_between(second_client.clone(), second_server.clone(), 200, 4, true, false, &[]),
            segment_between(
                second_client.clone(),
                second_server.clone(),
                201,
                5,
                false,
                true,
                &connection_request(),
            ),
            segment_between(second_server, second_client, 300, 6, false, true, &connection_confirm()),
        ])
        .unwrap();

        assert_eq!(flow.client.port, 3);
        assert_eq!(flow.server.port, 4);
    }

    #[test]
    fn reports_missing_tcp_flow_when_no_candidate_reassembles() {
        let client = endpoint(1);
        let server = endpoint(2);
        let request = connection_request();
        let error = assemble_flow(vec![
            segment_between(client.clone(), server.clone(), 100, 1, true, false, &[]),
            segment_between(client.clone(), server.clone(), 101, 2, false, true, &request[..8]),
            segment_between(
                client,
                server,
                101 + u32::try_from(request.len()).unwrap(),
                3,
                false,
                true,
                &request[8..],
            ),
        ])
        .unwrap_err();

        assert!(matches!(error, ReplayError::MissingTcpFlow));
    }

    #[test]
    fn separates_connections_that_reuse_a_tcp_tuple() {
        let client = endpoint(1);
        let server = endpoint(2);
        let flow = assemble_flow(vec![
            segment_between(client.clone(), server.clone(), 100, 1, true, false, &[]),
            segment_between(client.clone(), server.clone(), 101, 2, false, true, b"old"),
            segment_between(server.clone(), client.clone(), 300, 3, false, true, b"old"),
            segment_between(client.clone(), server.clone(), 500, 4, true, false, &[]),
            segment_between(
                client.clone(),
                server.clone(),
                501,
                5,
                false,
                true,
                &connection_request(),
            ),
            segment_between(server, client, 700, 6, false, true, &connection_confirm()),
        ])
        .unwrap();

        assert_eq!(flow.client_stream[0].0, 5);
        assert_eq!(flow.server_stream[0].0, 6);
    }

    #[test]
    fn preserves_peer_data_after_a_half_close() {
        let client = endpoint(1);
        let server = endpoint(2);
        let request = connection_request();
        let confirm = connection_confirm();
        let mut client_fin = segment_between(
            client.clone(),
            server.clone(),
            101 + u32::try_from(request.len()).unwrap(),
            3,
            false,
            true,
            &[],
        );
        client_fin.fin = true;
        let mut server_fin = segment_between(
            server.clone(),
            client.clone(),
            300 + u32::try_from(confirm.len() + 11).unwrap(),
            6,
            false,
            true,
            &[],
        );
        server_fin.fin = true;
        let flow = assemble_flow(vec![
            segment_between(client.clone(), server.clone(), 100, 1, true, false, &[]),
            segment_between(client.clone(), server.clone(), 101, 2, false, true, &request),
            client_fin,
            segment_between(server.clone(), client.clone(), 300, 4, false, true, &confirm),
            segment_between(
                server,
                client,
                300 + u32::try_from(confirm.len()).unwrap(),
                5,
                false,
                true,
                b"firstsecond",
            ),
            server_fin,
        ])
        .unwrap();

        assert_eq!(
            flatten(&flow.server_stream),
            [confirm.as_slice(), b"firstsecond"].concat()
        );
    }

    #[test]
    fn parses_direct_tcp_flow_from_pcapng() {
        let path = std::env::temp_dir().join(format!("ironrdp-capture-replay-{}.pcapng", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            pcapng([
                ethernet_tcp(1, 2, 100, 0x02, &[]),
                ethernet_tcp(1, 2, 101, 0x10, &connection_request()),
                ethernet_tcp(2, 1, 200, 0x10, &connection_confirm()),
            ]),
        )
        .unwrap();

        let capture = read_capture(&path).unwrap();

        assert_eq!(capture.flow.client.port, 1);
        assert_eq!(capture.flow.server.port, 2);
        assert_eq!(capture.flow.client_stream[0].0, 2);
        assert_eq!(capture.flow.server_stream[0].0, 3);
        assert_eq!(flatten(&capture.flow.client_stream), connection_request());
        assert_eq!(flatten(&capture.flow.server_stream), connection_confirm());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_unpadded_tls_key_log_entries() {
        let path = std::env::temp_dir().join(format!("ironrdp-capture-replay-secrets-{}.pcapng", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            pcapng_with_key_log(
                [
                    ethernet_tcp(1, 2, 100, 0x02, &[]),
                    ethernet_tcp(1, 2, 101, 0x10, &connection_request()),
                    ethernet_tcp(2, 1, 200, 0x10, &connection_confirm()),
                ],
                "CLIENT_RANDOM synthetic secret\n",
            ),
        )
        .unwrap();

        let capture = read_capture(&path).unwrap();

        assert_eq!(capture.tls_key_log.as_str(), "CLIENT_RANDOM synthetic secret\n");
        assert!(!format!("{capture:?}").contains("synthetic secret"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn ignores_padding_after_the_ipv4_packet() {
        let mut packet = ethernet_tcp(1, 2, 100, 0x10, b"payload");
        packet.extend([0; 16]);

        let segment = parse_tcp_packet(1, &packet).unwrap();

        assert_eq!(segment.data, b"payload");
    }

    #[test]
    fn parses_zero_length_ipv4_using_captured_extent() {
        let mut packet = ethernet_tcp(1, 2, 100, 0x10, b"payload");
        packet[16..18].copy_from_slice(&[0; 2]);

        let segment = parse_tcp_packet(1, &packet).unwrap();

        assert_eq!(segment.data, b"payload");
    }

    #[test]
    fn rejects_ambiguous_zero_length_ipv4_ethernet_padding() {
        let mut packet = ethernet_tcp(1, 2, 100, 0x10, &[]);
        packet[16..18].copy_from_slice(&[0; 2]);
        packet.extend([0; 6]);

        assert!(parse_tcp_packet(1, &packet).is_none());
    }

    #[test]
    fn rejects_zero_length_ipv4_without_a_complete_tcp_header() {
        let mut packet = ethernet_tcp(1, 2, 100, 0x10, b"payload");
        packet[16..18].copy_from_slice(&[0; 2]);
        packet.truncate(14 + 20 + 19);

        assert!(parse_tcp_packet(1, &packet).is_none());
    }

    #[test]
    fn parses_tcp_after_an_ipv6_hop_by_hop_header() {
        let segment = parse_tcp_packet(1, &ethernet_ipv6_tcp_with_hop_by_hop(1, 2, 100, b"payload")).unwrap();

        assert_eq!(segment.data, b"payload");
    }

    fn pcapng(frames: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
        pcapng_with_key_log(frames, "")
    }

    fn pcapng_with_key_log(frames: impl IntoIterator<Item = Vec<u8>>, tls_key_log: &str) -> Vec<u8> {
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
        if !tls_key_log.is_empty() {
            let mut body = Vec::new();
            body.extend(0x544c534bu32.to_le_bytes());
            body.extend(u32::try_from(tls_key_log.len()).unwrap().to_le_bytes());
            body.extend(tls_key_log.as_bytes());
            body.resize((body.len() + 3) & !3, 0);
            output.extend(block(0x0a, body));
        }
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

    fn ethernet_ipv6_tcp_with_hop_by_hop(
        source_port: u16,
        destination_port: u16,
        sequence: u32,
        data: &[u8],
    ) -> Vec<u8> {
        let mut packet = vec![0; 14 + 40 + 8 + 20];
        packet[12..14].copy_from_slice(&0x86ddu16.to_be_bytes());
        packet[14] = 0x60;
        let payload_length = u16::try_from(8 + 20 + data.len()).unwrap();
        packet[18..20].copy_from_slice(&payload_length.to_be_bytes());
        packet[20] = 0;
        packet[21] = 64;
        packet[29] = if source_port == 1 { 1 } else { 2 };
        packet[45] = if destination_port == 1 { 1 } else { 2 };
        packet[54] = 6;
        let tcp = &mut packet[62..];
        tcp[0..2].copy_from_slice(&source_port.to_be_bytes());
        tcp[2..4].copy_from_slice(&destination_port.to_be_bytes());
        tcp[4..8].copy_from_slice(&sequence.to_be_bytes());
        tcp[12] = 0x50;
        tcp[13] = 0x10;
        packet.extend(data);
        packet
    }
}
