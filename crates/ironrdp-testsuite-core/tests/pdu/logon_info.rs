use ironrdp_core::{EncodeErrorKind, decode, encode_vec};
use ironrdp_pdu::rdp::session_info::{LogonInfo, LogonInfoVersion1, LogonInfoVersion2};
use proptest::prelude::*;
use rstest::rstest;

const VERSION_1_DOMAIN_SIZE_OFFSET: usize = 0;
const VERSION_1_USER_NAME_SIZE_OFFSET: usize = 4 /* cbDomain */ + 52 /* Domain */;
const VERSION_2_DOMAIN_SIZE_OFFSET: usize = 2 /* Version */ + 4 /* Size */ + 4 /* SessionId */;
const VERSION_2_USER_NAME_SIZE_OFFSET: usize = VERSION_2_DOMAIN_SIZE_OFFSET + 4 /* cbDomain */;
const VERSION_2_FIXED_SIZE: usize = VERSION_2_USER_NAME_SIZE_OFFSET + 4 /* cbUserName */ + 558 /* Pad */;

fn logon_info(domain_name: &str, user_name: &str) -> LogonInfo {
    LogonInfo {
        session_id: 2,
        user_name: user_name.to_owned(),
        domain_name: domain_name.to_owned(),
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Names that fit their fields even when every character takes two UTF-16 units.
fn any_logon_info() -> impl Strategy<Value = LogonInfo> {
    (any::<u32>(), "\\PC{0,127}", "\\PC{0,12}").prop_map(|(session_id, user_name, domain_name)| LogonInfo {
        session_id,
        user_name,
        domain_name,
    })
}

#[rstest]
#[case::empty("", 2)]
#[case::ascii("NTDEV", 12)]
#[case::two_byte_utf8("M\u{fc}ller", 14)]
#[case::three_byte_utf8("\u{7528}\u{6237}", 6)]
#[case::surrogate_pair("\u{1f600}", 6)]
fn name_sizes_count_utf16_bytes_and_the_null_terminator(#[case] name: &str, #[case] expected_size: u32) {
    let logon_info = logon_info(name, name);

    // oracle: spec MS-RDPBCGR §2.2.10.1.1.1
    let version_1 = encode_vec(&LogonInfoVersion1 {
        logon_info: logon_info.clone(),
    })
    .unwrap();
    assert_eq!(read_u32(&version_1, VERSION_1_DOMAIN_SIZE_OFFSET), expected_size);
    assert_eq!(read_u32(&version_1, VERSION_1_USER_NAME_SIZE_OFFSET), expected_size);

    // oracle: spec MS-RDPBCGR §2.2.10.1.1.2
    let version_2 = encode_vec(&LogonInfoVersion2 { logon_info }).unwrap();
    assert_eq!(read_u32(&version_2, VERSION_2_DOMAIN_SIZE_OFFSET), expected_size);
    assert_eq!(read_u32(&version_2, VERSION_2_USER_NAME_SIZE_OFFSET), expected_size);
}

// oracle: spec MS-RDPBCGR §2.2.10.1.1.2
#[test]
fn version_2_ends_with_its_names_in_utf16() {
    let encoded = encode_vec(&LogonInfoVersion2 {
        logon_info: logon_info("\u{e9}", "\u{7528}"),
    })
    .unwrap();

    assert_eq!(
        encoded[VERSION_2_FIXED_SIZE..],
        [
            0xe9, 0x00, 0x00, 0x00, // Domain
            0x28, 0x75, 0x00, 0x00, // UserName
        ]
    );
}

// 25 and 255 characters leave room for the null terminator, and each of these takes three bytes in UTF-8.
#[test]
fn names_that_fill_their_fields_round_trip() {
    let logon_info = logon_info(&"\u{7528}".repeat(25), &"\u{7528}".repeat(255));

    // oracle: spec MS-RDPBCGR §2.2.10.1.1.1
    let version_1 = LogonInfoVersion1 {
        logon_info: logon_info.clone(),
    };
    assert_eq!(
        decode::<LogonInfoVersion1>(&encode_vec(&version_1).unwrap()).unwrap(),
        version_1
    );

    // oracle: reference FreeRDP/FreeRDP@d27a4f7c1c63b62a5e60e5d939ad116cfb58ffc1:libfreerdp/core/info.c
    let version_2 = LogonInfoVersion2 { logon_info };
    assert_eq!(
        decode::<LogonInfoVersion2>(&encode_vec(&version_2).unwrap()).unwrap(),
        version_2
    );
}

#[rstest]
#[case::domain_name("A".repeat(26), String::new())]
#[case::user_name(String::new(), "A".repeat(256))]
#[case::domain_name_of_surrogate_pairs("\u{1f600}".repeat(13), String::new())]
#[case::user_name_of_surrogate_pairs(String::new(), "\u{1f600}".repeat(128))]
fn names_longer_than_their_fields_are_not_encoded(#[case] domain_name: String, #[case] user_name: String) {
    let logon_info = logon_info(&domain_name, &user_name);

    // oracle: spec MS-RDPBCGR §2.2.10.1.1.1
    let version_1 = encode_vec(&LogonInfoVersion1 {
        logon_info: logon_info.clone(),
    });
    assert!(matches!(
        version_1.unwrap_err().kind(),
        EncodeErrorKind::InvalidField { .. }
    ));

    // oracle: reference FreeRDP/FreeRDP@d27a4f7c1c63b62a5e60e5d939ad116cfb58ffc1:libfreerdp/core/info.c
    let version_2 = encode_vec(&LogonInfoVersion2 { logon_info });
    assert!(matches!(
        version_2.unwrap_err().kind(),
        EncodeErrorKind::InvalidField { .. }
    ));
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn version_1_round_trips(logon_info in any_logon_info()) {
        let pdu = LogonInfoVersion1 { logon_info };

        prop_assert_eq!(decode::<LogonInfoVersion1>(&encode_vec(&pdu).unwrap()).unwrap(), pdu);
    }

    // oracle: property round-trip
    #[test]
    fn version_2_round_trips(logon_info in any_logon_info()) {
        let pdu = LogonInfoVersion2 { logon_info };

        prop_assert_eq!(decode::<LogonInfoVersion2>(&encode_vec(&pdu).unwrap()).unwrap(), pdu);
    }
}
