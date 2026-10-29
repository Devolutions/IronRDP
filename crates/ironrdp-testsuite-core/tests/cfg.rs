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

// -- DriveRedirection -------------------------------------------------------------

#[rstest]
#[case("", false, false, "")]
#[case("*", true, true, "")]
#[case("DynamicDrives", false, true, "")]
#[case("dynamicdrives;", false, true, "")]
#[case(r"C:\;E:\;", false, false, "CE")]
#[case("e:;c;D:/", false, false, "CDE")]
#[case(r" C:\ ; DynamicDrives ", false, true, "C")]
// Unrecognized entries (FreeRDP paths, devices, labels) are ignored.
#[case(r"C:\Users;%;DynamicDevices;CD:\;1:\", false, false, "")]
fn drive_redirection_parse(#[case] input: &str, #[case] all: bool, #[case] dynamic: bool, #[case] letters: &str) {
    let drives = ironrdp_cfg::DriveRedirection::parse(input);
    assert_eq!(drives.redirects_all(), all);
    assert_eq!(drives.redirects_dynamic(), dynamic);
    assert_eq!(drives.letters().collect::<String>(), letters);
}

#[rstest]
#[case("", &[])]
#[case(r"*;DynamicDrives;c;D:\;e:/", &[])]
#[case(r" C:\Users ; % ;DynamicDevices;CD:\;", &[r"C:\Users", "%", "DynamicDevices", r"CD:\"])]
fn drive_redirection_unrecognized_entries(#[case] input: &str, #[case] expected: &[&str]) {
    assert_eq!(
        ironrdp_cfg::DriveRedirection::unrecognized_entries(input).collect::<Vec<_>>(),
        expected
    );
}

#[rstest]
#[case("", "")]
#[case("*", "*")]
#[case(r"C:\;*", "*")]
#[case("e:;c", r"C:\;E:\")]
#[case("DynamicDrives;D", r"D:\;DynamicDrives")]
fn drive_redirection_display(#[case] input: &str, #[case] expected: &str) {
    let drives = ironrdp_cfg::DriveRedirection::parse(input);
    assert_eq!(drives.to_string(), expected);
    assert_eq!(ironrdp_cfg::DriveRedirection::parse(&drives.to_string()), drives);
}

#[test]
fn drive_redirection_dynamic_drives() {
    assert!(
        !ironrdp_cfg::DriveRedirection::parse("D")
            .with_dynamic(false)
            .redirects_dynamic()
    );
    assert!(
        ironrdp_cfg::DriveRedirection::parse("D")
            .with_dynamic(true)
            .redirects_dynamic()
    );
    assert!(
        ironrdp_cfg::DriveRedirection::all()
            .with_dynamic(false)
            .redirects_dynamic()
    );
}

// -- PropertySetExt::drive_redirection ----------------------------------------------

#[rstest]
#[case(None, None, None)]
#[case(Some(""), None, Some(""))]
#[case(None, Some(false), Some(""))]
#[case(None, Some(true), Some("*"))]
#[case(Some(""), Some(true), Some("*"))]
#[case(Some("  "), Some(true), Some("*"))]
#[case(Some(r"C:\"), Some(true), Some(r"C:\"))]
// An explicit opt-out wins over a stale drive list.
#[case(Some(r"C:\"), Some(false), Some(""))]
#[case(Some("*"), Some(false), Some(""))]
fn property_set_drive_redirection(
    #[case] drives_to_redirect: Option<&str>,
    #[case] redirect_drives: Option<bool>,
    #[case] expected: Option<&str>,
) {
    use ironrdp_cfg::PropertySetExt as _;

    let mut properties = ironrdp_propertyset::PropertySet::new();
    if let Some(value) = drives_to_redirect {
        properties.set_drives_to_redirect(value);
    }
    if let Some(value) = redirect_drives {
        properties.set_redirect_drives(value);
    }

    assert_eq!(
        properties.drive_redirection(),
        expected.map(ironrdp_cfg::DriveRedirection::parse)
    );
}

#[test]
fn property_set_drive_redirection_round_trips_through_rdp_file() {
    use ironrdp_cfg::PropertySetExt as _;

    let mut properties = ironrdp_propertyset::PropertySet::new();
    ironrdp_rdpfile::load(
        &mut properties,
        concat!(r"drivestoredirect:s:C:\;DynamicDrives", "\r\nredirectdrives:i:1\r\n"),
    )
    .expect("valid .rdp file");
    let drives = properties.drive_redirection().expect("drive selection is present");
    assert_eq!(drives.letters().collect::<String>(), "C");
    assert!(drives.redirects_dynamic());

    let mut written = ironrdp_propertyset::PropertySet::new();
    written.set_drive_redirection(drives);
    let content = ironrdp_rdpfile::write(&written);
    assert!(content.contains(r"drivestoredirect:s:C:\;DynamicDrives"), "{content}");
    assert!(content.contains("redirectdrives:i:1"), "{content}");
}
