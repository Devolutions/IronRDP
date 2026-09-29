//! Correlation of `FormatDataResponse`s with the `FormatDataRequest`s that
//! produced them when a second paste is initiated before the first is answered.
//!
//! A FormatDataResponse names no format, so `Cliprdr` must work out which
//! request a response answers. Responses arrive in the order the requests were
//! sent, so the Nth response answers the Nth request. This matters in practice:
//! several Windows applications (Firefox, Word) announce a single copy with two
//! FormatLists a few milliseconds apart, and a backend that fetches on each
//! announcement has a request outstanding when it issues the next one.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, FileDescriptor, FormatDataResponse, FormatList, FormatListResponse, PackedFileList,
};
use ironrdp_core::{AsAny, IntoOwned as _};
use ironrdp_svc::SvcProcessor as _;

fn file_list_format() -> ClipboardFormatId {
    ClipboardFormatId::new(0xC0BC)
}

#[derive(Debug, Default)]
struct Seen {
    /// Payloads delivered through `on_format_data_response` (`None` = an error response).
    data_responses: Vec<Option<Vec<u8>>>,
    file_lists: Vec<Vec<FileDescriptor>>,
}

#[derive(Debug)]
struct RecordingBackend {
    seen: Arc<Mutex<Seen>>,
}

impl CliprdrBackend for RecordingBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp/test"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
    }

    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _capabilities: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _available_formats: &[ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _request: ironrdp_cliprdr::pdu::FormatDataRequest) {}

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let payload = (!response.is_error()).then(|| response.data().to_vec());
        self.seen.lock().unwrap().data_responses.push(payload);
    }

    fn on_file_contents_request(&mut self, _request: ironrdp_cliprdr::pdu::FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _response: ironrdp_cliprdr::pdu::FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _data_id: ironrdp_cliprdr::pdu::LockDataId) {}
    fn on_unlock(&mut self, _data_id: ironrdp_cliprdr::pdu::LockDataId) {}

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], _clip_data_id: Option<u32>) {
        self.seen.lock().unwrap().file_lists.push(files.to_vec());
    }

    fn now_ms(&self) -> u64 {
        0
    }

    fn elapsed_ms(&self, _since: u64) -> u64 {
        0
    }
}

impl AsAny for RecordingBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

/// A ready client whose remote clipboard offers text and a file list.
fn ready_client() -> (CliprdrClient, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut cliprdr = CliprdrClient::new(Box::new(RecordingBackend {
        seen: Arc::clone(&seen),
    }));

    let _: Vec<_> = cliprdr.initiate_copy(&[]).unwrap().into();
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&ClipboardPdu::FormatListResponse(FormatListResponse::Ok)).unwrap())
        .unwrap();

    let formats = vec![
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(file_list_format()).with_name(ClipboardFormatName::FILE_LIST),
    ];
    let format_list = ClipboardPdu::FormatList(FormatList::new_unicode(&formats, true).unwrap());
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list).unwrap())
        .unwrap();

    (cliprdr, seen)
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
    FormatDataResponse::new_file_list(&list).unwrap().into_owned()
}

fn text_response() -> FormatDataResponse<'static> {
    FormatDataResponse::new_unicode_string("hello").into_owned()
}

/// Control: one request, one response — the file list is recognised.
#[test]
fn a_single_file_list_request_is_recognised() {
    let (mut cliprdr, seen) = ready_client();
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format()).unwrap().into();
    respond(&mut cliprdr, file_list_response());

    let seen = seen.lock().unwrap();
    assert_eq!(seen.file_lists.len(), 1);
    assert!(seen.data_responses.is_empty());
}

/// Text requested, then the file list requested before the text arrived. The
/// first response answers the TEXT request, so it must reach the backend as
/// data, and the second — the real file list — must be recognised as one.
#[test]
fn a_text_response_is_not_taken_for_a_later_file_list_request() {
    let (mut cliprdr, seen) = ready_client();
    let _: Vec<_> = cliprdr
        .initiate_paste(ClipboardFormatId::CF_UNICODETEXT)
        .unwrap()
        .into();
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format()).unwrap().into();

    respond(&mut cliprdr, text_response());
    respond(&mut cliprdr, file_list_response());

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.file_lists.len(),
        1,
        "the file list response must be recognised as the file list"
    );
    assert_eq!(seen.file_lists[0][0].name, "document.pdf");
    assert_eq!(
        seen.data_responses,
        vec![Some(text_response().data().to_vec())],
        "the text response must reach the backend as data"
    );
}

/// The reverse order: the file list requested, then text before the file list
/// arrived. The first response is the file list and must be recognised as one
/// rather than handed to the backend as if it were the text.
#[test]
fn a_file_list_response_is_not_taken_for_a_later_text_request() {
    let (mut cliprdr, seen) = ready_client();
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format()).unwrap().into();
    let _: Vec<_> = cliprdr
        .initiate_paste(ClipboardFormatId::CF_UNICODETEXT)
        .unwrap()
        .into();

    respond(&mut cliprdr, file_list_response());
    respond(&mut cliprdr, text_response());

    let seen = seen.lock().unwrap();
    assert_eq!(seen.file_lists.len(), 1, "the file list must be recognised");
    assert_eq!(
        seen.data_responses,
        vec![Some(text_response().data().to_vec())],
        "only the text response reaches the backend as data"
    );
}

/// The live trigger: the remote announces the same copy twice, and the second
/// FormatList arrives while the file list request made after the first is
/// still unanswered. That request is still answered, so its response is still
/// the file list.
#[test]
fn a_repeated_format_list_does_not_orphan_an_outstanding_request() {
    let (mut cliprdr, seen) = ready_client();
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format()).unwrap().into();

    let formats = vec![
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(file_list_format()).with_name(ClipboardFormatName::FILE_LIST),
    ];
    let format_list = ClipboardPdu::FormatList(FormatList::new_unicode(&formats, true).unwrap());
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list).unwrap())
        .unwrap();

    respond(&mut cliprdr, file_list_response());

    let seen = seen.lock().unwrap();
    assert_eq!(seen.file_lists.len(), 1, "the file list must be recognised");
    assert!(seen.data_responses.is_empty());
}
