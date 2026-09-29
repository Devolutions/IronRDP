//! Correlation of `FormatDataResponse`s with the `FormatDataRequest`s that
//! produced them when a second paste is initiated before the first is answered.
//!
//! A FormatDataResponse names no format, so `Cliprdr` must work out which
//! request a response answers. Responses arrive in the order the requests were
//! sent, so the Nth response answers the Nth request. This matters in practice:
//! several Windows applications (Firefox, Word) announce a single copy with two
//! FormatLists a few milliseconds apart, and a backend that fetches on each
//! announcement has a request outstanding when it issues the next one.

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardPdu, FileDescriptor,
    FormatDataResponse, FormatList, PackedFileList,
};
use ironrdp_svc::SvcProcessor as _;

use super::test_helpers::{FormatDataRecordingBackend, init_ready_client_with_backend};

fn file_list_format() -> ClipboardFormatId {
    ClipboardFormatId::new(0xC0BC)
}

/// The remote announces a copy offering text and, optionally, a file list.
fn announce(cliprdr: &mut CliprdrClient, with_file_list: bool) {
    let mut formats = vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
    if with_file_list {
        formats.push(ClipboardFormat::new(file_list_format()).with_name(ClipboardFormatName::FILE_LIST));
    }
    let format_list = ClipboardPdu::FormatList(FormatList::new_unicode(&formats, true).unwrap());
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list).unwrap())
        .unwrap();
}

/// A ready client whose remote clipboard offers text and a file list.
fn ready_client() -> CliprdrClient {
    let mut cliprdr = init_ready_client_with_backend(Box::new(FormatDataRecordingBackend::default()));
    announce(&mut cliprdr, true);
    cliprdr
}

fn backend(cliprdr: &CliprdrClient) -> &FormatDataRecordingBackend {
    cliprdr.downcast_backend::<FormatDataRecordingBackend>().unwrap()
}

fn paste(cliprdr: &mut CliprdrClient, format: ClipboardFormatId) {
    let _: Vec<_> = cliprdr.initiate_paste(format).unwrap().into();
}

fn respond(cliprdr: &mut CliprdrClient, response: FormatDataResponse<'_>) {
    let pdu = ClipboardPdu::FormatDataResponse(response);
    let _: Vec<_> = cliprdr.process(&ironrdp_core::encode_vec(&pdu).unwrap()).unwrap();
}

fn file_list_response() -> FormatDataResponse<'static> {
    let list = PackedFileList {
        files: vec![
            FileDescriptor::new("document.pdf")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_file_size(1024),
        ],
    };
    FormatDataResponse::new_file_list(&list).unwrap()
}

fn text_response() -> FormatDataResponse<'static> {
    FormatDataResponse::new_unicode_string("hello")
}

/// Control: one request, one response — the file list is recognised.
#[test]
fn a_single_file_list_request_is_recognised() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, file_list_format());
    respond(&mut cliprdr, file_list_response());

    assert_eq!(backend(&cliprdr).file_lists.len(), 1);
    assert!(backend(&cliprdr).data_responses.is_empty());
}

/// Text requested, then the file list requested before the text arrived. The
/// first response answers the TEXT request, so it must reach the backend as
/// data, and the second — the real file list — must be recognised as one.
#[test]
fn a_text_response_is_not_taken_for_a_later_file_list_request() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT);
    paste(&mut cliprdr, file_list_format());

    respond(&mut cliprdr, text_response());
    respond(&mut cliprdr, file_list_response());

    let seen = backend(&cliprdr);
    assert_eq!(seen.file_lists.len(), 1, "the file list must be recognised");
    assert_eq!(seen.file_lists[0][0].name, "document.pdf");
    assert_eq!(
        seen.data_responses,
        vec![text_response().data().to_vec()],
        "the text response must reach the backend as data"
    );
}

/// The reverse order: the file list requested, then text before the file list
/// arrived. The first response is the file list and must be recognised as one
/// rather than handed to the backend as if it were the text.
#[test]
fn a_file_list_response_is_not_taken_for_a_later_text_request() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, file_list_format());
    paste(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT);

    respond(&mut cliprdr, file_list_response());
    respond(&mut cliprdr, text_response());

    let seen = backend(&cliprdr);
    assert_eq!(seen.file_lists.len(), 1, "the file list must be recognised");
    assert_eq!(
        seen.data_responses,
        vec![text_response().data().to_vec()],
        "only the text response reaches the backend as data"
    );
}

/// The live trigger: the remote announces the same copy twice, and the second
/// FormatList arrives while the file list request made after the first is
/// still unanswered. That request is still answered, so its response is still
/// the file list.
#[test]
fn a_repeated_format_list_does_not_orphan_an_outstanding_request() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, file_list_format());
    announce(&mut cliprdr, true);
    respond(&mut cliprdr, file_list_response());

    assert_eq!(
        backend(&cliprdr).file_lists.len(),
        1,
        "the file list must be recognised"
    );
    assert!(backend(&cliprdr).data_responses.is_empty());
}

/// A response is classified by what was requested, not by the clipboard state
/// when it arrives: a FormatList that no longer offers a file list must not
/// turn the answer to a file list request into plain data.
#[test]
fn a_file_list_request_is_classified_when_it_is_sent() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, file_list_format());
    announce(&mut cliprdr, false);
    respond(&mut cliprdr, file_list_response());

    assert_eq!(
        backend(&cliprdr).file_lists.len(),
        1,
        "the file list must be recognised"
    );
    assert!(backend(&cliprdr).data_responses.is_empty());
}

/// A response is never given up on, however late it arrives: a response
/// carries no ID, so a late answer to an abandoned request would pair with the
/// next request instead. Here the text request is answered ten minutes late,
/// after the timer has run and a file list request has been queued behind it.
#[test]
fn a_late_response_is_still_correlated() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT);

    backend(&cliprdr).advance_ms(10 * 60 * 1000);
    let _: Vec<_> = cliprdr.drive_timeouts().unwrap().into();
    paste(&mut cliprdr, file_list_format());

    respond(&mut cliprdr, text_response());
    respond(&mut cliprdr, file_list_response());

    let seen = backend(&cliprdr);
    assert_eq!(seen.file_lists.len(), 1, "the file list must be recognised");
    assert_eq!(seen.data_responses, vec![text_response().data().to_vec()]);
    assert_eq!(seen.error_responses, 0, "nothing was answered with a synthetic error");
}

/// A failed response still answers its request: the one behind it stays
/// correlated.
#[test]
fn a_failed_response_keeps_later_responses_aligned() {
    let mut cliprdr = ready_client();
    paste(&mut cliprdr, file_list_format());
    paste(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT);

    respond(&mut cliprdr, FormatDataResponse::new_error());
    respond(&mut cliprdr, text_response());

    let seen = backend(&cliprdr);
    assert_eq!(seen.error_responses, 1, "the file list request failed");
    assert!(seen.file_lists.is_empty());
    assert_eq!(seen.data_responses, vec![text_response().data().to_vec()]);
}

/// A peer that stops answering cannot grow the queue without bound: past the
/// cap, `initiate_paste` fails instead of queueing.
#[test]
fn the_pending_request_queue_is_capped() {
    let mut cliprdr = ready_client();
    for _ in 0..64 {
        paste(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT);
    }
    assert!(cliprdr.initiate_paste(ClipboardFormatId::CF_UNICODETEXT).is_err());
}
