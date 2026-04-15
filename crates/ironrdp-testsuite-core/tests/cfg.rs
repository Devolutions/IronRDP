use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ironrdp_cfg::{ParseTargetAddrError, TargetAddr, TargetHost};
use rstest::rstest;

// -- TargetAddr::from_str -------------------------------------------------------

#[rstest]
// hostname
#[case("rdp.example.com", TargetHost::Domain("rdp.example.com".to_owned()), None)]
#[case("rdp.example.com:3389", TargetHost::Domain("rdp.example.com".to_owned()), Some(3389))]
// IPv4
#[case("192.168.1.1", TargetHost::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))), None)]
#[case(
    "192.168.1.1:3389",
    TargetHost::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
    Some(3389)
)]
// IPv6 (always bracketed in .rdp format)
#[case("[::1]", TargetHost::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)), None)]
#[case("[::1]:3389", TargetHost::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)), Some(3389))]
#[case("[2001:db8::1]:443", TargetHost::Ip(IpAddr::V6("2001:db8::1".parse().unwrap())), Some(443))]
// Unbracketed IPv6 — no port, must not misparse trailing segment as port
#[case("::1", TargetHost::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)), None)]
#[case("fe80::1", TargetHost::Ip(IpAddr::V6("fe80::1".parse().unwrap())), None)]
fn parse_valid(#[case] input: &str, #[case] expected_host: TargetHost, #[case] expected_port: Option<u16>) {
    let addr: TargetAddr = input.parse().unwrap();
    assert_eq!(addr.host, expected_host);
    assert_eq!(addr.port, expected_port);
}

#[rstest]
#[case("[::1", ParseTargetAddrError::UnclosedBracket)]
#[case("[not-ipv6]", ParseTargetAddrError::InvalidIpv6Addr)]
#[case("[127.0.0.1]", ParseTargetAddrError::InvalidIpv6Addr)]
#[case("[::1]:99999", ParseTargetAddrError::InvalidPort)]
#[case("[::1]garbage", ParseTargetAddrError::UnexpectedTrailing)]
#[case("rdp.example.com:99999", ParseTargetAddrError::InvalidPort)]
fn parse_invalid(#[case] input: &str, #[case] expected: ParseTargetAddrError) {
    assert_eq!(input.parse::<TargetAddr>().unwrap_err(), expected);
}

// -- TargetAddr::fmt ------------------------------------------------------------

/// IPv6 hosts must be re-bracketed on display; other hosts are written as-is.
#[rstest]
#[case("[::1]:3389", "[::1]:3389")]
#[case("[::1]", "[::1]")]
#[case("192.168.1.1:3389", "192.168.1.1:3389")]
#[case("192.168.1.1", "192.168.1.1")]
#[case("rdp.example.com", "rdp.example.com")]
fn display_roundtrip(#[case] input: &str, #[case] expected: &str) {
    let addr: TargetAddr = input.parse().unwrap();
    assert_eq!(addr.to_string(), expected);
}
