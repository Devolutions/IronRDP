use ironrdp_cliprdr::chunked_fetch::{ChunkedFetch, ChunkedFetchProgress};
use ironrdp_cliprdr::pdu::{FileContentsFlags, FileContentsResponse};

/// Generous enough to be a no-op against every file size used in these tests, so tests not
/// about the cap itself don't have to think about it.
const NO_EFFECTIVE_CAP: u64 = u64::MAX;

#[test]
fn known_size_skips_the_size_phase() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 64, None, NO_EFFECTIVE_CAP);

    let request = fetch.next_request().expect("fetch not finished");
    assert_eq!(request.flags, FileContentsFlags::RANGE);
    assert_eq!(request.position, 0);
    assert_eq!(request.requested_size, 10);
}

#[test]
fn zero_size_file_completes_immediately() {
    let mut fetch = ChunkedFetch::new(1, 0, 0, 64, None, NO_EFFECTIVE_CAP);
    assert!(fetch.is_finished());
    assert!(fetch.next_request().is_none());
    assert_eq!(fetch.into_data(), Vec::<u8>::new());
}

#[test]
fn size_query_phase_then_fetching() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4, None, NO_EFFECTIVE_CAP);

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
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4, None, NO_EFFECTIVE_CAP);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_size_response(1, 0));

    assert_eq!(progress, ChunkedFetchProgress::Complete);
    assert!(fetch.into_data().is_empty());
}

#[test]
fn assembles_multiple_chunks_in_order() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4, None, NO_EFFECTIVE_CAP);

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
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4, None, NO_EFFECTIVE_CAP);

    assert!(fetch.next_request().is_some());
    // A second call before on_response() must not produce a duplicate/overlapping request.
    assert!(fetch.next_request().is_none());
}

#[test]
fn error_response_fails_the_fetch() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4, None, NO_EFFECTIVE_CAP);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_error(1));

    assert_eq!(progress, ChunkedFetchProgress::Failed);
    assert!(fetch.is_finished());
    assert!(fetch.next_request().is_none());
}

#[test]
fn malformed_size_response_fails_the_fetch() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4, None, NO_EFFECTIVE_CAP);
    let _ = fetch.next_request();

    // Not exactly 8 bytes, per MS-RDPECLIP 2.2.5.4 SIZE response contract.
    let malformed = FileContentsResponse::new_data_response(1, b"abc".as_slice());
    let progress = fetch.on_response(&malformed);

    assert_eq!(progress, ChunkedFetchProgress::Failed);
}

#[test]
fn empty_data_before_completion_fails_rather_than_looping_forever() {
    let mut fetch = ChunkedFetch::new(1, 0, 10, 4, None, NO_EFFECTIVE_CAP);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, Vec::<u8>::new()));

    assert_eq!(progress, ChunkedFetchProgress::Failed);
}

#[test]
fn response_exceeding_requested_size_fails_the_fetch() {
    let mut fetch = ChunkedFetch::new(1, 0, 4, 64, None, NO_EFFECTIVE_CAP);
    let request = fetch.next_request().unwrap();
    assert_eq!(
        request.requested_size, 4,
        "requested exactly the file's remaining 4 bytes"
    );

    // Peer sends more than cbRequested. MS-RDPECLIP 2.2.5.3 makes that a protocol
    // violation regardless of whether it would still fit under the file's total size.
    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, b"abcdEXTRA".as_slice()));

    assert_eq!(progress, ChunkedFetchProgress::Failed);
}

#[test]
fn response_within_requested_size_still_clamps_against_total_size_as_a_backstop() {
    // Feeding a response with no prior next_request() call (so last_requested_size is
    // still None) exercises the defensive total_size clamp on its own, independent of
    // the cbRequested check above.
    let mut fetch = ChunkedFetch::new(1, 0, 4, 64, None, NO_EFFECTIVE_CAP);

    let progress = fetch.on_response(&FileContentsResponse::new_data_response(1, b"abcdEXTRA".as_slice()));

    assert_eq!(progress, ChunkedFetchProgress::Complete);
    assert_eq!(fetch.into_data(), b"abcd".to_vec());
}

#[test]
fn stream_id_is_stable_across_the_whole_fetch() {
    let fetch = ChunkedFetch::new(42, 0, 10, 4, None, NO_EFFECTIVE_CAP);
    assert_eq!(fetch.stream_id(), 42);
}

#[test]
fn clip_data_id_is_sent_on_every_request_instead_of_left_for_the_caller_to_default() {
    // A FormatList between requests can change what current_lock_id would default to;
    // ChunkedFetch must keep sending the id it was constructed with, not None.
    let mut fetch = ChunkedFetch::new(1, 0, 8, 4, Some(7), NO_EFFECTIVE_CAP);

    let r1 = fetch.next_request().unwrap();
    assert_eq!(r1.data_id, Some(7));
    let _ = fetch.on_response(&FileContentsResponse::new_data_response(1, b"abcd".as_slice()));

    let r2 = fetch.next_request().unwrap();
    assert_eq!(r2.data_id, Some(7));
}

#[test]
fn clip_data_id_is_sent_on_the_size_request_too() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 4, Some(3), NO_EFFECTIVE_CAP);

    let size_request = fetch.next_request().unwrap();
    assert_eq!(size_request.data_id, Some(3));
}

#[test]
fn known_size_over_the_cap_fails_before_any_request_is_issued() {
    let mut fetch = ChunkedFetch::new(1, 0, 1_000_000, 64, None, 10);

    assert!(fetch.is_finished());
    assert!(fetch.next_request().is_none());
}

#[test]
fn queried_size_over_the_cap_fails_before_any_range_request() {
    let mut fetch = ChunkedFetch::new_with_size_query(1, 0, 64, None, 10);
    let _ = fetch.next_request();

    let progress = fetch.on_response(&FileContentsResponse::new_size_response(1, 1_000_000));

    assert_eq!(progress, ChunkedFetchProgress::Failed);
    assert!(fetch.next_request().is_none());
}
