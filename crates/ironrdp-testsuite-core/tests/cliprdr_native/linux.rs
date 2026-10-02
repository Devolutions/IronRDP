use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataResponse};
use ironrdp_cliprdr_format::bitmap;
use ironrdp_cliprdr_native::linux::os::{OsClipboard, PNG, TEXT, Transfer};
use ironrdp_cliprdr_native::linux::worker::{Command, PASTE_TIMEOUT, Worker};

#[derive(Default)]
struct FakeOs {
    data: HashMap<String, Vec<u8>>,
    offers: Vec<(Vec<String>, u64)>,
    reads: usize,
    clears: usize,
    owns: bool,
}

struct FakeClipboard(Arc<Mutex<FakeOs>>);

impl OsClipboard for FakeClipboard {
    fn is_owner(&self) -> bool {
        self.0.lock().unwrap().owns
    }
    fn mime_types(&self) -> Vec<String> {
        self.0.lock().unwrap().data.keys().cloned().collect()
    }
    fn read(&mut self, mime: &str) -> Result<Option<Vec<u8>>, String> {
        let mut os = self.0.lock().unwrap();
        os.reads += 1;
        Ok(os.data.get(mime).cloned())
    }
    fn offer(&mut self, mimes: &[String], generation: u64) -> Result<(), String> {
        self.0.lock().unwrap().offers.push((mimes.to_vec(), generation));
        Ok(())
    }
    fn clear(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().clears += 1;
        Ok(())
    }
}

#[derive(Default, Debug, Clone)]
struct Recorder(Arc<Mutex<Vec<ClipboardMessage>>>);

impl ClipboardMessageProxy for Recorder {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        self.0.lock().unwrap().push(message);
    }
}

impl Recorder {
    fn take(&self) -> Vec<ClipboardMessage> {
        core::mem::take(&mut *self.0.lock().unwrap())
    }
}

type TestWorker = Worker<FakeClipboard, Recorder>;
fn worker() -> (TestWorker, Arc<Mutex<FakeOs>>, Recorder) {
    let os = Arc::new(Mutex::new(FakeOs::default()));
    let recorder = Recorder::default();
    (
        Worker::new(FakeClipboard(Arc::clone(&os)), recorder.clone()),
        os,
        recorder,
    )
}

fn remote(worker: &mut TestWorker, formats: &[ClipboardFormatId]) {
    worker.handle(Command::RemoteCopy(
        formats.iter().copied().map(ClipboardFormat::new).collect(),
    ));
}

fn paste(worker: &mut TestWorker, os: &Arc<Mutex<FakeOs>>, mime: &str) -> mpsc::Receiver<Option<Vec<u8>>> {
    let generation = os.lock().unwrap().offers.last().unwrap().1;
    let (sender, receiver) = mpsc::channel();
    worker.handle(Command::Paste(Transfer::new(
        mime.to_owned(),
        generation,
        move |data| {
            sender.send(data).unwrap();
        },
    )));
    receiver
}

fn text_response(worker: &mut TestWorker, text: &str) {
    worker.handle(Command::RemoteData(Some(
        FormatDataResponse::new_unicode_string(text).data().to_vec(),
    )));
}

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0x22, 0x44, 0x66, 0xff])
            .unwrap();
    }
    bytes
}

#[test]
fn selection_events_wait_for_monitor_ready_and_do_not_read_content() {
    let (mut worker, os, recorder) = worker();
    os.lock().unwrap().data.insert(TEXT.into(), b"hello".to_vec());
    worker.handle(Command::LocalChanged);
    assert!(recorder.take().is_empty());
    worker.handle(Command::AdvertiseLocal);
    assert!(
        matches!(recorder.take().as_slice(), [ClipboardMessage::SendInitiateCopy(formats)] if formats.len() == 1 && formats[0].id() == ClipboardFormatId::CF_UNICODETEXT)
    );
    assert_eq!(os.lock().unwrap().reads, 0);
}

