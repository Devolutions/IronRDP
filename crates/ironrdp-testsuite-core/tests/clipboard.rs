use expect_test::expect;
use ironrdp_cliprdr::pdu::{
    Capabilities, CapabilitySet, ClipboardFormat, ClipboardGeneralCapabilityFlags, ClipboardPdu,
    ClipboardProtocolVersion, FileContentsFlags, FileContentsRequest, FileContentsResponse, FormatDataRequest,
    FormatDataResponse, FormatList, FormatListResponse, GeneralCapabilitySet, LockDataId, PackedMetafileMappingMode,
};
use ironrdp_pdu::PduEncode;
use ironrdp_testsuite_core::encode_decode_test;

// Test blobs from [MS-RDPECLIP]
encode_decode_test! {
    capabilities:
        ClipboardPdu::Capabilites(
            Capabilities {
                capabilities: vec![
                    CapabilitySet::General(
                        GeneralCapabilitySet {
                            version: ClipboardProtocolVersion::V2,
                            general_flags: ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
                                | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
                        }
                    )
                ]
            }
        ),
        [
            0x07, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x0c, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00,
        ];

    monitor_ready:
        ClipboardPdu::MonitorReady,
        [
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

    format_list_response:
        ClipboardPdu::FormatListResponse(FormatListResponse::Ok),
        [
            0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

    lock:
        ClipboardPdu::LockData(LockDataId(8)),
        [
            0x0a, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
            0x08, 0x00, 0x00, 0x00,
        ];

    unlock:
        ClipboardPdu::UnlockData(LockDataId(8)),
        [
            0x0b, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
            0x08, 0x00, 0x00, 0x00,
        ];

    format_data_request:
        ClipboardPdu::FormatDataRequest(FormatDataRequest {
            format_id: 0x0d,
        }),
        [
            0x04, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
            0x0d, 0x00, 0x00, 0x00,
        ];

    format_data_response:
        ClipboardPdu::FormatDataResponse(
            FormatDataResponse::new_data(b"h\0e\0l\0l\0o\0 \0w\0o\0r\0l\0d\0\0\0".as_slice()),
        ),
        [
            0x05, 0x00, 0x01, 0x00, 0x18, 0x00, 0x00, 0x00,
            0x68, 0x00, 0x65, 0x00, 0x6c, 0x00, 0x6c, 0x00,
            0x6f, 0x00, 0x20, 0x00, 0x77, 0x00, 0x6f, 0x00,
            0x72, 0x00, 0x6c, 0x00, 0x64, 0x00, 0x00, 0x00,
        ];

    file_contents_request_size:
        ClipboardPdu::FileContentsRequest(FileContentsRequest {
            stream_id: 2,
            index: 1,
            flags: FileContentsFlags::SIZE,
            position: 0,
            requested_size: 8,
            data_id: None,
        }),
        [
            0x08, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
        ];

    file_contents_request_data:
        ClipboardPdu::FileContentsRequest(FileContentsRequest {
            stream_id: 2,
            index: 1,
            flags: FileContentsFlags::DATA,
            position: 0,
            requested_size: 65536,
            data_id: None,
        }),
        [
            0x08, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
        ];

    file_contents_response_size:
        ClipboardPdu::FileContentsResponse(FileContentsResponse::new_size_response(2, 44)),
        [
            0x09, 0x00, 0x01, 0x00, 0x0c, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

    file_contents_response_data:
        ClipboardPdu::FileContentsResponse(FileContentsResponse::new_data_response(
            2,
            b"The quick brown fox jumps over the lazy dog.".as_slice()
        )),
        [
            0x09, 0x00, 0x01, 0x00, 0x30, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x54, 0x68, 0x65, 0x20,
            0x71, 0x75, 0x69, 0x63, 0x6b, 0x20, 0x62, 0x72,
            0x6f, 0x77, 0x6e, 0x20, 0x66, 0x6f, 0x78, 0x20,
            0x6a, 0x75, 0x6d, 0x70, 0x73, 0x20, 0x6f, 0x76,
            0x65, 0x72, 0x20, 0x74, 0x68, 0x65, 0x20, 0x6c,
            0x61, 0x7a, 0x79, 0x20, 0x64, 0x6f, 0x67, 0x2e,
        ];
}

#[test]
fn client_temp_dir_encode_decode_ms_1() {
    // Test blob from [MS-RDPECLIP]
    let input = include_bytes!("../test_data/pdu/clipboard/client_temp_dir.pdu");

    let decoded_pdu: ClipboardPdu = ironrdp_pdu::decode(input).unwrap();

    if let ClipboardPdu::TemporaryDirectory(client_temp_dir) = &decoded_pdu {
        let path = client_temp_dir.temporary_directory_path().unwrap();
        expect![[r#"C:\DOCUME~1\ELTONS~1.NTD\LOCALS~1\Temp\cdepotslhrdp_1\_TSABD.tmp"#]].assert_eq(&path);
    } else {
        panic!("Expected ClientTemporaryDirectory");
    }

    let mut encoded = Vec::with_capacity(decoded_pdu.size());
    let _ = ironrdp_pdu::encode_buf(&decoded_pdu, &mut encoded).unwrap();

    assert_eq!(&encoded, input);
}

#[test]
fn format_list_ms_1() {
    // Test blob from [MS-RDPECLIP]
    let input = include_bytes!("../test_data/pdu/clipboard/format_list.pdu");

    let decoded_pdu: ClipboardPdu = ironrdp_pdu::decode(input).unwrap();

    if let ClipboardPdu::FormatList(format_list) = &decoded_pdu {
        let formats = format_list.get_formats(true).unwrap();

        expect![[r#"
            [
                ClipboardFormat {
                    id: 49156,
                    name: "Native",
                },
                ClipboardFormat {
                    id: 3,
                    name: "",
                },
                ClipboardFormat {
                    id: 8,
                    name: "",
                },
                ClipboardFormat {
                    id: 17,
                    name: "",
                },
            ]
        "#]]
        .assert_debug_eq(&formats);

        formats
    } else {
        panic!("Expected FormatList");
    };

    let mut encoded = Vec::with_capacity(decoded_pdu.size());
    let _ = ironrdp_pdu::encode_buf(&decoded_pdu, &mut encoded).unwrap();

    assert_eq!(&encoded, input);
}

#[test]
fn format_list_ms_2() {
    // Test blob from [MS-RDPECLIP]
    let input = include_bytes!("../test_data/pdu/clipboard/format_list_2.pdu");

    let decoded_pdu: ClipboardPdu = ironrdp_pdu::decode(input).unwrap();

    if let ClipboardPdu::FormatList(format_list) = &decoded_pdu {
        let formats = format_list.get_formats(true).unwrap();

        expect![[r#"
            [
                ClipboardFormat {
                    id: 49290,
                    name: "Rich Text Format",
                },
                ClipboardFormat {
                    id: 49477,
                    name: "Rich Text Format Without Objects",
                },
                ClipboardFormat {
                    id: 49475,
                    name: "RTF As Text",
                },
                ClipboardFormat {
                    id: 1,
                    name: "",
                },
                ClipboardFormat {
                    id: 13,
                    name: "",
                },
                ClipboardFormat {
                    id: 49156,
                    name: "Native",
                },
                ClipboardFormat {
                    id: 49166,
                    name: "Object Descriptor",
                },
                ClipboardFormat {
                    id: 3,
                    name: "",
                },
                ClipboardFormat {
                    id: 16,
                    name: "",
                },
                ClipboardFormat {
                    id: 7,
                    name: "",
                },
            ]
        "#]]
        .assert_debug_eq(&formats);

        formats
    } else {
        panic!("Expected FormatList");
    };

    let mut encoded = Vec::with_capacity(decoded_pdu.size());
    let _ = ironrdp_pdu::encode_buf(&decoded_pdu, &mut encoded).unwrap();

    assert_eq!(&encoded, input);
}

fn fake_format_list(use_ascii: bool, use_long_format: bool) -> Box<FormatList<'static>> {
    let formats = vec![
        ClipboardFormat {
            id: 42,
            name: "Hello".to_string(),
        },
        ClipboardFormat {
            id: 24,
            name: "".to_string(),
        },
        ClipboardFormat {
            id: 11,
            name: "World".to_string(),
        },
    ];

    let list = if use_ascii {
        FormatList::new_ascii(&formats, use_long_format).unwrap()
    } else {
        FormatList::new_unicode(&formats, use_long_format).unwrap()
    };

    Box::new(list)
}

#[test]
fn format_list_all_encodings() {
    // ASCII, short format names
    fake_format_list(true, false);
    // ASCII, long format names
    fake_format_list(true, true);
    // Unicode, short format names
    fake_format_list(false, false);
    // Unicode, long format names
    fake_format_list(false, true);
}

#[test]
fn metafile_pdu_ms() {
    // Test blob from [MS-RDPECLIP]
    let input = include_bytes!("../test_data/pdu/clipboard/metafile.pdu");

    let decoded_pdu: ClipboardPdu = ironrdp_pdu::decode(input).unwrap();

    if let ClipboardPdu::FormatDataResponse(response) = &decoded_pdu {
        let metafile = response.to_metafile().unwrap();

        assert_eq!(metafile.mapping_mode, PackedMetafileMappingMode::ANISOTROPIC);
        assert_eq!(metafile.x_ext, 556);
        assert_eq!(metafile.y_ext, 423);

        // Just check some known arbitrary byte in raw metafile data
        assert_eq!(metafile.data()[metafile.data().len() - 6], 0x03);
    } else {
        panic!("Expected FormatDataResponse");
    };

    let mut encoded = Vec::with_capacity(decoded_pdu.size());
    let _ = ironrdp_pdu::encode_buf(&decoded_pdu, &mut encoded).unwrap();

    assert_eq!(&encoded, input);
}

#[test]
fn palette_pdu_ms() {
    // Test blob from [MS-RDPECLIP]
    let input = include_bytes!("../test_data/pdu/clipboard/palette.pdu");

    let decoded_pdu: ClipboardPdu = ironrdp_pdu::decode(input).unwrap();

    if let ClipboardPdu::FormatDataResponse(response) = &decoded_pdu {
        let palette = response.to_palette().unwrap();

        assert_eq!(palette.entries.len(), 216);

        // Chack known palette color
        assert_eq!(palette.entries[53].red, 0xff);
        assert_eq!(palette.entries[53].green, 0x66);
        assert_eq!(palette.entries[53].blue, 0x33);
        assert_eq!(palette.entries[53].extra, 0x00);
    } else {
        panic!("Expected FormatDataResponse");
    };

    let mut encoded = Vec::with_capacity(decoded_pdu.size());
    let _ = ironrdp_pdu::encode_buf(&decoded_pdu, &mut encoded).unwrap();

    assert_eq!(&encoded, input);
}

#[test]
fn file_list_pdu_ms() {
    // Test blob from [MS-RDPECLIP]
    let input = include_bytes!("../test_data/pdu/clipboard/file_list.pdu");

    let decoded_pdu: ClipboardPdu = ironrdp_pdu::decode(input).unwrap();

    if let ClipboardPdu::FormatDataResponse(response) = &decoded_pdu {
        let file_list = response.to_file_list().unwrap();

        expect![[r#"
            [
                FileDescriptor {
                    attibutes: Some(
                        ClipboardFileAttributes(
                            ARCHIVE,
                        ),
                    ),
                    last_write_time: Some(
                        129010042240261384,
                    ),
                    file_size: Some(
                        44,
                    ),
                    name: "File1.txt",
                },
                FileDescriptor {
                    attibutes: Some(
                        ClipboardFileAttributes(
                            ARCHIVE,
                        ),
                    ),
                    last_write_time: Some(
                        129010042240261384,
                    ),
                    file_size: Some(
                        10,
                    ),
                    name: "File2.txt",
                },
            ]
        "#]]
        .assert_debug_eq(&file_list.files)
    } else {
        panic!("Expected FormatDataResponse");
    };

    let mut encoded = Vec::with_capacity(decoded_pdu.size());
    let _ = ironrdp_pdu::encode_buf(&decoded_pdu, &mut encoded).unwrap();

    assert_eq!(&encoded, input);
}
