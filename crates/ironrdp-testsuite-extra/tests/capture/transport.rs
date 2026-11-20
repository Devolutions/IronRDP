use std::path::PathBuf;

use ironrdp_capture::{CaptureError, flatten, read_capture};

fn temporary_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ironrdp-capture-{name}-{}.pcapng", std::process::id()))
}

fn capture(frames: impl IntoIterator<Item = Vec<u8>>, key_log: Option<&str>) -> Vec<u8> {
    let mut output = block(
        0x0a0d_0d0a,
        [
            0x4d, 0x3c, 0x2b, 0x1a, 1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ],
    );
    output.extend(block(1, [1, 0, 0, 0, 0xff, 0xff, 0, 0]));
    if let Some(key_log) = key_log {
        let mut body = Vec::new();
        body.extend(0x544c534bu32.to_le_bytes());
        body.extend(
            u32::try_from(key_log.len())
                .expect("key log length fits u32")
                .to_le_bytes(),
        );
        body.extend(key_log.as_bytes());
        body.resize((body.len() + 3) & !3, 0);
        output.extend(block(0x0a, body));
    }
    for frame in frames {
        let length = u32::try_from(frame.len()).expect("frame length fits u32");
        let mut body = Vec::new();
        body.extend(0u32.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend(length.to_le_bytes());
        body.extend(length.to_le_bytes());
        body.extend(frame);
        body.resize((body.len() + 3) & !3, 0);
        output.extend(block(6, body));
    }
    output
}

fn block(kind: u32, body: impl AsRef<[u8]>) -> Vec<u8> {
    let body = body.as_ref();
    let length = u32::try_from(body.len() + 12).expect("block length fits u32");
    let mut output = Vec::new();
    output.extend(kind.to_le_bytes());
    output.extend(length.to_le_bytes());
    output.extend(body);
    output.extend(length.to_le_bytes());
    output
}

pub(super) fn ethernet_tcp(source_port: u16, destination_port: u16, sequence: u32, flags: u8, data: &[u8]) -> Vec<u8> {
    let mut packet = vec![
        0;
        14 /* Ethernet */
            + 20 /* IPv4 */
            + 20 /* TCP */
    ];
    packet[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    packet[14] = 0x45;
    packet[16..18].copy_from_slice(
        &u16::try_from(20 + 20 + data.len())
            .expect("IPv4 packet length fits u16")
            .to_be_bytes(),
    );
    packet[22] = 64;
    packet[23] = 6;
    packet[26..30].copy_from_slice(&[127, 0, 0, if source_port == 1 { 1 } else { 2 }]);
    packet[30..34].copy_from_slice(&[127, 0, 0, if destination_port == 1 { 1 } else { 2 }]);
    let tcp = &mut packet[34..];
    tcp[0..2].copy_from_slice(&source_port.to_be_bytes());
    tcp[2..4].copy_from_slice(&destination_port.to_be_bytes());
    tcp[4..8].copy_from_slice(&sequence.to_be_bytes());
    tcp[12] = 0x50;
    tcp[13] = flags;
    packet.extend(data);
    packet
}

pub(super) fn request() -> [u8; 19] {
    [3, 0, 0, 19, 14, 0xe0, 0, 0, 0, 0, 0, 1, 0, 8, 0, 1, 0, 0, 0]
}

pub(super) fn confirm() -> [u8; 19] {
    [3, 0, 0, 19, 14, 0xd0, 0, 0, 0x12, 0x34, 0, 2, 0, 8, 0, 1, 0, 0, 0]
}

pub(super) fn read(
    name: &str,
    frames: impl IntoIterator<Item = Vec<u8>>,
    key_log: Option<&str>,
) -> ironrdp_capture::Capture {
    let path = temporary_path(name);
    std::fs::write(&path, capture(frames, key_log)).expect("write synthetic capture");
    let result = read_capture(&path);
    std::fs::remove_file(path).expect("remove synthetic capture");
    result.expect("read synthetic capture")
}

fn read_error(name: &str, frames: impl IntoIterator<Item = Vec<u8>>) -> CaptureError {
    let path = temporary_path(name);
    std::fs::write(&path, capture(frames, None)).expect("write synthetic capture");
    let result = read_capture(&path);
    std::fs::remove_file(path).expect("remove synthetic capture");
    result.expect_err("synthetic capture is rejected")
}

#[test]
// oracle: regression #2153; RFC 9293 §3.4 sequence ordering and duplicate detection
fn reassembles_retransmissions_and_preserves_packet_provenance() {
    let capture = read(
        "retransmission",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 111, 0x10, &request()[10..]),
            ethernet_tcp(1, 2, 101, 0x10, &request()[..10]),
            ethernet_tcp(1, 2, 101, 0x10, &request()[..10]),
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(capture.flow.client_stream[0].0, 3);
    assert_eq!(flatten(&capture.flow.client_stream), request());
}

#[test]
// oracle: mutant accepting contradictory bytes for the same TCP sequence range
fn rejects_conflicting_tcp_overlaps() {
    assert!(matches!(
        read_error(
            "conflicting-overlap",
            [
                ethernet_tcp(1, 2, 100, 0x02, &[]),
                ethernet_tcp(1, 2, 101, 0x10, &request()),
                ethernet_tcp(1, 2, 105, 0x10, b"conflict"),
            ],
        ),
        CaptureError::MissingTcpFlow
    ));
}

#[test]
// oracle: spec RFC 9293 §3.4
fn reassembles_wrapped_tcp_sequence_numbers() {
    let capture = read(
        "wrapped-sequence",
        [
            ethernet_tcp(1, 2, u32::MAX - 1, 0x02, &[]),
            ethernet_tcp(1, 2, u32::MAX, 0x10, &request()[..3]),
            ethernet_tcp(1, 2, 2, 0x10, &request()[3..]),
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(flatten(&capture.flow.client_stream), request());
}

#[test]
// oracle: mutant choosing a gateway candidate before a direct RDP flow
fn selects_direct_rdp_before_gateway_flows() {
    let capture = read(
        "flow-selection",
        [
            ethernet_tcp(1, 443, 100, 0x02, &[]),
            ethernet_tcp(1, 443, 101, 0x10, b"KDC"),
            ethernet_tcp(443, 1, 200, 0x10, b"KDC"),
            ethernet_tcp(3, 4, 300, 0x02, &[]),
            ethernet_tcp(3, 4, 301, 0x10, &request()),
            ethernet_tcp(4, 3, 400, 0x10, &confirm()),
            ethernet_tcp(2, 443, 500, 0x02, &[]),
            ethernet_tcp(2, 443, 501, 0x10, b"RDG"),
            ethernet_tcp(443, 2, 600, 0x10, b"RDG"),
        ],
        None,
    );

    assert_eq!(capture.flow.client.port, 3);
    assert!(capture.gateway_alternates.is_empty());
}

#[test]
// oracle: regression #2153 alternate flow payload preservation
fn retains_later_gateway_flows_as_alternates() {
    let capture = read(
        "gateway-alternates",
        [
            ethernet_tcp(1, 443, 100, 0x02, &[]),
            ethernet_tcp(1, 443, 101, 0x10, b"KDC"),
            ethernet_tcp(443, 1, 200, 0x10, b"KDC"),
            ethernet_tcp(2, 443, 300, 0x02, &[]),
            ethernet_tcp(2, 443, 301, 0x10, b"RDG"),
            ethernet_tcp(443, 2, 400, 0x10, b"RDG"),
        ],
        None,
    );

    assert_eq!(capture.flow.client.port, 1);
    assert_eq!(capture.gateway_alternates.len(), 1);
    assert_eq!(capture.gateway_alternates[0].client.port, 2);
    assert_eq!(flatten(&capture.flow.client_stream), b"KDC");
    assert_eq!(flatten(&capture.gateway_alternates[0].client_stream), b"RDG");
}

#[test]
// oracle: mutant dropping embedded key-log bytes or exposing them through Debug
fn reads_unpadded_tls_key_log_and_redacts_it_from_debug_output() {
    let capture = read(
        "key-log",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, &request()),
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        Some("CLIENT_RANDOM synthetic secret\n"),
    );

    assert_eq!(capture.tls_key_log.as_str(), "CLIENT_RANDOM synthetic secret\n");
    assert!(!format!("{capture:?}").contains("synthetic secret"));
}

#[test]
// oracle: spec MS-RDPBCGR §2.2.1.1
fn rejects_invalid_x224_connection_initiation() {
    let capture = read(
        "invalid-x224",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, &[3, 0, 0, 7, 2, 0xe0, 0]),
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
            ethernet_tcp(3, 443, 300, 0x02, &[]),
            ethernet_tcp(3, 443, 301, 0x10, b"CLIENT"),
            ethernet_tcp(443, 3, 400, 0x10, b"SERVER"),
        ],
        None,
    );

    assert_eq!(capture.flow.server.port, 443);
}

#[test]
// oracle: spec MS-RDPBCGR §2.2.1.1
fn rejects_x224_connection_request_after_leading_data() {
    let capture = read(
        "leading-data",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, &[b"not-rdp".as_slice(), request().as_slice()].concat()),
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
            ethernet_tcp(3, 443, 300, 0x02, &[]),
            ethernet_tcp(3, 443, 301, 0x10, b"CLIENT"),
            ethernet_tcp(443, 3, 400, 0x10, b"SERVER"),
        ],
        None,
    );

    assert_eq!(capture.flow.server.port, 443);
}

