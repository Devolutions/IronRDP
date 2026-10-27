//! STUN Binding probes for opt-in UDP server-reflexive address discovery.
//!
//! This implements the unauthenticated Binding method from [RFC 8489] without external STUN crate dependencies so default builds remain lightweight.
//! It does not implement ICE connectivity checks, TURN, or candidate signaling.
//!
//! [RFC 8489]: https://www.rfc-editor.org/rfc/rfc8489

use core::fmt;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::time::Duration;
use std::io;

use tokio::net::UdpSocket;

const STUN_HEADER_LENGTH: usize = 20;
const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS_RESPONSE: u16 = 0x0101;
const MAPPED_ADDRESS: u16 = 0x0001;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;

fn xor_port_mask(cookie: u32) -> u16 {
    let bytes = cookie.to_be_bytes();
    u16::from_be_bytes([bytes[0], bytes[1]])
}

/// A failed STUN Binding probe.
#[derive(Debug)]
pub enum StunError {
    /// The UDP socket failed while sending or receiving the request.
    Io(io::Error),
    /// No matching STUN response arrived before the deadline.
    Timeout,
    /// The server response did not contain a valid mapped address.
    InvalidResponse(&'static str),
}

impl fmt::Display for StunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "STUN UDP socket error: {error}"),
            Self::Timeout => write!(f, "STUN Binding request timed out"),
            Self::InvalidResponse(reason) => write!(f, "invalid STUN Binding response: {reason}"),
        }
    }
}

impl core::error::Error for StunError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Timeout | Self::InvalidResponse(_) => None,
        }
    }
}

