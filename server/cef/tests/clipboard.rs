#![cfg(feature = "runtime")]

use ironrdp_cef_server::clipboard::{Clipboard, ClipboardItem, MAX_BYTES, Snapshot};
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory as _};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, FileContentsFlags, FileContentsResponse, FileDescriptor,
    FormatDataResponse,
};
use ironrdp_server::{ServerEvent, ServerEventSender as _};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::sync::watch;

const CF_HTML_COPIED: &[u8] = b"Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000190\r\nStartFragment:0000000141\r\nEndFragment:0000000154\r\n<html>\r\n<body>\r\n<!--StartFragment--><b>copied</b><!--EndFragment-->\r\n</body>\r\n</html>";
const CF_HTML_NEW: &[u8] = b"Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000187\r\nStartFragment:0000000141\r\nEndFragment:0000000151\r\n<html>\r\n<body>\r\n<!--StartFragment--><b>new</b><!--EndFragment-->\r\n</body>\r\n</html>";
const CF_HTML_PAGE: &[u8] = b"Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000188\r\nStartFragment:0000000141\r\nEndFragment:0000000152\r\n<html>\r\n<body>\r\n<!--StartFragment--><i>page</i><!--EndFragment-->\r\n</body>\r\n</html>";

struct Fixture {
    clipboard: Clipboard,
    backend: Box<dyn CliprdrBackend>,
    browser: watch::Receiver<Option<ClipboardItem>>,
    events: UnboundedReceiver<ServerEvent>,
}

impl Fixture {
    fn new() -> Self {
        Self::new_with_staging_dir(None)
    }

    fn new_with_staging_dir(staging_dir: Option<std::path::PathBuf>) -> Self {
        let (clipboard, browser) = Clipboard::new();
        let mut clipboard = match staging_dir {
            Some(path) => clipboard.with_staging_dir(path),
            None => clipboard,
        };
        let (sender, events) = unbounded_channel();
        clipboard.set_sender(sender);
        let backend = clipboard.build_cliprdr_backend();
        Self {
            clipboard,
            backend,
            browser,
            events,
        }
    }

    fn next(&mut self) -> ClipboardMessage {
        match self.events.try_recv().expect("clipboard message") {
            ServerEvent::Clipboard(message) => message,
            ServerEvent::ClipboardWithCompletion(message, _) => message,
            ServerEvent::ClipboardWithValidity { message, .. } => message,
            other => panic!("unexpected server event {other:?}"),
        }
    }

    fn expect_paste(&mut self) -> ClipboardFormatId {
        match self.next() {
            ClipboardMessage::SendInitiatePaste(format) => format,
            other => panic!("expected a format data request, got {other:?}"),
        }
    }

    fn browser_item(&mut self) -> Snapshot {
        self.browser
            .borrow_and_update()
            .as_ref()
            .expect("browser clipboard item")
            .snapshot
            .clone()
    }
}

// oracle: spec MS-RDPECLIP §§2.2.5.3-2.2.5.4
#[test]
fn client_file_copy_requests_bounded_ranges_and_stages_the_contents() {
    let staging = tempfile::tempdir().expect("staging directory");
    let mut fixture = Fixture::new_with_staging_dir(Some(staging.path().to_path_buf()));
    assert!(
        fixture
            .backend
            .client_capabilities()
            .contains(ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS)
    );
    fixture.backend.on_process_negotiated_capabilities(
        ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA,
    );
    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId(0xC0FF)).with_name(ClipboardFormatName::FILE_LIST)]);
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0FF));

    let file_size = 64 * 1024 + 1;
    fixture
        .backend
        .on_remote_file_list(&[FileDescriptor::new("client.bin")], Some(7));
    let ClipboardMessage::SendFileContentsRequest(size_request) = fixture.next() else {
        panic!("expected a file-size request");
    };
    assert_eq!(size_request.flags, FileContentsFlags::SIZE);
    assert_eq!(size_request.data_id, Some(7));

    fixture
        .backend
        .on_file_contents_response(FileContentsResponse::new_size_response(
            size_request.stream_id,
            file_size as u64,
        ));
    let ClipboardMessage::SendFileContentsRequest(range_request) = fixture.next() else {
        panic!("expected a file-range request");
    };
    let mut position = 0_u64;
    let mut request = range_request;
    while position < file_size as u64 {
        assert_eq!(request.flags, FileContentsFlags::RANGE);
        assert_eq!(request.position, position);
        let requested_size = u64::from(request.requested_size);
        assert!(0 < requested_size && requested_size <= file_size as u64 - position);
        fixture
            .backend
            .on_file_contents_response(FileContentsResponse::new_data_response(
                request.stream_id,
                vec![0x5A; usize::try_from(requested_size).unwrap()],
            ));
        position += requested_size;
        if position < file_size as u64 {
            let ClipboardMessage::SendFileContentsRequest(next_request) = fixture.next() else {
                panic!("expected the remaining file range request");
            };
            request = next_request;
        }
    }
    let paths = fixture
        .browser
        .borrow_and_update()
        .as_ref()
        .expect("browser clipboard item")
        .files
        .clone();
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).expect("staged file"), vec![0x5A; file_size]);
}