#[test]
// oracle: spec MS-RDPBCGR §2.2.1.1
fn accepts_variable_length_x224_connection_request() {
    let mut request = Vec::from([
        3, 0, 0, 47, 42, 0xe0, 0, 0, 0, 0, 0, b'C', b'o', b'o', b'k', b'i', b'e', b':', b' ', b'm', b's', b't', b's',
        b'h', b'a', b's', b'h', b'=', b's', b'y', b'n', b't', b'h', b'e', b't', b'i', b'c', b'\r', b'\n',
    ]);
    request.extend([1, 0, 8, 0, 3, 0, 0, 0]);

    let capture = read(
        "variable-x224",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, &request),
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(flatten(&capture.flow.client_stream), request);
}

#[test]
// oracle: mutant stopping at a broken flow before a valid RDP candidate
fn skips_incomplete_flows_before_a_valid_rdp_connection() {
    let capture = read(
        "incomplete-before-rdp",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, b"bad"),
            ethernet_tcp(1, 2, 101, 0x10, b"XX"),
            ethernet_tcp(3, 4, 200, 0x02, &[]),
            ethernet_tcp(3, 4, 201, 0x10, &request()),
            ethernet_tcp(4, 3, 300, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(capture.flow.client.port, 3);
    assert_eq!(capture.flow.server.port, 4);
}

#[test]
// oracle: mutant misclassifying an incomplete TCP candidate as unsupported transport
fn reports_missing_tcp_flow_when_no_candidate_reassembles() {
    assert!(matches!(
        read_error(
            "unreassembled",
            [
                ethernet_tcp(1, 2, 100, 0x02, &[]),
                ethernet_tcp(1, 2, 101, 0x10, &request()[..8]),
                ethernet_tcp(
                    1,
                    2,
                    101 + u32::try_from(request().len()).expect("length fits u32"),
                    0x10,
                    &request()[8..]
                ),
            ],
        ),
        CaptureError::MissingTcpFlow
    ));
}

#[test]
// oracle: spec RFC 9293 §3.4.1
fn separates_connections_that_reuse_a_tcp_tuple() {
    let capture = read(
        "reused-tuple",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, b"old"),
            ethernet_tcp(2, 1, 300, 0x10, b"old"),
            ethernet_tcp(1, 2, 500, 0x02, &[]),
            ethernet_tcp(1, 2, 501, 0x10, &request()),
            ethernet_tcp(2, 1, 700, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(capture.flow.client_stream[0].0, 5);
    assert_eq!(capture.flow.server_stream[0].0, 6);
}

#[test]
// oracle: spec RFC 9293 §3.6.1
fn preserves_peer_data_after_a_half_close() {
    let request = request();
    let confirm = confirm();
    let capture = read(
        "half-close",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            ethernet_tcp(1, 2, 101, 0x10, &request),
            ethernet_tcp(
                1,
                2,
                101 + u32::try_from(request.len()).expect("length fits u32"),
                0x11,
                &[],
            ),
            ethernet_tcp(2, 1, 300, 0x10, &confirm),
            ethernet_tcp(
                2,
                1,
                300 + u32::try_from(confirm.len()).expect("length fits u32"),
                0x10,
                b"firstsecond",
            ),
            ethernet_tcp(
                2,
                1,
                300 + u32::try_from(confirm.len() + 11).expect("length fits u32"),
                0x11,
                &[],
            ),
        ],
        None,
    );

    assert_eq!(
        flatten(&capture.flow.server_stream),
        [confirm.as_slice(), b"firstsecond"].concat()
    );
}

#[test]
// oracle: spec RFC 791 §3.1
fn ignores_padding_after_the_ipv4_packet() {
    let mut client = ethernet_tcp(1, 2, 101, 0x10, &request());
    client.extend([0; 16]);
    let capture = read(
        "ipv4-padding",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            client,
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(flatten(&capture.flow.client_stream), request());
}

#[test]
// oracle: mutant rejecting NIC-offload IPv4 captures with an unset total length
fn parses_zero_length_ipv4_using_captured_extent() {
    let mut client = ethernet_tcp(1, 2, 101, 0x10, &request());
    client[16..18].copy_from_slice(&[0; 2]);
    let capture = read(
        "zero-ipv4-length",
        [
            ethernet_tcp(1, 2, 100, 0x02, &[]),
            client,
            ethernet_tcp(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(flatten(&capture.flow.client_stream), request());
}

#[test]
// oracle: mutant treating ambiguous Ethernet padding as TCP data
fn rejects_ambiguous_zero_length_ipv4_ethernet_padding() {
    let request = request();
    let mut client = ethernet_tcp(1, 2, 100, 0x02, &request[..6]);
    client[16..18].copy_from_slice(&[0; 2]);

    assert!(matches!(
        read_error(
            "ambiguous-ipv4-padding",
            [
                client,
                ethernet_tcp(1, 2, 107, 0x10, &request[6..]),
                ethernet_tcp(2, 1, 200, 0x10, &confirm()),
            ],
        ),
        CaptureError::UnsupportedTransport
    ));
}

#[test]
// oracle: regression #2153 truncated zero-length IPv4 TCP header
fn rejects_zero_length_ipv4_without_a_complete_tcp_header() {
    let mut client = ethernet_tcp(1, 2, 100, 0x02, b"payload");
    client[16..18].copy_from_slice(&[0; 2]);
    client.truncate(
        14 /* Ethernet */
            + 20 /* IPv4 */
            + 19, /* truncated TCP */
    );

    assert!(matches!(
        read_error(
            "truncated-tcp-header",
            [
                client,
                ethernet_tcp(1, 2, 101, 0x10, &request()),
                ethernet_tcp(2, 1, 200, 0x10, &confirm()),
            ],
        ),
        CaptureError::UnsupportedTransport
    ));
}

#[test]
// oracle: spec RFC 8200 §4.1
fn parses_tcp_after_an_ipv6_hop_by_hop_header() {
    let capture = read(
        "ipv6-hop-by-hop",
        [
            ethernet_ipv6_tcp_with_hop_by_hop(1, 2, 100, 0x02, &[]),
            ethernet_ipv6_tcp_with_hop_by_hop(1, 2, 101, 0x10, &request()),
            ethernet_ipv6_tcp_with_hop_by_hop(2, 1, 200, 0x10, &confirm()),
        ],
        None,
    );

    assert_eq!(flatten(&capture.flow.client_stream), request());
}

fn ethernet_ipv6_tcp_with_hop_by_hop(
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    flags: u8,
    data: &[u8],
) -> Vec<u8> {
    let mut packet = vec![
        0;
        14 /* Ethernet */
            + 40 /* IPv6 */
            + 8 /* hop-by-hop options */
            + 20 /* TCP */
    ];
    packet[12..14].copy_from_slice(&0x86ddu16.to_be_bytes());
    packet[14] = 0x60;
    packet[18..20].copy_from_slice(
        &u16::try_from(8 + 20 + data.len())
            .expect("IPv6 payload length fits u16")
            .to_be_bytes(),
    );
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
    tcp[13] = flags;
    packet.extend(data);
    packet
}
