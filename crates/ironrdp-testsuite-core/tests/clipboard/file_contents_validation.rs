/// [MS-RDPECLIP] File Contents Request/Response Validation Tests
///
/// These tests verify spec compliance per MS-RDPECLIP 2.2.5.3, 2.2.5.4, and 3.1.5.4.5-3.1.5.4.8.
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardPdu, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatDataResponse, PackedFileList,
};

// ============================================================================
// Flags Mutual Exclusion Tests (MS-RDPECLIP 2.2.5.3)
// ============================================================================

#[test]
fn test_flags_validation_both_set() {
    // [MS-RDPECLIP] 2.2.5.3 - SIZE and RANGE flags MUST NOT be set simultaneously
    let flags = FileContentsFlags::SIZE | FileContentsFlags::RANGE;
    let result = flags.validate();
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "SIZE and RANGE flags are mutually exclusive per MS-RDPECLIP 2.2.5.3"
    );
}

#[test]
fn test_flags_validation_neither_set() {
    // [MS-RDPECLIP] 2.2.5.3 - Exactly one of SIZE or RANGE must be set
    let flags = FileContentsFlags::empty();
    let result = flags.validate();
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "exactly one of SIZE or RANGE must be set");
}

#[test]
fn test_flags_validation_size_only() {
    // Valid: only SIZE flag set
    let flags = FileContentsFlags::SIZE;
    let result = flags.validate();
    assert!(result.is_ok());
}

#[test]
fn test_flags_validation_range_only() {
    // Valid: only RANGE flag set
    let flags = FileContentsFlags::RANGE;
    let result = flags.validate();
    assert!(result.is_ok());
}

#[test]
fn test_invalid_flags_rejected_during_decode() {
    // [MS-RDPECLIP] 2.2.5.3 - Decoder should reject invalid flag combinations
    let request = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE | FileContentsFlags::RANGE, // Invalid combination
        position: 0,
        requested_size: 8,
        data_id: None,
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);

    // Encode with invalid flags
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    // Decode should fail due to validation
    let result = ironrdp_core::decode::<ClipboardPdu<'_>>(&encoded);
    assert!(result.is_err(), "Decode should reject mutually exclusive flags");
}

// ============================================================================
// SIZE Request Constraints Tests (MS-RDPECLIP 2.2.5.3)
// ============================================================================

#[test]
fn test_size_request_valid_constraints() {
    // [MS-RDPECLIP] 2.2.5.3 - Valid SIZE request: requested_size=8, position=0
    let request = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
    let decoded = ironrdp_core::decode::<ClipboardPdu<'_>>(&encoded).unwrap();

    // Should decode successfully
    assert!(matches!(decoded, ClipboardPdu::FileContentsRequest(_)));
}

#[test]
fn test_size_request_invalid_requested_size() {
    // [MS-RDPECLIP] 2.2.5.3 - SIZE request MUST have requested_size=8
    let request = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 1024, // Invalid - should be 8
        data_id: None,
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    // Decode should fail
    let result = ironrdp_core::decode::<ClipboardPdu<'_>>(&encoded);
    assert!(
        result.is_err(),
        "SIZE request with requested_size != 8 should be rejected"
    );
}

#[test]
fn test_size_request_invalid_position() {
    // [MS-RDPECLIP] 2.2.5.3 - SIZE request MUST have position=0
    let request = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 1024, // Invalid - should be 0
        requested_size: 8,
        data_id: None,
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    // Decode should fail
    let result = ironrdp_core::decode::<ClipboardPdu<'_>>(&encoded);
    assert!(result.is_err(), "SIZE request with position != 0 should be rejected");
}

// ============================================================================
// SIZE Response Data Length Validation Tests (MS-RDPECLIP 2.2.5.4)
// ============================================================================

#[test]
fn test_size_response_valid_length() {
    // [MS-RDPECLIP] 2.2.5.4 - SIZE response with exactly 8 bytes
    let response = FileContentsResponse::new_size_response(1, 1024);

    assert_eq!(response.data().len(), 8);
    assert!(!response.is_error());

    let size = response.data_as_size().unwrap();
    assert_eq!(size, 1024);
}

#[test]
fn test_size_response_invalid_length_too_short() {
    // [MS-RDPECLIP] 2.2.5.4 - SIZE response with wrong length should fail parsing
    let response = FileContentsResponse::new_data_response(1, vec![1, 2, 3, 4]); // Only 4 bytes

    let result = response.data_as_size();
    assert!(result.is_err(), "SIZE response with 4 bytes should fail data_as_size()");
}

#[test]
fn test_size_response_invalid_length_too_long() {
    // [MS-RDPECLIP] 2.2.5.4 - SIZE response with too many bytes should fail
    let response = FileContentsResponse::new_data_response(1, vec![0u8; 16]); // 16 bytes

    let result = response.data_as_size();
    assert!(
        result.is_err(),
        "SIZE response with 16 bytes should fail data_as_size()"
    );
}

// ============================================================================
// FAIL Response Data Validation Tests (MS-RDPECLIP 2.2.5.4)
// ============================================================================

#[test]
fn test_fail_response_zero_length() {
    // [MS-RDPECLIP] 2.2.5.4 - FAIL response MUST have zero-length data
    let response = FileContentsResponse::new_error(42);

    assert!(response.is_error());
    assert_eq!(response.data().len(), 0, "Error response must have zero-length data");
}

#[test]
fn test_fail_response_encoding() {
    // Verify error response encodes with CB_RESPONSE_FAIL flag
    let response = FileContentsResponse::new_error(123);
    let pdu = ClipboardPdu::FileContentsResponse(response);

    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
    let decoded = ironrdp_core::decode::<ClipboardPdu<'_>>(&encoded).unwrap();

    if let ClipboardPdu::FileContentsResponse(resp) = decoded {
        assert!(resp.is_error());
        assert_eq!(resp.data().len(), 0);
        assert_eq!(resp.stream_id(), 123);
    } else {
        panic!("Expected FileContentsResponse");
    }
}

// ============================================================================
// File List Round-Trip Tests
// ============================================================================

#[test]
fn test_file_list_with_file_sizes_for_bounds_validation() {
    // Verify file list round-trip preserves file_size needed for bounds validation
    let file_list = PackedFileList {
        files: vec![
            FileDescriptor::new("small.txt")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_last_write_time(129010042240261384)
                .with_file_size(1024),
            FileDescriptor::new("huge.dat")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_last_write_time(129010042240261384)
                .with_file_size(10_000_000_000), // 10GB
        ],
    };

    let response = FormatDataResponse::new_file_list(&file_list).unwrap();
    let pdu = ClipboardPdu::FormatDataResponse(response);
    let encoded = ironrdp_core::encode_vec(&pdu).unwrap();

    let decoded = ironrdp_core::decode::<ClipboardPdu<'_>>(&encoded).unwrap();
    if let ClipboardPdu::FormatDataResponse(resp) = decoded {
        let decoded_list = resp.to_file_list().unwrap();
        assert_eq!(decoded_list.files.len(), 2);
        assert_eq!(decoded_list.files[0].file_size, Some(1024));
        assert_eq!(decoded_list.files[1].file_size, Some(10_000_000_000));
    } else {
        panic!("Expected FormatDataResponse");
    }
}