#[test]
fn a_new_local_copy_with_the_same_formats_is_announced() {
    let (mut worker, _, recorder) = worker();
    worker.handle(Command::AdvertiseLocal);
    recorder.take();
    worker.handle(Command::LocalChanged);
    worker.handle(Command::LocalChanged);
    assert_eq!(recorder.take().len(), 2);
}

#[test]
fn remote_formats_are_advertised_without_fetching_data() {
    let (mut worker, os, recorder) = worker();
    remote(
        &mut worker,
        &[ClipboardFormatId::CF_UNICODETEXT, ClipboardFormatId::CF_DIBV5],
    );
    assert!(recorder.take().is_empty());
    assert_eq!(os.lock().unwrap().offers[0].0, [TEXT, "text/plain", PNG]);
    let response = paste(&mut worker, &os, TEXT);
    assert!(matches!(
        recorder.take().as_slice(),
        [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT)]
    ));
    text_response(&mut worker, "only when pasted");
    assert_eq!(response.recv().unwrap(), Some(b"only when pasted".to_vec()));
}

#[test]
fn same_format_waiters_share_one_wire_request_and_cache() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let first = paste(&mut worker, &os, TEXT);
    let second = paste(&mut worker, &os, "text/plain");
    assert_eq!(recorder.take().len(), 1);
    text_response(&mut worker, "shared");
    assert_eq!(first.recv().unwrap(), Some(b"shared".to_vec()));
    assert_eq!(second.recv().unwrap(), Some(b"shared".to_vec()));
    let cached = paste(&mut worker, &os, TEXT);
    assert_eq!(cached.recv().unwrap(), Some(b"shared".to_vec()));
    assert!(recorder.take().is_empty());
}

#[test]
fn different_formats_are_requested_one_at_a_time() {
    let (mut worker, os, recorder) = worker();
    remote(
        &mut worker,
        &[
            ClipboardFormatId::CF_UNICODETEXT,
            ClipboardFormatId::CF_DIB,
            ClipboardFormatId::CF_DIBV5,
        ],
    );
    let text = paste(&mut worker, &os, TEXT);
    let image = paste(&mut worker, &os, PNG);
    assert_eq!(recorder.take().len(), 1);
    text_response(&mut worker, "text");
    assert_eq!(text.recv().unwrap(), Some(b"text".to_vec()));
    assert!(matches!(
        recorder.take().as_slice(),
        [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_DIBV5)]
    ));
    worker.handle(Command::RemoteData(Some(bitmap::png_to_cf_dibv5(&png()).unwrap())));
    let decoded = image.recv().unwrap().unwrap();
    assert_eq!(
        bitmap::png_to_cf_dibv5(&decoded).unwrap(),
        bitmap::png_to_cf_dibv5(&png()).unwrap()
    );
}

#[test]
fn a_timed_out_request_is_drained_before_another_can_start() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let old = paste(&mut worker, &os, TEXT);
    recorder.take();
    worker.expire(Instant::now() + PASTE_TIMEOUT);
    assert_eq!(old.recv().unwrap(), None);
    remote(&mut worker, &[ClipboardFormatId::CF_DIB]);
    let blocked = paste(&mut worker, &os, PNG);
    assert_eq!(blocked.recv().unwrap(), None);
    assert!(
        recorder.take().is_empty(),
        "timeout must not start an ambiguous second request"
    );
    text_response(&mut worker, "late old response");
    let current = paste(&mut worker, &os, PNG);
    assert!(matches!(
        recorder.take().as_slice(),
        [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_DIB)]
    ));
    worker.handle(Command::RemoteData(Some(bitmap::png_to_cf_dib(&png()).unwrap())));
    assert!(current.recv().unwrap().is_some());
}

#[test]
fn replaced_remote_selection_discards_the_old_response() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let old = paste(&mut worker, &os, TEXT);
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    assert_eq!(old.recv().unwrap(), None);
    let new = paste(&mut worker, &os, TEXT);
    assert_eq!(recorder.take().len(), 1);
    text_response(&mut worker, "old");
    assert_eq!(
        recorder.take().len(),
        1,
        "new request starts only after old reply is drained"
    );
    text_response(&mut worker, "new");
    assert_eq!(new.recv().unwrap(), Some(b"new".to_vec()));
}