// oracle: spec MS-RDPECLIP §2.2.5.4
#[test]
fn client_file_copy_stages_zero_byte_files() {
    let staging = tempfile::tempdir().expect("staging directory");
    let mut fixture = Fixture::new_with_staging_dir(Some(staging.path().to_path_buf()));
    fixture.backend.on_process_negotiated_capabilities(
        ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    );
    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId(0xC0FF)).with_name(ClipboardFormatName::FILE_LIST)]);
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0FF));
    fixture
        .backend
        .on_remote_file_list(&[FileDescriptor::new("empty.txt")], None);
    let ClipboardMessage::SendFileContentsRequest(size_request) = fixture.next() else {
        panic!("expected a file-size request");
    };
    assert_eq!(size_request.flags, FileContentsFlags::SIZE);

    fixture
        .backend
        .on_file_contents_response(FileContentsResponse::new_size_response(size_request.stream_id, 0));

    let item = fixture
        .browser
        .borrow_and_update()
        .clone()
        .expect("browser clipboard item");
    assert_eq!(item.files.len(), 1);
    assert_eq!(std::fs::metadata(&item.files[0]).expect("staged file").len(), 0);
    assert!(
        fixture.events.try_recv().is_err(),
        "zero-byte file triggered an unnecessary range request"
    );
}

// oracle: spec MS-RDPECLIP §3.1.5.2.1
#[test]
fn new_clipboard_copy_replaces_text_and_file_contents() {
    let staging = tempfile::tempdir().expect("staging directory");
    let mut fixture = Fixture::new_with_staging_dir(Some(staging.path().to_path_buf()));
    fixture.backend.on_process_negotiated_capabilities(
        ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    );
    fixture.backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(ClipboardFormatId(0xC0FF)).with_name(ClipboardFormatName::FILE_LIST),
    ]);
    assert!(fixture.browser.borrow_and_update().is_none());

    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("copied"));
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0FF));
    fixture
        .backend
        .on_remote_file_list(&[FileDescriptor::new("client.bin")], None);
    // oracle: mutant publish a partial browser clipboard snapshot while the file format is still pending.
    assert!(!fixture.browser.has_changed().expect("clipboard sender"));
    let ClipboardMessage::SendFileContentsRequest(size_request) = fixture.next() else {
        panic!("expected a file-size request");
    };
    fixture
        .backend
        .on_file_contents_response(FileContentsResponse::new_size_response(size_request.stream_id, 1));
    let ClipboardMessage::SendFileContentsRequest(data_request) = fixture.next() else {
        panic!("expected a file-range request");
    };
    fixture
        .backend
        .on_file_contents_response(FileContentsResponse::new_data_response(
            data_request.stream_id,
            vec![0x5A],
        ));
    let item = fixture
        .browser
        .borrow_and_update()
        .clone()
        .expect("browser clipboard item");
    assert_eq!(item.snapshot.text.as_deref(), Some("copied"));
    assert_eq!(item.files.len(), 1);

    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    // oracle: mutant replace the browser clipboard before the newly advertised content is fetched.
    assert!(!fixture.browser.has_changed().expect("clipboard sender"));
    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("replacement"));
    let item = fixture
        .browser
        .borrow_and_update()
        .clone()
        .expect("browser clipboard item");
    assert_eq!(item.snapshot.text.as_deref(), Some("replacement"));
    assert!(item.files.is_empty());
}

