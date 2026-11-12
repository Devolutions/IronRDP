#![cfg(feature = "runtime")]

use ironrdp_cef_server::clipboard::{Clipboard, Snapshot};
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory as _};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, ClipboardFormatName, FormatDataResponse};
use ironrdp_server::{ServerEvent, ServerEventSender as _};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::sync::watch;

const CF_HTML_COPIED: &[u8] = b"Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000190\r\nStartFragment:0000000141\r\nEndFragment:0000000154\r\n<html>\r\n<body>\r\n<!--StartFragment--><b>copied</b><!--EndFragment-->\r\n</body>\r\n</html>";
const CF_HTML_NEW: &[u8] = b"Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000187\r\nStartFragment:0000000141\r\nEndFragment:0000000151\r\n<html>\r\n<body>\r\n<!--StartFragment--><b>new</b><!--EndFragment-->\r\n</body>\r\n</html>";
const CF_HTML_PAGE: &[u8] = b"Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000188\r\nStartFragment:0000000141\r\nEndFragment:0000000152\r\n<html>\r\n<body>\r\n<!--StartFragment--><i>page</i><!--EndFragment-->\r\n</body>\r\n</html>";

struct Fixture {
    clipboard: Clipboard,
    backend: Box<dyn CliprdrBackend>,
    browser: watch::Receiver<Option<Snapshot>>,
    events: UnboundedReceiver<ServerEvent>,
}

impl Fixture {
    fn new() -> Self {
        let (mut clipboard, browser) = Clipboard::new();
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
            .clone()
    }
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
    // oracle: spec MS-RDPECLIP §3.1.5.2.2
    assert!(!fixture.browser.has_changed().expect("clipboard sender"));

    // oracle: spec MS-RDPECLIP §2.2.3.1
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

// oracle: spec MS-RDPECLIP §2.2.3.1
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

// oracle: spec MS-RDPECLIP §2.2.3.1
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

// oracle: spec MS-RDPECLIP §2.2.3.1
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
