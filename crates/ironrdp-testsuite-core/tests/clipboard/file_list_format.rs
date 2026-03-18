use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardPdu, FileDescriptor,
    FormatDataRequest, FormatDataResponse, FormatList, MAX_FILE_COUNT, PackedFileList,
};

// [MS-RDPECLIP] 2.2.5.2 - File List Format Tests
// Note: FileDescriptor encode/decode is already tested in file_list_pdu_ms test with real MS test data.
// These tests focus on higher-level integration and round-trip testing.

#[test]
fn format_list_with_file_group_descriptor() {
    // Test that FileGroupDescriptorW format can be properly encoded in a FormatList
    let formats = vec![ClipboardFormat::new(ClipboardFormatId::new(0xC0BC)).with_name(ClipboardFormatName::FILE_LIST)];

    let format_list = FormatList::new_unicode(&formats, true).unwrap();
    let encoded = ironrdp_core::encode_vec(&ClipboardPdu::FormatList(format_list)).unwrap();

    // Decode and verify
    let decoded: ClipboardPdu<'_> = ironrdp_core::decode(&encoded).unwrap();
    if let ClipboardPdu::FormatList(list) = decoded {
        let decoded_formats = list.get_formats(true).unwrap();
        assert_eq!(decoded_formats.len(), 1);
        assert_eq!(decoded_formats[0].id, ClipboardFormatId::new(0xC0BC));
        assert_eq!(
            decoded_formats[0].name.as_ref().unwrap().value(),
            "FileGroupDescriptorW"
        );
    } else {
        panic!("Expected FormatList PDU");
    }
}

#[test]
fn format_data_request_for_file_list() {
    // Test requesting file list format data
    let request = FormatDataRequest {
        format: ClipboardFormatId::new(0xC0BC),
    };

    let pdu = ClipboardPdu::FormatDataRequest(request);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    let decoded: ClipboardPdu<'_> = ironrdp_core::decode(&encoded).unwrap();
    if let ClipboardPdu::FormatDataRequest(req) = decoded {
        assert_eq!(req.format, ClipboardFormatId::new(0xC0BC));
    } else {
        panic!("Expected FormatDataRequest PDU");
    }
}

#[test]
fn format_data_response_with_file_list() {
    // Test sending file list in format data response
    let file_list = PackedFileList {
        files: vec![
            FileDescriptor::new("file1.txt")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_last_write_time(129010042240261384)
                .with_file_size(1024),
            FileDescriptor::new("file2.dat")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_last_write_time(129010042240261384)
                .with_file_size(2048),
        ],
    };

    let response = FormatDataResponse::new_file_list(&file_list).unwrap();
    let pdu = ClipboardPdu::FormatDataResponse(response);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    let decoded: ClipboardPdu<'_> = ironrdp_core::decode(&encoded).unwrap();
    if let ClipboardPdu::FormatDataResponse(resp) = decoded {
        assert!(!resp.is_error());

        let decoded_list = resp.to_file_list().unwrap();
        assert_eq!(decoded_list.files.len(), 2);
        assert_eq!(decoded_list.files[0].name, "file1.txt");
        assert_eq!(decoded_list.files[0].file_size, Some(1024));
        assert_eq!(decoded_list.files[1].name, "file2.dat");
        assert_eq!(decoded_list.files[1].file_size, Some(2048));
    } else {
        panic!("Expected FormatDataResponse PDU");
    }
}

#[test]
fn empty_file_list() {
    // Test handling empty file lists gracefully
    let file_list = PackedFileList { files: vec![] };

    let response = FormatDataResponse::new_file_list(&file_list).unwrap();
    let pdu = ClipboardPdu::FormatDataResponse(response);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    let decoded: ClipboardPdu<'_> = ironrdp_core::decode(&encoded).unwrap();
    if let ClipboardPdu::FormatDataResponse(resp) = decoded {
        let decoded_list = resp.to_file_list().unwrap();
        assert_eq!(decoded_list.files.len(), 0);
    } else {
        panic!("Expected FormatDataResponse PDU");
    }
}

#[test]
fn file_descriptor_with_minimal_metadata() {
    // Test file descriptor with only filename (no attributes, size, or timestamp)
    let file_desc = FileDescriptor::new("minimal.txt");

    let encoded = ironrdp_core::encode_vec(&file_desc).unwrap();
    let decoded: FileDescriptor = ironrdp_core::decode(&encoded).unwrap();

    assert_eq!(decoded.name, "minimal.txt");
    assert!(decoded.attributes.is_none());
    assert!(decoded.last_write_time.is_none());
    assert!(decoded.file_size.is_none());
}

#[test]
fn file_descriptor_preserves_metadata() {
    // Test that all metadata fields are preserved through encode/decode
    let original = FileDescriptor::new("test_file.doc")
        .with_attributes(ClipboardFileAttributes::READONLY | ClipboardFileAttributes::HIDDEN)
        .with_last_write_time(132489216000000000) // Some arbitrary timestamp
        .with_file_size(9876543210); // Large file size

    let encoded = ironrdp_core::encode_vec(&original).unwrap();
    let decoded: FileDescriptor = ironrdp_core::decode(&encoded).unwrap();

    assert_eq!(decoded.name, original.name);
    assert_eq!(decoded.attributes, original.attributes);
    assert_eq!(decoded.last_write_time, original.last_write_time);
    assert_eq!(decoded.file_size, original.file_size);
}

#[test]
fn packed_file_list_rejects_count_exceeding_max() {
    // Craft a minimal buffer where cItems exceeds MAX_FILE_COUNT.
    // PackedFileList::decode reads a u32 cItems first, then checks
    // against MAX_FILE_COUNT before allocating.
    let count = u32::try_from(MAX_FILE_COUNT + 1).unwrap();
    let buf = count.to_le_bytes();

    let result = ironrdp_core::decode::<PackedFileList>(&buf);
    assert!(result.is_err(), "decode must reject cItems > MAX_FILE_COUNT");
}