// oracle: mutant failed file-list requests must not block other clipboard formats.
#[test]
fn failed_file_list_does_not_block_text() {
    let staging = tempfile::tempdir().expect("staging directory");
    let mut fixture = Fixture::new_with_staging_dir(Some(staging.path().to_path_buf()));
    fixture.backend.on_process_negotiated_capabilities(
        ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    );
    fixture.backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(ClipboardFormatId(0xC0FF)).with_name(ClipboardFormatName::FILE_LIST),
    ]);
    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("copied"));
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0FF));
    fixture.backend.on_format_data_response(FormatDataResponse::new_error());

    let item = fixture
        .browser
        .borrow_and_update()
        .clone()
        .expect("browser clipboard item");
    assert_eq!(item.snapshot.text.as_deref(), Some("copied"));
    assert!(item.files.is_empty());
}

// oracle: mutant stale file-transfer data replaces a newer clipboard copy.
#[test]
fn superseded_file_transfer_cannot_replace_the_new_copy() {
    let staging = tempfile::tempdir().expect("staging directory");
    let mut fixture = Fixture::new_with_staging_dir(Some(staging.path().to_path_buf()));
    fixture.backend.on_process_negotiated_capabilities(
        ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    );
    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId(0xC0FF)).with_name(ClipboardFormatName::FILE_LIST)]);
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0FF));
    fixture
        .backend
        .on_remote_file_list(&[FileDescriptor::new("client.bin")], None);
    let ClipboardMessage::SendFileContentsRequest(size_request) = fixture.next() else {
        panic!("expected a file-size request");
    };

    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    fixture
        .backend
        .on_file_contents_response(FileContentsResponse::new_size_response(size_request.stream_id, 1));
    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("replacement"));
    let item = fixture
        .browser
        .borrow_and_update()
        .clone()
        .expect("browser clipboard item");
    assert_eq!(item.snapshot.text.as_deref(), Some("replacement"));
    assert!(item.files.is_empty());
    // oracle: mutant stale file response queues a request for the superseded copy.
    assert!(
        fixture.events.try_recv().is_err(),
        "a stale file request remained queued"
    );
}

// oracle: mutant a client filename escapes the configured staging root.
#[test]
fn client_file_copy_rejects_path_components() {
    let root = tempfile::tempdir().expect("temporary root");
    let staging = root.path().join("staging");
    let mut fixture = Fixture::new_with_staging_dir(Some(staging));
    fixture.backend.on_process_negotiated_capabilities(
        ironrdp_cliprdr::pdu::ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    );
    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId(0xC0FF)).with_name(ClipboardFormatName::FILE_LIST)]);
    let _ = fixture.expect_paste();

    fixture
        .backend
        .on_remote_file_list(&[FileDescriptor::new("../../../escape.txt")], None);

    assert!(fixture.events.try_recv().is_err());
    assert!(!root.path().join("escape.txt").exists());
    let item = fixture
        .browser
        .borrow_and_update()
        .clone()
        .expect("browser clipboard item");
    assert!(item.files.is_empty());
}

fn html_format(id: u32) -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId(id)).with_name(ClipboardFormatName::HTML)
}

// oracle: spec MS-RDPECLIP §2.2.5.2
#[test]
fn client_copy_delivers_one_item_with_all_supported_formats() {
    let mut fixture = Fixture::new();
    fixture.backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        html_format(0xC0DE),
    ]);

    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("copied"));
    // oracle: mutant publish an incomplete browser clipboard snapshot while other advertised formats are pending.
    assert!(!fixture.browser.has_changed().expect("clipboard sender"));

    // oracle: spec MS-RDPECLIP §2.2.5.1
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0DE));
    // oracle: spec Microsoft HTML Clipboard Format §Description Headers and Offsets
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_data(CF_HTML_COPIED.to_vec()));

    assert_eq!(
        fixture.browser_item(),
        Snapshot {
            text: Some("copied".to_owned()),
            html: Some("<b>copied</b>".to_owned()),
            png: None,
        }
    );
}

