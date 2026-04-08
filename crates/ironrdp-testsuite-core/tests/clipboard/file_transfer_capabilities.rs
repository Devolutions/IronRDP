use ironrdp_cliprdr::pdu::{ClipboardGeneralCapabilityFlags, *};
use ironrdp_testsuite_core::encode_decode_test;

// [MS-RDPECLIP] 2.2.2.1 General Capability Set (CLIPRDR_GENERAL_CAPABILITY)
// Tests for file transfer capability flags negotiation

encode_decode_test! {
    // Test all file transfer capability flags together
    capabilities_all_file_transfer_flags:
        ClipboardPdu::Capabilities(
            Capabilities {
                capabilities: vec![
                    CapabilitySet::General(
                        GeneralCapabilitySet {
                            version: ClipboardProtocolVersion::V2,
                            general_flags: ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
                                | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
                                | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
                                | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED,
                        }
                    )
                ]
            }
        ),
        [
            // PartialHeader (8 bytes)
            0x07, 0x00, 0x00, 0x00,  // msgType: CB_CLIP_CAPS (7)
            0x10, 0x00, 0x00, 0x00,  // dataLen: 16
            // cCapabilitiesSets (2 bytes) + pad (2 bytes)
            0x01, 0x00,              // cCapabilitiesSets: 1
            0x00, 0x00,              // pad
            // CapabilitySet header
            0x01, 0x00,              // capabilitySetType: CB_CAPSTYPE_GENERAL (1)
            0x0c, 0x00,              // lengthCapability: 12
            // GeneralCapabilitySet
            0x02, 0x00, 0x00, 0x00,  // version: CB_CAPS_VERSION_2 (2)
            0x3e, 0x00, 0x00, 0x00,  // generalFlags: 0x3e
                                     // = USE_LONG_FORMAT_NAMES (0x02)
                                     // | STREAM_FILECLIP_ENABLED (0x04)
                                     // | FILECLIP_NO_FILE_PATHS (0x08)
                                     // | CAN_LOCK_CLIPDATA (0x10)
                                     // | HUGE_FILE_SUPPORT_ENABLED (0x20)
        ];

    // Test minimal file transfer capabilities (streaming only)
    capabilities_minimal_file_transfer:
        ClipboardPdu::Capabilities(
            Capabilities {
                capabilities: vec![
                    CapabilitySet::General(
                        GeneralCapabilitySet {
                            version: ClipboardProtocolVersion::V2,
                            general_flags: ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
                        }
                    )
                ]
            }
        ),
        [
            0x07, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x0c, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
        ];

    // Test file transfer with locking support
    capabilities_with_locking:
        ClipboardPdu::Capabilities(
            Capabilities {
                capabilities: vec![
                    CapabilitySet::General(
                        GeneralCapabilitySet {
                            version: ClipboardProtocolVersion::V2,
                            general_flags: ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                                | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA,
                        }
                    )
                ]
            }
        ),
        [
            0x07, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x0c, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00,  // 0x04 | 0x10 = 0x14
        ];

    // Test huge file support
    capabilities_huge_files:
        ClipboardPdu::Capabilities(
            Capabilities {
                capabilities: vec![
                    CapabilitySet::General(
                        GeneralCapabilitySet {
                            version: ClipboardProtocolVersion::V2,
                            general_flags: ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                                | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED,
                        }
                    )
                ]
            }
        ),
        [
            0x07, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x0c, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00,  // 0x04 | 0x20 = 0x24
        ];
}

#[test]
fn capability_flags_bitwise_operations() {
    use ClipboardGeneralCapabilityFlags as Flags;

    // Test individual flag values match spec
    assert_eq!(Flags::USE_LONG_FORMAT_NAMES.bits(), 0x0000_0002);
    assert_eq!(Flags::STREAM_FILECLIP_ENABLED.bits(), 0x0000_0004);
    assert_eq!(Flags::FILECLIP_NO_FILE_PATHS.bits(), 0x0000_0008);
    assert_eq!(Flags::CAN_LOCK_CLIPDATA.bits(), 0x0000_0010);
    assert_eq!(Flags::HUGE_FILE_SUPPORT_ENABLED.bits(), 0x0000_0020);

    // Test flag combinations
    let all_file_flags = Flags::STREAM_FILECLIP_ENABLED
        | Flags::FILECLIP_NO_FILE_PATHS
        | Flags::CAN_LOCK_CLIPDATA
        | Flags::HUGE_FILE_SUPPORT_ENABLED;

    assert_eq!(all_file_flags.bits(), 0x0000_003c); // 0x04 | 0x08 | 0x10 | 0x20

    // Test flag checking
    assert!(all_file_flags.contains(Flags::STREAM_FILECLIP_ENABLED));
    assert!(all_file_flags.contains(Flags::FILECLIP_NO_FILE_PATHS));
    assert!(all_file_flags.contains(Flags::CAN_LOCK_CLIPDATA));
    assert!(all_file_flags.contains(Flags::HUGE_FILE_SUPPORT_ENABLED));
    assert!(!all_file_flags.contains(Flags::USE_LONG_FORMAT_NAMES));
}

