use ironrdp_cliprdr::chunked_fetch::{ChunkedFetch, ChunkedFetchProgress};
use ironrdp_cliprdr::pdu::{FileContentsFlags, FileContentsResponse};

#[test]
fn known_size_skips_the_size_phase() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 64);

    let request = fetch.next_request().expect("fetch not finished");
    assert_eq!(request.flags, FileContentsFlags::RANGE);
    assert_eq!(request.position, 0);
    assert_eq!(request.requested_size, 10);
}

#[test]
fn zero_size_file_completes_immediately() {
    let mut fetch = ChunkedFetch::new(1, 0, 0, 64);
    assert!(fetch.is_finished());
    assert!(fetch.next_request().is_none());
    assert_eq!(fetch.into_data(), Vec::<u8>::new());
}

#[test]
fn size_query_phase_then_fetching() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4);

    let size_request = fetch.next_request().expect("fetch not finished");
    assert_eq!(size_request.flags, FileContentsFlags::SIZE);
    assert_eq!(size_request.position, 0);
    assert_eq!(size_request.requested_size, 8);

    // Nothing until the SIZE response lands.
    assert!(fetch.next_request().is_none());

    let size_response = FileContentsResponse::new_size_response(1, 9);
    let progress = fetch.on_response(&size_response);
    assert_eq!(progress, ChunkedFetchProgress::InProgress);

    let range_request = fetch.next_request().expect("fetch not finished");
    assert_eq!(range_request.flags, FileContentsFlags::RANGE);
    assert_eq!(range_request.position, 0);
    assert_eq!(range_request.requested_size, 4);
}

#[test]
fn size_query_of_zero_completes_without_a_range_request() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_size_response(1, 0));

    assert_eq!(progress, ChunkedFetchProgress::Complete);
    assert!(fetch.into_data().is_empty());
}

#[test]
fn assembles_multiple_chunks_in_order() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4);

    let r1 = fetch.next_request().unwrap();
    assert_eq!((r1.position, r1.requested_size), (0, 4));
    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, b"abcd".as_slice()));
    assert_eq!(progress, ChunkedFetchProgress::InProgress);

    let r2 = fetch.next_request().unwrap();
    assert_eq!((r2.position, r2.requested_size), (4, 4));
    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, b"efgh".as_slice()));
    assert_eq!(progress, ChunkedFetchProgress::InProgress);

    // Last chunk is shorter than chunk_size: 10 - 8 = 2 bytes remain.
    let r3 = fetch.next_request().unwrap();
    assert_eq!((r3.position, r3.requested_size), (8, 2));
    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, b"ij".as_slice()));
    assert_eq!(progress, ChunkedFetchProgress::Complete);

    assert_eq!(fetch.into_data(), b"abcdefghij".to_vec());
}

#[test]
fn next_request_returns_none_while_a_response_is_outstanding() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4);

    assert!(fetch.next_request().is_some());
    // A second call before on_response() must not produce a duplicate/overlapping request.
    assert!(fetch.next_request().is_none());
}

#[test]
fn error_response_fails_the_fetch() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_error(1));

    assert_eq!(progress, ChunkedFetchProgress::Failed);
    assert!(fetch.is_finished());
    assert!(fetch.next_request().is_none());
}

#[test]
fn malformed_size_response_fails_the_fetch() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4);
    let _ = fetch.next_request();

    // Not exactly 8 bytes, per MS-RDPECLIP 2.2.5.4 SIZE response contract.
    let malformed = FileContentsResponse::new_data_response(1, b"abc".as_slice());
    let progress = fetch.on_response(&malformed);

    assert_eq!(progress, ChunkedFetchProgress::Failed);
}

#[test]
fn empty_data_before_completion_fails_rather_than_looping_forever() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, Vec::<u8>::new()));

    assert_eq!(progress, ChunkedFetchProgress::Failed);
}

#[test]
fn oversized_response_is_clamped_to_the_remaining_bytes() {
    let mut fetch = ChunkedFetch::new(1, 0, 4, 64);
    let _ = fetch.next_request();

    // Peer sends more than the file's total size; must not overrun.
    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, b"abcdEXTRA".as_slice()));

    assert_eq!(progress, ChunkedFetchProgress::Complete);
    assert_eq!(fetch.into_data(), b"abcd".to_vec());
}

#[test]
fn stream_id_is_stable_across_the_whole_fetch() {
    let fetch = ChunkedFetch::new(42, 0, 10, 4);
    assert_eq!(fetch.stream_id(), 42);
}