// oracle: mutant a delayed response for an old clipboard copy replaces the current copy.
#[test]
fn superseded_client_copy_response_is_discarded() {
    let mut fixture = Fixture::new();
    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);

    fixture.backend.on_remote_copy(&[html_format(0xC0DE)]);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("old"));

    // oracle: spec MS-RDPECLIP §2.2.5.1
    assert_eq!(fixture.expect_paste(), ClipboardFormatId(0xC0DE));
    // oracle: spec Microsoft HTML Clipboard Format §Description Headers and Offsets
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_data(CF_HTML_NEW.to_vec()));
    assert_eq!(
        fixture.browser_item(),
        Snapshot {
            text: None,
            html: Some("<b>new</b>".to_owned()),
            png: None,
        }
    );
}

// oracle: spec MS-RDPECLIP §3.1.5.2.1
#[test]
fn unsupported_client_copy_clears_previous_supported_content() {
    let mut fixture = Fixture::new();
    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("old"));
    assert_eq!(
        fixture.browser_item(),
        Snapshot {
            text: Some("old".to_owned()),
            html: None,
            png: None,
        }
    );

    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId(0xFFFF))]);

    assert_eq!(fixture.browser_item(), Snapshot::default());
}

// oracle: mutant intermediate clipboard copies replace the latest queued browser snapshot.
#[test]
fn latest_client_copy_replaces_queued_updates() {
    let mut fixture = Fixture::new();
    for text in ["one", "two", "three", "four", "five"] {
        fixture
            .backend
            .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
        assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
        fixture
            .backend
            .on_format_data_response(FormatDataResponse::new_unicode_string(text));
    }

    assert_eq!(
        fixture.browser_item(),
        Snapshot {
            text: Some("five".to_owned()),
            html: None,
            png: None,
        }
    );
}

// oracle: spec MS-RDPECLIP §2.2.5.1
#[test]
fn browser_copy_is_served_in_the_advertised_formats() {
    let mut fixture = Fixture::new();
    fixture.clipboard.offer(Snapshot {
        text: Some("page text".to_owned()),
        html: Some("<i>page</i>".to_owned()),
        png: None,
    });
    let ClipboardMessage::SendInitiateCopy(formats) = fixture.next() else {
        panic!("expected a format list");
    };
    // oracle: spec Microsoft HTML Clipboard Format §Description
    let html = formats
        .iter()
        .find(|format| format.name() == Some(&ClipboardFormatName::HTML))
        .expect("HTML format")
        .id();
    assert!(
        formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT)
    );

    fixture
        .backend
        .on_format_data_request(ironrdp_cliprdr::pdu::FormatDataRequest { format: html });
    let ClipboardMessage::SendFormatData(response) = fixture.next() else {
        panic!("expected format data");
    };
    // oracle: spec Microsoft HTML Clipboard Format §Description Headers and Offsets
    assert_eq!(response.data(), CF_HTML_PAGE);

    fixture
        .backend
        .on_format_data_request(ironrdp_cliprdr::pdu::FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });
    let ClipboardMessage::SendFormatData(response) = fixture.next() else {
        panic!("expected format data");
    };
    assert_eq!(response.to_unicode_string().expect("text"), "page text");
}

// oracle: spec MS-RDPECLIP §2.2.5.2
#[test]
fn unavailable_format_request_fails() {
    let mut fixture = Fixture::new();
    fixture.clipboard.offer(Snapshot {
        text: Some("page text".to_owned()),
        html: None,
        png: None,
    });
    let _ = fixture.next();
    fixture
        .backend
        .on_format_data_request(ironrdp_cliprdr::pdu::FormatDataRequest {
            format: ClipboardFormatId::CF_DIB,
        });
    let ClipboardMessage::SendFormatData(response) = fixture.next() else {
        panic!("expected format data");
    };
    assert!(response.is_error());
}

// oracle: spec MS-RDPECLIP §2.2.5.2; mutant emitting an encoded format response larger than MAX_BYTES.
#[test]
fn outbound_format_data_exceeding_max_bytes_on_wire_fails() {
    let mut fixture = Fixture::new();
    fixture.clipboard.offer(Snapshot {
        text: Some("a".repeat(MAX_BYTES / 2)),
        html: None,
        png: None,
    });
    let _ = fixture.next();

    fixture
        .backend
        .on_format_data_request(ironrdp_cliprdr::pdu::FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });
    let ClipboardMessage::SendFormatData(response) = fixture.next() else {
        panic!("expected format data");
    };
    assert!(response.is_error());
}