#[test]
fn local_selection_change_cancels_waiters_and_discards_remote_data() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let response = paste(&mut worker, &os, TEXT);
    worker.handle(Command::LocalChanged);
    assert_eq!(response.recv().unwrap(), None);
    recorder.take();
    text_response(&mut worker, "stale");
    assert!(recorder.take().is_empty());
    assert_eq!(
        os.lock().unwrap().offers.len(),
        1,
        "a response cannot reclaim the clipboard"
    );
}

#[test]
fn stale_os_paste_and_unadvertised_formats_are_refused() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let generation = os.lock().unwrap().offers[0].1;
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let (sender, receiver) = mpsc::channel();
    worker.handle(Command::Paste(Transfer::new(TEXT.into(), generation, move |data| {
        sender.send(data).unwrap();
    })));
    assert_eq!(receiver.recv().unwrap(), None);
    assert_eq!(paste(&mut worker, &os, PNG).recv().unwrap(), None);
    assert!(recorder.take().is_empty());
}

#[test]
fn failed_remote_response_closes_the_os_paste() {
    let (mut worker, os, _) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let response = paste(&mut worker, &os, TEXT);
    worker.handle(Command::RemoteData(None));
    assert_eq!(response.recv().unwrap(), None);
}

#[test]
fn pending_os_pastes_are_bounded() {
    let (mut worker, os, _) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let waiters: Vec<_> = core::iter::repeat_with(|| paste(&mut worker, &os, TEXT))
        .take(16)
        .collect();
    assert_eq!(paste(&mut worker, &os, TEXT).recv().unwrap(), None);
    text_response(&mut worker, "bounded");
    for waiter in waiters {
        assert_eq!(waiter.recv().unwrap(), Some(b"bounded".to_vec()));
    }
}

#[test]
fn image_dimensions_are_checked_by_the_shared_bitmap_converter() {
    let (mut worker, os, _) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_DIB]);
    let response = paste(&mut worker, &os, PNG);
    let mut header = vec![0u8; 40];
    header[0..4].copy_from_slice(&40u32.to_le_bytes());
    header[4..8].copy_from_slice(&100_000i32.to_le_bytes());
    header[8..12].copy_from_slice(&100_000i32.to_le_bytes());
    header[12..14].copy_from_slice(&1u16.to_le_bytes());
    header[14..16].copy_from_slice(&32u16.to_le_bytes());
    worker.handle(Command::RemoteData(Some(header)));
    assert_eq!(response.recv().unwrap(), None);
}

#[test]
fn local_content_is_read_only_when_the_peer_requests_it() {
    let (mut worker, os, recorder) = worker();
    os.lock()
        .unwrap()
        .data
        .insert(TEXT.into(), "local \u{03bb}".as_bytes().to_vec());
    os.lock().unwrap().data.insert(PNG.into(), png());
    worker.handle(Command::AdvertiseLocal);
    recorder.take();
    assert_eq!(os.lock().unwrap().reads, 0);
    worker.handle(Command::RenderLocal(ClipboardFormatId::CF_UNICODETEXT));
    assert!(
        matches!(recorder.take().as_slice(), [ClipboardMessage::SendFormatData(response)] if response.to_unicode_string().unwrap() == "local \u{03bb}")
    );
    worker.handle(Command::RenderLocal(ClipboardFormatId::CF_DIBV5));
    assert!(
        matches!(recorder.take().as_slice(), [ClipboardMessage::SendFormatData(response)] if bitmap::validate_dibv5(response.data()).is_ok())
    );
    assert_eq!(os.lock().unwrap().reads, 2);
}