#[test]
fn capability_negotiation_downgrade() {
    use ClipboardGeneralCapabilityFlags as Flags;

    // Client supports all file transfer features
    let client_caps = Capabilities::new(
        ClipboardProtocolVersion::V2,
        Flags::USE_LONG_FORMAT_NAMES
            | Flags::STREAM_FILECLIP_ENABLED
            | Flags::FILECLIP_NO_FILE_PATHS
            | Flags::CAN_LOCK_CLIPDATA
            | Flags::HUGE_FILE_SUPPORT_ENABLED,
    );

    // Server only supports basic file streaming
    let server_caps = Capabilities::new(
        ClipboardProtocolVersion::V2,
        Flags::USE_LONG_FORMAT_NAMES | Flags::STREAM_FILECLIP_ENABLED,
    );

    let mut negotiated = client_caps;
    negotiated.downgrade(&server_caps);

    // After negotiation, only common flags should remain
    let negotiated_flags = negotiated.flags();
    assert!(negotiated_flags.contains(Flags::USE_LONG_FORMAT_NAMES));
    assert!(negotiated_flags.contains(Flags::STREAM_FILECLIP_ENABLED));
    assert!(!negotiated_flags.contains(Flags::FILECLIP_NO_FILE_PATHS));
    assert!(!negotiated_flags.contains(Flags::CAN_LOCK_CLIPDATA));
    assert!(!negotiated_flags.contains(Flags::HUGE_FILE_SUPPORT_ENABLED));
}

#[test]
fn capability_negotiation_no_file_transfer() {
    use ClipboardGeneralCapabilityFlags as Flags;

    // Client supports file transfer
    let client_caps = Capabilities::new(
        ClipboardProtocolVersion::V2,
        Flags::USE_LONG_FORMAT_NAMES | Flags::STREAM_FILECLIP_ENABLED,
    );

    // Server does not support file transfer (text only)
    let server_caps = Capabilities::new(ClipboardProtocolVersion::V2, Flags::USE_LONG_FORMAT_NAMES);

    let mut negotiated = client_caps;
    negotiated.downgrade(&server_caps);

    // After negotiation, file transfer should be disabled
    let negotiated_flags = negotiated.flags();
    assert!(negotiated_flags.contains(Flags::USE_LONG_FORMAT_NAMES));
    assert!(!negotiated_flags.contains(Flags::STREAM_FILECLIP_ENABLED));
}

#[test]
fn capability_version_downgrade() {
    use ClipboardGeneralCapabilityFlags as Flags;

    // Client uses V2
    let client_caps = Capabilities::new(ClipboardProtocolVersion::V2, Flags::USE_LONG_FORMAT_NAMES);

    // Server uses V1
    let server_caps = Capabilities::new(ClipboardProtocolVersion::V1, Flags::USE_LONG_FORMAT_NAMES);

    let mut negotiated = client_caps;
    negotiated.downgrade(&server_caps);

    // Version should downgrade to V1
    assert_eq!(negotiated.version(), ClipboardProtocolVersion::V1);
}

/// [MS-RDPECLIP] 2.2.2.1.1.1 - The version field is for informational purposes
/// and MUST NOT be used to make protocol capability decisions. Unknown version
/// values must not cause decode failures.
#[test]
fn capability_unknown_version_accepted() {
    use ironrdp_core::{Decode as _, Encode as _};

    // Build a Capabilities PDU with a hypothetical version 3
    let caps = Capabilities::new(
        ClipboardProtocolVersion::Unknown(3),
        ClipboardGeneralCapabilityFlags::empty(),
    );

    let mut buf = vec![0u8; caps.size() + 2 /* msgType */];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    let pdu = ClipboardPdu::Capabilities(caps);
    pdu.encode(&mut cursor).unwrap();

    // Decode should succeed (not reject the PDU)
    let mut read = ironrdp_core::ReadCursor::new(&buf);
    let decoded = ClipboardPdu::decode(&mut read).unwrap();

    match decoded {
        ClipboardPdu::Capabilities(decoded_caps) => {
            assert_eq!(decoded_caps.version(), ClipboardProtocolVersion::Unknown(3));
        }
        other => panic!("expected Capabilities PDU, got {other:?}"),
    }
}

/// Unknown versions should downgrade to V1 during negotiation,
/// since they don't match V2.
#[test]
fn capability_unknown_version_downgrades() {
    use ClipboardGeneralCapabilityFlags as Flags;

    let client_caps = Capabilities::new(ClipboardProtocolVersion::V2, Flags::USE_LONG_FORMAT_NAMES);
    let server_caps = Capabilities::new(ClipboardProtocolVersion::Unknown(3), Flags::USE_LONG_FORMAT_NAMES);

    let mut negotiated = client_caps;
    negotiated.downgrade(&server_caps);

    // Unknown version != V2, so downgrade picks V1
    assert_eq!(negotiated.version(), ClipboardProtocolVersion::V1);
}