/// Sends one STUN Binding request and returns its server-reflexive address.
///
/// The caller resolves and selects the STUN server, binds `socket`, and supplies a fresh CSPRNG-generated 96-bit transaction ID.
/// Accepting `transaction_id` from the caller keeps the default feature set free of a mandatory RNG dependency and lets callers correlate or retry probes.
/// The socket remains available for [`crate::connect_udp_with_socket`], allowing an embedder to attempt RDPEUDP2 on the same local socket after candidate gathering.
///
/// The call waits only for a response from `stun_server` with the matching transaction ID.
/// Unrelated datagrams are ignored until `timeout` expires.
/// This sends one request without retransmission, so callers can retry with a new transaction ID after a timeout.
/// The caller must not concurrently read from `socket`.
///
/// The returned address is a STUN server-reflexive mapping, not proof that a remote peer can reach it.
/// NATs can map the same local socket differently for different remote endpoints.
/// When a response contains both `XOR-MAPPED-ADDRESS` and legacy `MAPPED-ADDRESS`, `XOR-MAPPED-ADDRESS` takes precedence ([RFC 8489] §§14.1–14.2).
///
/// [RFC 8489]: https://www.rfc-editor.org/rfc/rfc8489
///
/// # Errors
///
/// Returns [`StunError::Timeout`] if no matching response arrives before `timeout`, [`StunError::Io`] for a socket failure, or [`StunError::InvalidResponse`] for a malformed matching response.
pub async fn query_server_reflexive_address(
    socket: &UdpSocket,
    stun_server: SocketAddr,
    transaction_id: [u8; 12],
    timeout: Duration,
) -> Result<SocketAddr, StunError> {
    let mut request = [0; STUN_HEADER_LENGTH];
    request[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    request[2..4].copy_from_slice(&0u16.to_be_bytes());
    request[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    request[8..20].copy_from_slice(&transaction_id);

    tokio::time::timeout(timeout, async {
        socket.send_to(&request, stun_server).await.map_err(StunError::Io)?;

        let mut response = vec![0u8; 4096];
        loop {
            let (length, source) = socket.recv_from(&mut response).await.map_err(StunError::Io)?;
            if source != stun_server || length < STUN_HEADER_LENGTH {
                continue;
            }
            if response[0] & 0b1100_0000 != 0 || response[4..8] != STUN_MAGIC_COOKIE.to_be_bytes() {
                continue;
            }
            if response[8..STUN_HEADER_LENGTH] != transaction_id {
                continue;
            }

            return parse_binding_success_response(&response[..length], transaction_id);
        }
    })
    .await
    .map_err(|_| StunError::Timeout)?
}

fn parse_binding_success_response(bytes: &[u8], transaction_id: [u8; 12]) -> Result<SocketAddr, StunError> {
    if bytes.len() < STUN_HEADER_LENGTH {
        return Err(StunError::InvalidResponse("header is truncated"));
    }
    if bytes[0] & 0b1100_0000 != 0 {
        return Err(StunError::InvalidResponse("STUN header bits are invalid"));
    }
    if u16::from_be_bytes([bytes[0], bytes[1]]) != BINDING_SUCCESS_RESPONSE {
        return Err(StunError::InvalidResponse("message is not a Binding success response"));
    }

    let body_length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if body_length % 4 != 0 || bytes.len() != STUN_HEADER_LENGTH + body_length {
        return Err(StunError::InvalidResponse("message length is invalid"));
    }
    if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != STUN_MAGIC_COOKIE {
        return Err(StunError::InvalidResponse("magic cookie is invalid"));
    }
    if bytes[8..STUN_HEADER_LENGTH] != transaction_id {
        return Err(StunError::InvalidResponse("transaction ID does not match"));
    }

    let mut offset = STUN_HEADER_LENGTH;
    let mut mapped_address = None;
    let mut xor_mapped_address = None;
    while offset < bytes.len() {
        if bytes.len() - offset < 4 {
            return Err(StunError::InvalidResponse("attribute header is truncated"));
        }

        let attribute_type = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        let value_length = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
        let value_start = offset + 4;
        let value_end = value_start
            .checked_add(value_length)
            .filter(|end| *end <= bytes.len())
            .ok_or(StunError::InvalidResponse("attribute value is truncated"))?;
        let padded_length = (value_length + 3) & !3;
        offset = value_start
            .checked_add(padded_length)
            .filter(|end| *end <= bytes.len())
            .ok_or(StunError::InvalidResponse("attribute padding is truncated"))?;

        match attribute_type {
            MAPPED_ADDRESS => {
                if mapped_address.is_none() {
                    mapped_address = Some(decode_address(&bytes[value_start..value_end], None, transaction_id)?);
                }
            }
            XOR_MAPPED_ADDRESS => {
                if xor_mapped_address.is_none() {
                    xor_mapped_address = Some(decode_address(
                        &bytes[value_start..value_end],
                        Some(STUN_MAGIC_COOKIE),
                        transaction_id,
                    )?);
                }
            }
            0x0000..=0x7FFF => {
                return Err(StunError::InvalidResponse("unknown comprehension-required attribute"));
            }
            0x8000..=0xFFFF => {}
        }
    }

    xor_mapped_address
        .or(mapped_address)
        .ok_or(StunError::InvalidResponse("mapped address attribute is missing"))
}

fn decode_address(value: &[u8], xor_cookie: Option<u32>, transaction_id: [u8; 12]) -> Result<SocketAddr, StunError> {
    if value.len() < 4 {
        return Err(StunError::InvalidResponse("address attribute header is invalid"));
    }

    let port_mask = xor_cookie.map_or(0, xor_port_mask);
    let port = u16::from_be_bytes([value[2], value[3]]) ^ port_mask;
    let address = match value[1] {
        0x01 if value.len() == 8 => {
            let mut octets = [value[4], value[5], value[6], value[7]];
            if let Some(cookie) = xor_cookie {
                for (octet, mask) in octets.iter_mut().zip(cookie.to_be_bytes()) {
                    *octet ^= mask;
                }
            }
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(octets), port))
        }
        0x02 if value.len() == 20 => {
            let mut octets = [0; 16];
            octets.copy_from_slice(&value[4..20]);
            if let Some(cookie) = xor_cookie {
                for (index, octet) in octets.iter_mut().enumerate() {
                    let mask = if index < 4 {
                        cookie.to_be_bytes()[index]
                    } else {
                        transaction_id[index - 4]
                    };
                    *octet ^= mask;
                }
            }
            SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(octets), port, 0, 0))
        }
        0x01 | 0x02 => return Err(StunError::InvalidResponse("address attribute length is invalid")),
        _ => return Err(StunError::InvalidResponse("address family is unsupported")),
    };

    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TRANSACTION_ID: [u8; 12] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76];

    fn binding_response(attribute_type: u16, value: &[u8], transaction_id: [u8; 12]) -> Vec<u8> {
        let mut response = Vec::with_capacity(STUN_HEADER_LENGTH + 4 + value.len());
        let value_length = u16::try_from(value.len()).expect("test attribute length fits in the STUN field");
        let body_length =
            u16::try_from(4 + ((value.len() + 3) & !3)).expect("test message length fits in the STUN field");
        response.extend_from_slice(&BINDING_SUCCESS_RESPONSE.to_be_bytes());
        response.extend_from_slice(&body_length.to_be_bytes());
        response.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
        response.extend_from_slice(&transaction_id);
        response.extend_from_slice(&attribute_type.to_be_bytes());
        response.extend_from_slice(&value_length.to_be_bytes());
        response.extend_from_slice(value);
        response.resize(STUN_HEADER_LENGTH + 4 + ((value.len() + 3) & !3), 0);
        response
    }

    fn xor_mapped_v4(address: Ipv4Addr, port: u16) -> Vec<u8> {
        let mut value = vec![0, 1];
        let port_mask = xor_port_mask(STUN_MAGIC_COOKIE);
        value.extend_from_slice(&(port ^ port_mask).to_be_bytes());
        value.extend(
            address
                .octets()
                .into_iter()
                .zip(STUN_MAGIC_COOKIE.to_be_bytes())
                .map(|(octet, mask)| octet ^ mask),
        );
        value
    }

    fn xor_mapped_v6(address: Ipv6Addr, port: u16, transaction_id: [u8; 12]) -> Vec<u8> {
        let mut value = vec![0, 2];
        let port_mask = xor_port_mask(STUN_MAGIC_COOKIE);
        value.extend_from_slice(&(port ^ port_mask).to_be_bytes());
        for (index, octet) in address.octets().into_iter().enumerate() {
            let mask = if index < 4 {
                STUN_MAGIC_COOKIE.to_be_bytes()[index]
            } else {
                transaction_id[index - 4]
            };
            value.push(octet ^ mask);
        }
        value
    }

    // RFC 5769 Section 2.2 IPv4 Binding response test vector:
    // transaction ID b7 e7 a7 01 bc 34 d6 86 fa 87 df ae and
    // XOR-MAPPED-ADDRESS value 00 01 a1 47 e1 12 a6 43 decode to 192.0.2.1:32853.
    // oracle: spec RFC 5769 §2.2
    #[test]
    fn decodes_xor_mapped_ipv4_address() {
        const RFC5769_TX_ID: [u8; 12] = [0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae];
        const RFC5769_XOR_MAPPED_V4: [u8; 8] = [0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43];
        let response = binding_response(XOR_MAPPED_ADDRESS, &RFC5769_XOR_MAPPED_V4, RFC5769_TX_ID);

        assert_eq!(
            parse_binding_success_response(&response, RFC5769_TX_ID).expect("valid STUN response"),
            "192.0.2.1:32853".parse().expect("valid socket address")
        );
    }

    // oracle: spec RFC 8489 §14.1
    #[test]
    fn decodes_mapped_ipv4_address_when_xor_attribute_is_absent() {
        let response = binding_response(MAPPED_ADDRESS, &[0, 1, 0x13, 0x88, 192, 0, 2, 1], TRANSACTION_ID);

        assert_eq!(
            parse_binding_success_response(&response, TRANSACTION_ID).expect("valid STUN response"),
            "192.0.2.1:5000".parse().expect("valid socket address")
        );
    }

    // RFC 5769 Section 2.2 IPv6 Binding response test vector:
    // transaction ID b7 e7 a7 01 bc 34 d6 86 fa 87 df ae and
    // XOR-MAPPED-ADDRESS value 00 02 a1 47 01 13 a9 fa a5 d3 f1 79 bc 25 f4 b5 be d2 b9 d9
    // decode to [2001:db8:1234:5678:11:2233:4455:6677]:32853.
    // oracle: spec RFC 5769 §2.2
    #[test]
    fn decodes_xor_mapped_ipv6_address() {
        const RFC5769_TX_ID: [u8; 12] = [0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae];
        const RFC5769_XOR_MAPPED_V6: [u8; 20] = [
            0x00, 0x02, 0xa1, 0x47, 0x01, 0x13, 0xa9, 0xfa, 0xa5, 0xd3, 0xf1, 0x79, 0xbc, 0x25, 0xf4, 0xb5, 0xbe, 0xd2,
            0xb9, 0xd9,
        ];
        let response = binding_response(XOR_MAPPED_ADDRESS, &RFC5769_XOR_MAPPED_V6, RFC5769_TX_ID);

        assert_eq!(
            parse_binding_success_response(&response, RFC5769_TX_ID).expect("valid STUN response"),
            "[2001:db8:1234:5678:11:2233:4455:6677]:32853"
                .parse()
                .expect("valid socket address")
        );
    }

    // oracle: spec RFC 8489 §14.2
    #[test]
    fn prefers_xor_mapped_address_over_mapped_address() {
        let fallback = [0, 1, 0x13, 0x88, 192, 0, 2, 1];
        let preferred = xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152);
        let mut response = binding_response(MAPPED_ADDRESS, &fallback, TRANSACTION_ID);
        response.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
        let preferred_length = u16::try_from(preferred.len()).expect("test attribute length fits in the STUN field");
        response.extend_from_slice(&preferred_length.to_be_bytes());
        response.extend_from_slice(&preferred);
        response.resize((response.len() + 3) & !3, 0);
        let body_length =
            u16::try_from(response.len() - STUN_HEADER_LENGTH).expect("test message length fits in the STUN field");
        response[2..4].copy_from_slice(&body_length.to_be_bytes());

        assert_eq!(
            parse_binding_success_response(&response, TRANSACTION_ID).expect("valid STUN response"),
            "203.0.113.10:49152".parse().expect("valid socket address")
        );
    }

    // oracle: spec RFC 8489 §5
    #[test]
    fn rejects_truncated_attribute_value() {
        let valid = binding_response(
            XOR_MAPPED_ADDRESS,
            &xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152),
            TRANSACTION_ID,
        );
        assert!(
            parse_binding_success_response(&valid[..valid.len() - 1], TRANSACTION_ID).is_err(),
            "truncated STUN attribute value must be rejected"
        );
    }

    // oracle: spec RFC 8489 §5
    #[test]
    fn rejects_mismatched_transaction_id() {
        let valid = binding_response(
            XOR_MAPPED_ADDRESS,
            &xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152),
            TRANSACTION_ID,
        );
        assert!(
            parse_binding_success_response(&valid, [0; 12]).is_err(),
            "mismatched STUN transaction ID must be rejected"
        );
    }

    // RFC 8489 Section 14: "only the first occurrence needs to be processed by a receiver, and any duplicates MAY be ignored by a receiver."
    // oracle: spec RFC 8489 §14
    #[test]
    fn retains_first_address_and_ignores_duplicate_attributes() {
        let first = xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152);
        let second = xor_mapped_v4(Ipv4Addr::new(192, 0, 2, 99), 5000);
        let mut response = binding_response(XOR_MAPPED_ADDRESS, &first, TRANSACTION_ID);
        response.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
        let second_length = u16::try_from(second.len()).expect("test attribute length fits in the STUN field");
        response.extend_from_slice(&second_length.to_be_bytes());
        response.extend_from_slice(&second);
        let body_length =
            u16::try_from(response.len() - STUN_HEADER_LENGTH).expect("test message length fits in the STUN field");
        response[2..4].copy_from_slice(&body_length.to_be_bytes());

        assert_eq!(
            parse_binding_success_response(&response, TRANSACTION_ID).expect("valid STUN response"),
            "203.0.113.10:49152".parse().expect("valid socket address")
        );
    }

    // oracle: spec RFC 8489 §14
    #[test]
    fn ignores_unknown_comprehension_optional_attributes() {
        let mapped = xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152);
        let mut with_optional = binding_response(0x8022, b"test", TRANSACTION_ID);
        with_optional.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
        let mapped_length = u16::try_from(mapped.len()).expect("test attribute length fits in the STUN field");
        with_optional.extend_from_slice(&mapped_length.to_be_bytes());
        with_optional.extend_from_slice(&mapped);
        let optional_body_length = u16::try_from(with_optional.len() - STUN_HEADER_LENGTH)
            .expect("test message length fits in the STUN field");
        with_optional[2..4].copy_from_slice(&optional_body_length.to_be_bytes());

        assert_eq!(
            parse_binding_success_response(&with_optional, TRANSACTION_ID).expect("valid STUN response"),
            "203.0.113.10:49152".parse().expect("valid socket address")
        );
    }

    // RFC 8489 Section 6.3.3: "If the success response contains unknown comprehension-required attributes, the response is discarded and the transaction is considered to have failed."
    // oracle: spec RFC 8489 §6.3.3
    #[test]
    fn rejects_unknown_comprehension_required_attributes() {
        let mapped = xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152);
        let mapped_length = u16::try_from(mapped.len()).expect("test attribute length fits in the STUN field");
        let mut with_required = binding_response(0x7FFF, b"test", TRANSACTION_ID);
        with_required.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
        with_required.extend_from_slice(&mapped_length.to_be_bytes());
        with_required.extend_from_slice(&mapped);
        let required_body_length = u16::try_from(with_required.len() - STUN_HEADER_LENGTH)
            .expect("test message length fits in the STUN field");
        with_required[2..4].copy_from_slice(&required_body_length.to_be_bytes());

        assert!(
            parse_binding_success_response(&with_required, TRANSACTION_ID).is_err(),
            "unknown comprehension-required STUN attribute must be rejected"
        );
    }

    // oracle: spec RFC 8489 §14.1
    #[test]
    fn ignores_nonzero_reserved_byte_in_address_attribute() {
        let mut value = xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152);
        value[0] = 0xFF;
        let response = binding_response(XOR_MAPPED_ADDRESS, &value, TRANSACTION_ID);

        assert_eq!(
            parse_binding_success_response(&response, TRANSACTION_ID).expect("valid STUN response"),
            "203.0.113.10:49152".parse().expect("valid socket address")
        );
    }

    // oracle: reference RFC 8489 §14.2 XOR-MAPPED-ADDRESS IPv4 encoder
    proptest! {
        #[test]
        fn roundtrips_xor_mapped_ipv4_socket_addresses(
            octets in any::<[u8; 4]>(),
            port in any::<u16>(),
            transaction_id in any::<[u8; 12]>(),
        ) {
            let address = Ipv4Addr::from(octets);
            let value = xor_mapped_v4(address, port);
            let response = binding_response(XOR_MAPPED_ADDRESS, &value, transaction_id);
            let decoded = parse_binding_success_response(&response, transaction_id)
                .expect("valid IPv4 XOR-MAPPED-ADDRESS response");
            prop_assert_eq!(decoded, SocketAddr::new(address.into(), port));
        }
    }

    // oracle: reference RFC 8489 §14.2 XOR-MAPPED-ADDRESS IPv6 encoder
    proptest! {
        #[test]
        fn roundtrips_xor_mapped_ipv6_socket_addresses(
            octets in any::<[u8; 16]>(),
            port in any::<u16>(),
            transaction_id in any::<[u8; 12]>(),
        ) {
            let address = Ipv6Addr::from(octets);
            let value = xor_mapped_v6(address, port, transaction_id);
            let response = binding_response(XOR_MAPPED_ADDRESS, &value, transaction_id);
            let decoded = parse_binding_success_response(&response, transaction_id)
                .expect("valid IPv6 XOR-MAPPED-ADDRESS response");
            prop_assert_eq!(decoded, SocketAddr::new(address.into(), port));
        }
    }

    // oracle: spec RFC 8489 §5
    proptest! {
        #[test]
        fn rejects_arbitrary_malformed_packets_without_panicking(
            bytes in prop::collection::vec(any::<u8>(), 0..=256),
            transaction_id in any::<[u8; 12]>(),
        ) {
            if let Ok(_address) = parse_binding_success_response(&bytes, transaction_id) {
                prop_assert!(STUN_HEADER_LENGTH <= bytes.len());
                prop_assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), BINDING_SUCCESS_RESPONSE);
                prop_assert_eq!(u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]), STUN_MAGIC_COOKIE);
                prop_assert_eq!(&bytes[8..STUN_HEADER_LENGTH], &transaction_id);
            }
        }
    }

    // oracle: spec RFC 8489 §6.3
    #[tokio::test]
    async fn binding_probe_uses_the_supplied_socket_and_transaction_id() {
        let server = UdpSocket::bind("127.0.0.1:0").await.expect("bind STUN server");
        let server_addr = server.local_addr().expect("STUN server address");
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("bind client socket");
        let response = binding_response(
            XOR_MAPPED_ADDRESS,
            &xor_mapped_v4(Ipv4Addr::new(203, 0, 113, 10), 49_152),
            TRANSACTION_ID,
        );
        let unrelated_response = binding_response(
            XOR_MAPPED_ADDRESS,
            &xor_mapped_v4(Ipv4Addr::new(192, 0, 2, 1), 5000),
            [0; 12],
        );

        let server_task = tokio::spawn(async move {
            let mut request = [0; STUN_HEADER_LENGTH];
            let (length, peer) = server.recv_from(&mut request).await.expect("receive STUN request");
            assert_eq!(length, STUN_HEADER_LENGTH);
            assert_eq!(&request[..2], &BINDING_REQUEST.to_be_bytes());
            assert_eq!(&request[8..], &TRANSACTION_ID);
            server
                .send_to(&unrelated_response, peer)
                .await
                .expect("send unrelated STUN response");
            server.send_to(&response, peer).await.expect("send STUN response");
        });

        let mapped = query_server_reflexive_address(&client, server_addr, TRANSACTION_ID, Duration::from_secs(1))
            .await
            .expect("STUN probe succeeds");

        assert_eq!(mapped, "203.0.113.10:49152".parse().expect("valid socket address"));
        assert!(client.local_addr().expect("client local address").port() != 0);
        server_task.await.expect("STUN server task");
    }
}