#[test]
fn shared_loop_detector_rejects_a_clipboard_manager_echo() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let response = paste(&mut worker, &os, TEXT);
    text_response(&mut worker, "echoed");
    response.recv().unwrap();
    os.lock().unwrap().data.insert(TEXT.into(), b"echoed".to_vec());
    worker.handle(Command::LocalChanged);
    recorder.take();
    worker.handle(Command::RenderLocal(ClipboardFormatId::CF_UNICODETEXT));
    assert!(matches!(recorder.take().as_slice(), [ClipboardMessage::SendFormatData(response)] if response.is_error()));
    os.lock().unwrap().data.insert(TEXT.into(), b"different".to_vec());
    worker.handle(Command::RenderLocal(ClipboardFormatId::CF_UNICODETEXT));
    assert!(matches!(recorder.take().as_slice(), [ClipboardMessage::SendFormatData(response)] if !response.is_error()));
}

#[test]
fn channel_reset_cancels_pending_pastes_and_releases_ownership() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    let response = paste(&mut worker, &os, TEXT);
    worker.handle(Command::Reset);
    assert_eq!(response.recv().unwrap(), None);
    assert_eq!(os.lock().unwrap().clears, 1);
    recorder.take();
    worker.handle(Command::LocalChanged);
    assert!(recorder.take().is_empty());
}

/// Run against a private X server: DISPLAY=:N cargo test -p ironrdp-testsuite-core
/// x11_delayed_pastes -- --ignored --test-threads=1.
#[test]
#[ignore = "requires a dedicated X11 display"]
fn x11_delayed_pastes_use_notifications_and_incremental_transfers() {
    use core::time::Duration;

    use ironrdp_cliprdr_native::linux::x11::X11Clipboard;

    let (owner_events, owner_receiver) = mpsc::channel();
    let mut owner = X11Clipboard::open(owner_events).unwrap();
    let (reader_events, reader_receiver) = mpsc::channel();
    let reader = X11Clipboard::open(reader_events).unwrap();
    owner.offer(&[TEXT.into(), PNG.into()], 42).unwrap();
    loop {
        if let Command::LocalChanged = reader_receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            let mimes = reader.mime_types();
            if mimes.contains(&TEXT.to_owned()) && mimes.contains(&PNG.to_owned()) {
                break;
            }
        }
    }
    assert!(
        !owner_receiver
            .try_iter()
            .any(|event| matches!(event, Command::Paste(_)))
    );
    let thread = std::thread::spawn(move || {
        let mut reader = reader;
        let result = reader.read(TEXT);
        (reader, result)
    });
    let transfer = loop {
        if let Command::Paste(transfer) = owner_receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            break transfer;
        }
    };
    assert_eq!(transfer.generation(), 42);
    // Exceeds the core X11 request length and must use INCR in both directions.
    let bytes = "clipboard \u{03bb}\n".repeat(30_000).into_bytes();
    transfer.finish(Some(bytes.clone()));
    let (mut reader, received) = thread.join().unwrap();
    assert_eq!(received.unwrap(), Some(bytes));

    // Becoming the owner must not report our own selection back as a local copy.
    assert!(
        !owner_receiver
            .try_iter()
            .any(|event| matches!(event, Command::LocalChanged))
    );
    reader.offer(&[TEXT.into()], 43).unwrap();
    loop {
        if let Command::LocalChanged = owner_receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            if owner.mime_types().contains(&TEXT.to_owned()) {
                break;
            }
        }
    }
    assert_eq!(owner.mime_types(), [TEXT]);
}