// oracle: mutant queueing unbounded clipboard format payloads while earlier writes have not completed.
#[test]
fn stalled_outbound_responses_cap_queued_payload_and_recover_after_completion() {
    let mut fixture = Fixture::new();
    fixture.clipboard.offer(Snapshot {
        text: Some("a".repeat(MAX_BYTES / 4 - 1)),
        html: None,
        png: None,
    });
    let _ = fixture.next();

    let request = ironrdp_cliprdr::pdu::FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    };
    fixture.backend.on_format_data_request(request.clone());
    fixture.backend.on_format_data_request(request.clone());
    fixture.backend.on_format_data_request(request.clone());

    let ServerEvent::ClipboardWithCompletion(ClipboardMessage::SendFormatData(first), first_done) =
        fixture.events.try_recv().expect("first response")
    else {
        panic!("expected tracked format data");
    };
    let ServerEvent::ClipboardWithCompletion(ClipboardMessage::SendFormatData(second), _second_done) =
        fixture.events.try_recv().expect("second response")
    else {
        panic!("expected tracked format data");
    };
    let ClipboardMessage::SendFormatData(third) = fixture.next() else {
        panic!("expected format data");
    };
    assert!(!first.is_error());
    assert!(!second.is_error());
    assert!(third.is_error());

    first_done.send(()).expect("complete first write");
    fixture.backend.on_format_data_request(request);
    let ClipboardMessage::SendFormatData(fourth) = fixture.next() else {
        panic!("expected format data after completion");
    };
    assert!(!fourth.is_error());
}

// oracle: mutant queueing unbounded browser clipboard offers while an earlier offer is still waiting on the writer.
#[test]
fn stalled_browser_offers_coalesce_until_previous_offer_completes() {
    let mut fixture = Fixture::new();
    fixture.clipboard.offer(Snapshot {
        text: Some("first".to_owned()),
        ..Snapshot::default()
    });
    fixture.clipboard.offer(Snapshot {
        html: Some("<b>second</b>".to_owned()),
        ..Snapshot::default()
    });
    fixture.clipboard.offer(Snapshot {
        text: Some("latest".to_owned()),
        html: Some("<b>latest</b>".to_owned()),
        png: None,
    });

    let ServerEvent::ClipboardWithValidity {
        message: ClipboardMessage::SendInitiateCopy(first_formats),
        completion: first_done,
        is_current,
    } = fixture.events.try_recv().expect("initial offer")
    else {
        panic!("expected tracked clipboard offer");
    };
    assert_eq!(first_formats.len(), 1);
    assert!(is_current());
    assert!(
        fixture.events.try_recv().is_err(),
        "redundant offer queued behind stalled writer"
    );

    first_done.send(()).expect("complete stalled offer");
    fixture.clipboard.tick();
    let ClipboardMessage::SendInitiateCopy(latest_formats) = fixture.next() else {
        panic!("expected coalesced offer after completion");
    };
    assert_eq!(latest_formats.len(), 2);
}

// oracle: mutant emitting an already-queued or coalesced browser offer after a newer client copy arrives.
#[test]
fn remote_copy_cancels_queued_and_coalesced_browser_offers() {
    let mut fixture = Fixture::new();
    fixture.clipboard.offer(Snapshot {
        text: Some("first".to_owned()),
        ..Snapshot::default()
    });
    fixture.clipboard.offer(Snapshot {
        html: Some("<b>stale browser</b>".to_owned()),
        ..Snapshot::default()
    });
    let ServerEvent::ClipboardWithValidity {
        message: ClipboardMessage::SendInitiateCopy(_),
        completion: first_done,
        is_current: first_is_current,
    } = fixture.events.try_recv().expect("initial offer")
    else {
        panic!("expected tracked clipboard offer");
    };
    assert!(first_is_current());

    fixture
        .backend
        .on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    assert!(
        !first_is_current(),
        "already-queued browser offer remained current after newer client copy"
    );
    assert!(first_done.is_closed());
    assert_eq!(fixture.expect_paste(), ClipboardFormatId::CF_UNICODETEXT);
    fixture
        .backend
        .on_format_data_response(FormatDataResponse::new_unicode_string("client wins"));

    fixture.clipboard.tick();
    assert!(
        fixture.events.try_recv().is_err(),
        "superseded browser offer overwrote the newer client copy"
    );
}
