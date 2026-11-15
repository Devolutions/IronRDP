use core::time::Duration;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::pdu::{FileContentsFlags, FileContentsRequest};
use ironrdp_cliprdr_native::macos_test_support::ClipboardHarness;
use ironrdp_viewer::app::test_clipboard_deadline;

const FILE_LIMIT: usize = 1000;

#[derive(Debug)]
struct TestProxy(Sender<ClipboardMessage>);

impl ClipboardMessageProxy for TestProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        self.0.send(message).expect("record clipboard message");
    }
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("macos-clipboard-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn clipboard() -> (ClipboardHarness, Receiver<ClipboardMessage>) {
    let (messages_tx, messages) = mpsc::channel();
    (ClipboardHarness::new(TestProxy(messages_tx)), messages)
}

#[test]
fn idle_clipboard_keeps_a_slow_poll_and_active_work_uses_a_short_poll() {
    // oracle: mutant stopping idle clipboard polls.
    let (mut clipboard, _) = clipboard();
    let now = Instant::now();
    assert!(!clipboard.needs_poll());
    let idle = test_clipboard_deadline(now, clipboard.needs_poll(), None);
    assert!(now + Duration::from_millis(10) < idle);
    assert!(idle <= now + Duration::from_secs(1));

    clipboard.replace_offer(&PathBuf::from("placeholder"), 3);
    assert!(clipboard.needs_poll());
    let active = test_clipboard_deadline(now, clipboard.needs_poll(), None);
    assert_eq!(active, now + Duration::from_millis(10));

    let earlier = now + Duration::from_millis(1);
    assert_eq!(test_clipboard_deadline(now, true, Some(earlier)), earlier);
    assert_eq!(test_clipboard_deadline(now, false, Some(earlier)), earlier);
}

#[test]
fn oversized_folder_is_not_silently_truncated() {
    // oracle: mutant silently truncating oversized folder offers.
    let root = TestDir::new("oversized");
    for index in 0..FILE_LIMIT {
        fs::write(root.0.join(index.to_string()), []).expect("create test file");
    }
    let (mut clipboard, messages) = clipboard();
    assert!(clipboard.advertise_files(core::slice::from_ref(&root.0)));
    assert!(matches!(
        messages.try_recv().expect("error message"),
        ClipboardMessage::Error(_)
    ));
    assert!(matches!(
        messages.try_recv().expect("empty format list"),
        ClipboardMessage::SendInitiateCopy(formats) if formats.is_empty()
    ));
}

#[test]
fn folder_at_entry_limit_is_advertised_completely() {
    // oracle: mutant off-by-one error at the entry limit.
    let root = TestDir::new("limit");
    for index in 0..FILE_LIMIT - 1 {
        fs::write(root.0.join(index.to_string()), []).expect("create test file");
    }
    let (mut clipboard, messages) = clipboard();
    assert!(clipboard.advertise_files(core::slice::from_ref(&root.0)));
    assert!(matches!(
        messages.try_recv().expect("file offer"),
        ClipboardMessage::SendInitiateFileCopy(files) if files.len() == FILE_LIMIT
    ));
}

#[test]
fn unknown_lock_does_not_read_current_local_files() {
    // oracle: MS-RDPECLIP 3.1.5.3.2; a lock identifies the retained file stream data.
    let root = TestDir::new("unknown-lock");
    let path = root.0.join("file");
    fs::write(&path, b"current").expect("create file");
    let (mut clipboard, messages) = clipboard();
    assert!(clipboard.advertise_files(&[path]));
    let _ = messages.try_recv().expect("file offer");
    clipboard.request_local_file(FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: 0,
        requested_size: 7,
        data_id: Some(123),
    });
    assert!(matches!(
        messages.try_recv().expect("file response"),
        ClipboardMessage::SendFileContentsResponse(response) if response.is_error()
    ));
}

#[test]
fn stale_response_from_previous_offer_does_not_complete_new_download() {
    // oracle: mutant reusing a stream ID for a new offer.
    let root = TestDir::new("stale");
    let old_path = root.0.join("old");
    let new_path = root.0.join("new");
    fs::write(&old_path, [0; 3]).expect("create old placeholder");
    fs::write(&new_path, [0; 3]).expect("create new placeholder");

    let (mut clipboard, messages) = clipboard();
    clipboard.replace_offer(&old_path, 3);
    let _old_result = clipboard.request_read(&old_path);
    let ClipboardMessage::SendFileContentsRequest(old_request) = messages.try_recv().expect("old request") else {
        panic!("expected old file request");
    };

    clipboard.replace_offer(&new_path, 3);
    let result = clipboard.request_read(&new_path);
    let ClipboardMessage::SendFileContentsRequest(new_request) = messages.try_recv().expect("new request") else {
        panic!("expected new file request");
    };
    clipboard.receive_contents(old_request.stream_id, b"old");
    assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
    clipboard.receive_contents(new_request.stream_id, b"new");
    assert!(result.try_recv().expect("download result").is_ok());
    assert_eq!(fs::read(&new_path).expect("read placeholder"), b"new");
}

#[test]
fn exhausted_stream_ids_fail_instead_of_being_reused() {
    // oracle: mutant wrapping the stream ID counter.
    let root = TestDir::new("stream-limit");
    let old_path = root.0.join("old");
    let new_path = root.0.join("new");
    fs::write(&old_path, [0; 3]).expect("create old placeholder");
    fs::write(&new_path, [0; 3]).expect("create new placeholder");

    let (mut clipboard, messages) = clipboard();
    clipboard.set_next_stream_id(u32::MAX);
    clipboard.replace_offer(&old_path, 3);
    let _old_result = clipboard.request_read(&old_path);
    assert!(matches!(
        messages.try_recv().expect("last request"),
        ClipboardMessage::SendFileContentsRequest(request) if request.stream_id == u32::MAX
    ));

    clipboard.replace_offer(&new_path, 3);
    let result = clipboard.request_read(&new_path);
    assert!(result.try_recv().expect("download result").is_err());
    assert!(matches!(messages.try_recv(), Err(mpsc::TryRecvError::Empty)));
}