#[test]
#[ignore = "requires a dedicated X11 display and xclip"]
fn x11_clipboard_interoperates_with_xclip_in_both_directions() {
    use core::time::Duration;
    use std::io::Write as _;
    use std::process::{Command as ProcessCommand, Stdio};

    use ironrdp_cliprdr_native::linux::x11::X11Clipboard;

    let (events, receiver) = mpsc::channel();
    let mut clipboard = X11Clipboard::open(events).unwrap();
    clipboard.offer(&[TEXT.into()], 1).unwrap();
    // The read is processed after the ownership command on the X11 worker.
    // Our own selection is not read back, but this waits for the claim to finish.
    assert!(clipboard.read(TEXT).unwrap().is_none());
    let reader = std::thread::spawn(|| {
        ProcessCommand::new("xclip")
            .args(["-selection", "clipboard", "-out"])
            .output()
            .unwrap()
    });
    let transfer = loop {
        if let Command::Paste(transfer) = receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            break transfer;
        }
    };
    let data = "remote unicode \u{03bb}\n".repeat(20_000).into_bytes();
    transfer.finish(Some(data.clone()));
    // ICCCM requires an already-started INCR transfer to finish even after
    // the owner relinquishes the selection.
    clipboard.clear().unwrap();
    let output = reader.join().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, data);

    let mut writer = ProcessCommand::new("xclip")
        .args(["-selection", "clipboard", "-in", "-quiet"])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = writer.stdin.take().unwrap();
    stdin.write_all(&data).unwrap();
    drop(stdin);
    loop {
        if let Command::LocalChanged = receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            if clipboard.mime_types().contains(&TEXT.to_owned()) {
                break;
            }
        }
    }
    let result = clipboard.read(TEXT);
    let _ = writer.kill();
    let _ = writer.wait();
    assert_eq!(result.unwrap(), Some(data));
}

#[test]
fn queued_local_event_does_not_cancel_an_acknowledged_remote_selection() {
    let (mut worker, os, recorder) = worker();
    remote(&mut worker, &[ClipboardFormatId::CF_UNICODETEXT]);
    os.lock().unwrap().owns = true;
    worker.handle(Command::LocalChanged);
    let response = paste(&mut worker, &os, TEXT);
    assert!(matches!(
        recorder.take().as_slice(),
        [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT)]
    ));
    text_response(&mut worker, "current remote");
    assert_eq!(response.recv().unwrap(), Some(b"current remote".to_vec()));
}

#[test]
fn local_notifications_refresh_the_current_selection_formats() {
    let (mut worker, os, recorder) = worker();
    worker.handle(Command::AdvertiseLocal);
    recorder.take();
    os.lock()
        .unwrap()
        .data
        .insert(TEXT.into(), b"newest selection".to_vec());
    worker.handle(Command::LocalChanged);
    assert!(
        matches!(recorder.take().as_slice(), [ClipboardMessage::SendInitiateCopy(formats)] if formats.len() == 1 && formats[0].id() == ClipboardFormatId::CF_UNICODETEXT)
    );
}

#[test]
#[ignore = "requires a dedicated X11 display"]
fn x11_bounds_requests_before_queuing_clipboard_data() {
    use core::time::Duration;
    use ironrdp_cliprdr_native::linux::x11::X11Clipboard;

    let (owner_events, owner_receiver) = mpsc::channel();
    let mut owner = X11Clipboard::open(owner_events).unwrap();
    owner.offer(&[TEXT.into()], 1).unwrap();
    let (results, received) = mpsc::channel();
    let readers: Vec<_> = core::iter::repeat_with(|| {
        let results = results.clone();
        std::thread::spawn(move || {
            let (events, notifications) = mpsc::channel();
            let mut reader = X11Clipboard::open(events).unwrap();
            loop {
                if let Command::LocalChanged = notifications.recv_timeout(Duration::from_secs(5)).unwrap() {
                    if reader.mime_types().contains(&TEXT.to_owned()) {
                        break;
                    }
                }
            }
            results.send(reader.read(TEXT)).unwrap();
        })
    })
    .take(17)
    .collect();
    let mut held = Vec::new();
    while held.len() < 16 {
        if let Command::Paste(transfer) = owner_receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            held.push(transfer);
        }
    }
    assert!(
        received.recv_timeout(Duration::from_secs(2)).unwrap().is_err(),
        "the seventeenth read is refused before a Paste is queued"
    );
    assert!(
        !owner_receiver
            .try_iter()
            .any(|event| matches!(event, Command::Paste(_)))
    );
    for transfer in held {
        transfer.finish(None);
    }
    for _ in 0..16 {
        assert!(received.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
    }
    for reader in readers {
        reader.join().unwrap();
    }
}
