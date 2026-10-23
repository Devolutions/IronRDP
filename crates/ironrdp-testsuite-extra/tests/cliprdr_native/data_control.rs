//! Tests for the Wayland data-control clipboard client that need no compositor.

use core::time::Duration;
use std::{
    collections::HashSet,
    io::{Read as _, Write as _},
    os::fd::OwnedFd,
    sync::{Arc, Mutex, PoisonError, mpsc},
    time::Instant,
};

use ironrdp_cliprdr_native::data_control::state::{
    Command, MAX_PASTES, OwnMarker, Shared, State, write_with_idle_timeout,
};
use ironrdp_cliprdr_native::data_control::{DataControl, Error, Options, Preference, Protocol, find_mime_match};

fn mimes(types: &[&str]) -> Vec<String> {
    types.iter().map(|t| (*t).to_owned()).collect()
}

fn read_all(fd: OwnedFd) -> Vec<u8> {
    let mut buf = Vec::new();
    std::fs::File::from(fd).read_to_end(&mut buf).unwrap();
    buf
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let (reader, writer) = std::io::pipe().unwrap();
    (OwnedFd::from(reader), OwnedFd::from(writer))
}

#[test]
fn mime_exact_match_is_returned() {
    let available = mimes(&["text/plain", "text/html"]);
    assert_eq!(find_mime_match("text/html", &available), Some("text/html"));
}

#[test]
fn mime_requested_charset_is_stripped() {
    let available = mimes(&["text/plain"]);
    assert_eq!(
        find_mime_match("text/plain;charset=utf-8", &available),
        Some("text/plain")
    );
}

#[test]
fn mime_missing_charset_is_added() {
    let available = mimes(&["text/plain;charset=utf-8"]);
    assert_eq!(
        find_mime_match("text/plain", &available),
        Some("text/plain;charset=utf-8")
    );
}

#[test]
fn mime_non_text_types_get_no_charset_fallback() {
    let available = mimes(&["image/png;foo=bar"]);
    assert_eq!(find_mime_match("image/png", &available), None);
}

#[test]
fn mime_exact_match_beats_a_charset_variant() {
    let available = mimes(&["text/plain;charset=utf-8", "text/plain"]);
    assert_eq!(find_mime_match("text/plain", &available), Some("text/plain"));
}

#[test]
fn mime_any_charset_of_the_same_base_type_is_accepted() {
    let available = mimes(&["text/plain;charset=iso-8859-1"]);
    assert_eq!(
        find_mime_match("text/plain", &available),
        Some("text/plain;charset=iso-8859-1")
    );
}

#[test]
fn the_type_a_read_will_use_is_the_one_find_mime_match_reports() {
    // A source that offers only another charset is matched, and a caller that needs to know
    // which charset it will get asks `find_mime_match` first.
    let available = mimes(&["text/plain;charset=iso-8859-1", "image/png"]);
    assert_eq!(
        find_mime_match("text/plain;charset=utf-8", &available),
        Some("text/plain;charset=iso-8859-1")
    );
}

#[test]
fn auto_preference_tries_ext_then_wlr() {
    assert_eq!(Options::new().candidates(), [Protocol::Ext, Protocol::Wlr]);
}

#[test]
fn wlr_preference_tries_wlr_first() {
    let options = Options::new().preference(Preference::Wlr);
    assert_eq!(options.candidates(), [Protocol::Wlr, Protocol::Ext]);
}

#[test]
fn disabling_fallback_leaves_only_the_preferred_protocol() {
    let options = Options::new().preference(Preference::Wlr).allow_fallback(false);
    assert_eq!(options.candidates(), [Protocol::Wlr]);
}

#[test]
fn a_selection_that_carries_our_marker_is_ours_and_loses_the_marker() {
    let marker = OwnMarker::random();
    let mut types = mimes(&["text/plain;charset=utf-8", marker.as_str()]);

    assert!(marker.take_from(&mut types));

    assert_eq!(types, mimes(&["text/plain;charset=utf-8"]));
}

#[test]
fn a_copy_by_another_client_is_foreign_whatever_types_it_offers() {
    // The same types as ours, offered in a different order, still don't make it ours.
    let marker = OwnMarker::random();
    let mut types = mimes(&["text/plain", "text/plain;charset=utf-8"]);

    assert!(!marker.take_from(&mut types));

    assert_eq!(types, mimes(&["text/plain", "text/plain;charset=utf-8"]));
}

#[test]
fn the_marker_of_another_client_is_removed_but_is_not_ours() {
    let ours = OwnMarker::random();
    let theirs = OwnMarker::random();
    assert_ne!(ours.as_str(), theirs.as_str());
    let mut types = mimes(&["image/png", theirs.as_str()]);

    assert!(!ours.take_from(&mut types));

    assert_eq!(types, mimes(&["image/png"]));
}

#[test]
fn a_clone_of_our_content_by_a_clipboard_manager_is_ours() {
    // A manager that re-offers our selection carries every type along, the marker included.
    let marker = OwnMarker::random();
    let mut types = mimes(&["text/html", "text/plain", marker.as_str()]);

    assert!(marker.take_from(&mut types));

    assert_eq!(types, mimes(&["text/html", "text/plain"]));
}

#[test]
fn markers_made_in_one_process_are_all_different() {
    let markers: HashSet<String> = core::iter::repeat_with(|| OwnMarker::random().as_str().to_owned())
        .take(1000)
        .collect();

    assert_eq!(markers.len(), 1000);
}

#[test]
fn only_private_markers_are_recognised_as_markers() {
    assert!(OwnMarker::is_marker(OwnMarker::random().as_str()));
    assert!(!OwnMarker::is_marker("text/plain"));
    assert!(!OwnMarker::is_marker("application/x-kde-passwordManagerHint"));
}

#[test]
fn clearing_the_selection_notifies_and_advances_the_serial() {
    let mut state = State::default();
    let called = Arc::new(Mutex::new(false));
    let seen = Arc::clone(&called);
    state.shared().lock().unwrap().on_change = Some(Arc::new(move |types: Vec<String>| {
        assert!(types.is_empty());
        *seen.lock().unwrap() = true;
    }));

    state.on_selection_cleared();

    assert!(*called.lock().unwrap());
    let shared = state.shared().lock().unwrap();
    assert!(shared.mime_types.is_empty());
    assert_eq!(shared.serial, 1);
}

#[test]
fn cancelling_our_source_drops_its_data() {
    let mut state = State::default();
    state.insert_source_data("text/plain", b"hello".to_vec());
    state.on_source_cancelled();
    assert!(!state.has_source_data());
}

#[test]
fn device_finished_drops_source_data() {
    let mut state = State::default();
    state.insert_source_data("text/plain", b"hello".to_vec());
    state.on_device_finished();
    assert!(!state.has_source_data());
}

#[test]
fn clearing_the_selection_ends_ownership() {
    let mut state = State::default();
    state.shared().lock().unwrap().own_source_live = true;
    state.on_selection_cleared();
    assert!(!state.shared().lock().unwrap().own_source_live);
}

#[test]
fn cancelling_our_source_ends_ownership() {
    let mut state = State::default();
    state.shared().lock().unwrap().own_source_live = true;
    state.on_source_cancelled();
    assert!(!state.shared().lock().unwrap().own_source_live);
}

#[test]
fn device_finished_ends_ownership() {
    let mut state = State::default();
    state.shared().lock().unwrap().own_source_live = true;
    state.on_device_finished();
    assert!(!state.shared().lock().unwrap().own_source_live);
}

#[test]
fn a_mime_type_without_a_pending_offer_is_ignored() {
    let mut state = State::default();
    state.on_offer_mime_type("text/plain".to_owned());
}

#[test]
fn a_paste_of_an_unknown_type_closes_the_pipe_with_no_data() {
    let mut state = State::default();
    let (read, write) = pipe();
    state.on_source_send("text/unknown", write);
    assert!(read_all(read).is_empty());
}

#[test]
fn a_paste_is_answered_from_cached_data() {
    let mut state = State::default();
    state.insert_source_data("text/plain", b"hello world".to_vec());
    let (read, write) = pipe();
    state.on_source_send("text/plain", write);
    assert_eq!(read_all(read), b"hello world");
}

#[test]
fn data_added_after_the_selection_is_served_to_the_next_paste() {
    let mut state = State::default();
    state.update_source_data("text/plain".to_owned(), b"hello".to_vec());
    let (read, write) = pipe();
    state.on_source_send("text/plain", write);
    assert_eq!(read_all(read), b"hello");
}

#[test]
fn data_added_again_for_a_type_replaces_the_earlier_data() {
    let mut state = State::default();
    state.update_source_data("text/plain".to_owned(), b"first".to_vec());
    state.update_source_data("text/plain".to_owned(), b"second".to_vec());
    let (read, write) = pipe();
    state.on_source_send("text/plain", write);
    assert_eq!(read_all(read), b"second");
}

#[test]
fn update_data_sends_the_data_to_the_worker() {
    let (clipboard, _shared, commands) = clipboard_offering(&["text/plain"]);

    clipboard.update_data("text/plain", b"hello".to_vec()).unwrap();

    let Command::UpdateSourceData { mime_type, data } = commands.try_recv().unwrap() else {
        unreachable!("update_data sends UpdateSourceData");
    };
    assert_eq!(mime_type, "text/plain");
    assert_eq!(data, b"hello");
}

#[test]
fn update_data_reports_a_stopped_worker() {
    let (clipboard, _shared, commands) = clipboard_offering(&["text/plain"]);
    drop(commands);

    let result = clipboard.update_data("text/plain", b"hello".to_vec());

    assert!(matches!(result, Err(Error::Stopped)));
}

#[test]
fn a_paste_of_the_bare_type_is_answered_from_the_utf8_entry() {
    let mut state = State::default();
    state.insert_source_data("text/plain;charset=utf-8", b"hello".to_vec());
    let (read, write) = pipe();
    state.on_source_send("text/plain", write);
    assert_eq!(read_all(read), b"hello");
}

#[test]
fn a_paste_with_a_charset_is_answered_from_the_bare_entry() {
    let mut state = State::default();
    state.insert_source_data("text/plain", b"hello".to_vec());
    let (read, write) = pipe();
    state.on_source_send("text/plain;charset=utf-8", write);
    assert_eq!(read_all(read), b"hello");
}

#[test]
fn a_paste_is_not_answered_with_bytes_in_another_charset() {
    let mut state = State::default();
    state.insert_source_data("text/plain;charset=iso-8859-1", b"\xe9t\xe9".to_vec());
    for requested in ["text/plain;charset=utf-8", "text/plain"] {
        let (read, write) = pipe();
        state.on_source_send(requested, write);
        assert!(read_all(read).is_empty(), "{requested} got bytes in another charset");
    }
}

type TransferLog = Arc<Mutex<Vec<(u32, String)>>>;

/// State advertising `image/png` with a transfer callback that records what it was asked for.
fn state_with_transfer_callback() -> (State, TransferLog) {
    let mut state = State::default();
    state.set_advertised(mimes(&["image/png"]));
    let requests: TransferLog = Arc::default();
    let seen = Arc::clone(&requests);
    state.shared().lock().unwrap().on_transfer = Some(Arc::new(move |serial, mime| {
        seen.lock().unwrap().push((serial, mime));
    }));
    (state, requests)
}

#[test]
fn a_paste_without_data_is_held_until_the_transfer_completes() {
    let (mut state, requests) = state_with_transfer_callback();
    let (first_read, first_write) = pipe();
    let (second_read, second_write) = pipe();

    state.on_source_send("image/png", first_write);
    state.on_source_send("image/png", second_write);
    // Both pastes share one transfer.
    assert_eq!(*requests.lock().unwrap(), vec![(1, "image/png".to_owned())]);

    state.complete_transfer(1, Some(b"png bytes".to_vec()));
    assert_eq!(read_all(first_read), b"png bytes");
    assert_eq!(read_all(second_read), b"png bytes");

    // A later paste is served from the cache without a new transfer.
    let (third_read, third_write) = pipe();
    state.on_source_send("image/png", third_write);
    assert_eq!(read_all(third_read), b"png bytes");
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn a_held_paste_gets_nothing_when_the_selection_is_lost() {
    let (mut state, _requests) = state_with_transfer_callback();
    let (read, write) = pipe();
    state.on_source_send("image/png", write);

    state.on_source_cancelled();
    assert!(read_all(read).is_empty());
    // An answer for the lost selection is ignored.
    state.complete_transfer(1, Some(b"late".to_vec()));
    assert!(!state.has_source_data());
}

#[test]
fn a_failed_transfer_lets_the_next_paste_raise_a_new_request() {
    let (mut state, requests) = state_with_transfer_callback();
    let (first_read, first_write) = pipe();
    state.on_source_send("image/png", first_write);

    state.complete_transfer(1, None);
    assert!(read_all(first_read).is_empty());

    let (_second_read, second_write) = pipe();
    state.on_source_send("image/png", second_write);
    assert_eq!(
        *requests.lock().unwrap(),
        vec![(1, "image/png".to_owned()), (2, "image/png".to_owned())]
    );
}

#[test]
fn a_paste_of_the_marker_closes_the_pipe_and_raises_no_transfer() {
    let (mut state, requests) = state_with_transfer_callback();
    let marker = state.own_marker().to_owned();
    let (read, write) = pipe();

    state.on_source_send(&marker, write);

    assert!(read_all(read).is_empty());
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn a_paste_of_an_unadvertised_type_raises_no_transfer() {
    let (mut state, requests) = state_with_transfer_callback();
    let (read, write) = pipe();
    state.on_source_send("text/html", write);
    assert!(read_all(read).is_empty());
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn pastes_beyond_the_limit_are_closed_without_data() {
    let (mut state, requests) = state_with_transfer_callback();
    let mut held = Vec::new();
    for _ in 0..MAX_PASTES {
        let (read, write) = pipe();
        state.on_source_send("image/png", write);
        held.push(read);
    }
    // The held pastes share one transfer.
    assert_eq!(requests.lock().unwrap().len(), 1);

    // One more is closed at once, so reading it ends without an answer.
    let (read, write) = pipe();
    state.on_source_send("image/png", write);
    assert!(read_all(read).is_empty());

    state.complete_transfer(1, Some(b"png bytes".to_vec()));
    for read in held {
        assert_eq!(read_all(read), b"png bytes");
    }
}

#[test]
fn a_finished_paste_gives_its_place_back() {
    let mut state = State::default();
    state.insert_source_data("text/plain", b"hello".to_vec());
    // Three times as many pastes as places, one after the other.
    for _ in 0..MAX_PASTES * 3 {
        let (read, write) = pipe();
        state.on_source_send("text/plain", write);
        assert_eq!(read_all(read), b"hello");
    }
}

#[test]
fn a_held_paste_that_waits_too_long_gets_no_data() {
    let (mut state, _requests) = state_with_transfer_callback();
    assert_eq!(state.expire_transfers(Instant::now()), None);

    let (read, write) = pipe();
    state.on_source_send("image/png", write);
    let due = state
        .expire_transfers(Instant::now())
        .expect("a held paste has a deadline");

    // Just before the deadline the paste is still held. At the deadline it is closed.
    assert_eq!(state.expire_transfers(due - Duration::from_millis(1)), Some(due));
    assert_eq!(state.expire_transfers(due), None);
    assert!(read_all(read).is_empty());
}

#[test]
fn a_paste_after_the_wait_joins_the_transfer_still_pending() {
    let (mut state, requests) = state_with_transfer_callback();
    let (first_read, first_write) = pipe();
    state.on_source_send("image/png", first_write);
    let due = state
        .expire_transfers(Instant::now())
        .expect("a held paste has a deadline");
    state.expire_transfers(due);
    assert!(read_all(first_read).is_empty());

    // The transfer is still pending, so this paste raises no second request.
    let (second_read, second_write) = pipe();
    state.on_source_send("image/png", second_write);
    assert_eq!(requests.lock().unwrap().len(), 1);

    state.complete_transfer(1, Some(b"late bytes".to_vec()));
    assert_eq!(read_all(second_read), b"late bytes");
}

#[test]
fn an_answer_after_the_wait_is_kept_for_the_next_paste() {
    let (mut state, requests) = state_with_transfer_callback();
    let (read, write) = pipe();
    state.on_source_send("image/png", write);
    let due = state
        .expire_transfers(Instant::now())
        .expect("a held paste has a deadline");
    state.expire_transfers(due);
    assert!(read_all(read).is_empty());

    state.complete_transfer(1, Some(b"late bytes".to_vec()));
    let (read, write) = pipe();
    state.on_source_send("image/png", write);
    assert_eq!(read_all(read), b"late bytes");
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn a_reader_that_takes_nothing_ends_the_write_after_the_idle_time() {
    let (_read, write) = pipe();
    let mut pipe_end = std::fs::File::from(write);
    // Far more than a pipe holds, so the write has to wait for the reader.
    let data = vec![0x5a; 1 << 20];

    let error = write_with_idle_timeout(&mut pipe_end, &data, Duration::from_millis(200)).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
}

#[test]
fn a_slow_reader_that_keeps_reading_is_not_cut_off() {
    let (read, write) = pipe();
    let data: Vec<u8> = (0..=250u8).cycle().take(256 * 1024).collect();
    let reader = std::thread::spawn(move || {
        let mut pipe_end = std::fs::File::from(read);
        let mut received = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            // Every pause is far shorter than the idle time the writer is given.
            std::thread::sleep(Duration::from_millis(50));
            match pipe_end.read(&mut buf).unwrap() {
                0 => break received,
                n => received.extend_from_slice(&buf[..n]),
            }
        }
    });

    let mut pipe_end = std::fs::File::from(write);
    write_with_idle_timeout(&mut pipe_end, &data, Duration::from_secs(2)).unwrap();
    drop(pipe_end);

    assert_eq!(reader.join().unwrap(), data);
}

#[test]
fn the_change_callback_may_lock_the_shared_state() {
    let mut state = State::default();
    let shared = Arc::clone(state.shared());
    let relocked = Arc::new(Mutex::new(false));
    let seen = Arc::clone(&relocked);
    state.shared().lock().unwrap().on_change = Some(Arc::new(move |_types: Vec<String>| {
        // A callback that reads the handle locks this state; it must not be held.
        *seen.lock().unwrap() = shared.try_lock().is_ok();
    }));

    state.on_selection_cleared();

    assert!(*relocked.lock().unwrap());
}

/// Poison the lock of the shared state, as a panic in a thread that holds it does.
fn poison(shared: &Arc<Mutex<Shared>>) {
    let shared = Arc::clone(shared);
    let thread = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        unreachable!("poisoning the lock on purpose");
    });
    assert!(thread.join().is_err());
}

#[test]
fn a_cleared_selection_is_still_recorded_after_the_lock_was_poisoned() {
    let mut state = State::default();
    let called = Arc::new(Mutex::new(false));
    let seen = Arc::clone(&called);
    state.shared().lock().unwrap().on_change = Some(Arc::new(move |_types: Vec<String>| {
        *seen.lock().unwrap() = true;
    }));
    poison(state.shared());

    state.on_selection_cleared();

    assert!(*called.lock().unwrap());
    assert_eq!(state.shared().lock().unwrap_or_else(PoisonError::into_inner).serial, 1);
}

#[test]
fn a_paste_still_reaches_the_transfer_callback_after_the_lock_was_poisoned() {
    let (mut state, requests) = state_with_transfer_callback();
    poison(state.shared());

    let (_read, write) = pipe();
    state.on_source_send("image/png", write);

    assert_eq!(*requests.lock().unwrap(), vec![(1, "image/png".to_owned())]);
}

#[test]
fn cancelling_our_source_still_ends_ownership_after_the_lock_was_poisoned() {
    let mut state = State::default();
    state.shared().lock().unwrap().own_source_live = true;
    poison(state.shared());

    state.on_source_cancelled();

    let shared = state.shared().lock().unwrap_or_else(PoisonError::into_inner);
    assert!(!shared.own_source_live);
}

/// Replace the selection, as a selection event from the compositor does.
fn select(shared: &Mutex<Shared>, types: &[&str]) {
    let mut shared = shared.lock().unwrap();
    shared.serial += 1;
    shared.mime_types = mimes(types);
}

/// A clipboard handle with no worker, over a selection that offers `types`, and the commands it sends.
fn clipboard_offering(types: &[&str]) -> (DataControl, Arc<Mutex<Shared>>, mpsc::Receiver<Command>) {
    let shared: Arc<Mutex<Shared>> = Arc::default();
    select(&shared, types);
    let (clipboard, commands) = DataControl::without_worker(Arc::clone(&shared)).unwrap();
    (clipboard, shared, commands)
}

/// The write end of the pipe that a read sent to the worker.
fn receive_request(commands: &mpsc::Receiver<Command>) -> OwnedFd {
    let Command::ReceiveFromOffer { fd, .. } = commands.recv().unwrap() else {
        unreachable!("a read sends ReceiveFromOffer");
    };
    fd
}

#[test]
fn a_read_returns_what_the_source_wrote() {
    let (clipboard, _shared, commands) = clipboard_offering(&["text/plain"]);
    let source = std::thread::spawn(move || {
        let mut pipe = std::fs::File::from(receive_request(&commands));
        pipe.write_all(b"hello").unwrap();
    });

    assert_eq!(clipboard.read("text/plain").unwrap(), Some(b"hello".to_vec()));
    source.join().unwrap();
}

#[test]
fn a_source_that_writes_nothing_gives_an_empty_read() {
    let (clipboard, _shared, commands) = clipboard_offering(&["text/plain"]);
    let source = std::thread::spawn(move || drop(receive_request(&commands)));

    assert_eq!(clipboard.read("text/plain").unwrap(), Some(Vec::new()));
    source.join().unwrap();
}

#[test]
fn a_read_fails_when_the_worker_stopped_before_anything_was_sent() {
    let (clipboard, shared, commands) = clipboard_offering(&["text/plain"]);
    let source = std::thread::spawn(move || {
        let pipe = receive_request(&commands);
        // The worker failed with the request still queued, so the pipe closes with nothing.
        shared.lock().unwrap().stopped = true;
        drop(pipe);
    });

    assert!(matches!(clipboard.read("text/plain"), Err(Error::Stopped)));
    source.join().unwrap();
}

#[test]
fn data_that_arrived_is_returned_even_if_the_worker_has_stopped_since() {
    let (clipboard, shared, commands) = clipboard_offering(&["text/plain"]);
    let source = std::thread::spawn(move || {
        let mut pipe = std::fs::File::from(receive_request(&commands));
        pipe.write_all(b"hello").unwrap();
        shared.lock().unwrap().stopped = true;
    });

    assert_eq!(clipboard.read("text/plain").unwrap(), Some(b"hello".to_vec()));
    source.join().unwrap();
}

#[test]
fn a_read_asks_the_source_for_the_type_it_offered_and_returns_its_bytes() {
    let (clipboard, _shared, commands) = clipboard_offering(&["text/plain;charset=iso-8859-1"]);
    let source = std::thread::spawn(move || {
        let Command::ReceiveFromOffer { mime_type, fd } = commands.recv().unwrap() else {
            unreachable!("a read sends ReceiveFromOffer");
        };
        std::fs::File::from(fd).write_all(b"\xe9t\xe9").unwrap();
        mime_type
    });

    // The bytes are in the source's charset, not the requested one, and the request says so.
    assert_eq!(
        clipboard.read("text/plain;charset=utf-8").unwrap(),
        Some(b"\xe9t\xe9".to_vec())
    );
    assert_eq!(source.join().unwrap(), "text/plain;charset=iso-8859-1");
}

#[test]
fn a_read_of_a_type_the_selection_does_not_offer_is_none() {
    let (clipboard, _shared, commands) = clipboard_offering(&["text/plain"]);

    assert_eq!(clipboard.read("image/png").unwrap(), None);
    assert!(
        commands.try_recv().is_err(),
        "nothing is requested for a type that isn't offered"
    );
}

#[test]
fn a_read_fails_when_the_selection_changed_before_the_worker_answered() {
    let (clipboard, shared, commands) = clipboard_offering(&["text/plain"]);
    let source = std::thread::spawn(move || {
        let pipe = receive_request(&commands);
        // The worker handles the selection event ahead of the request, so the offer the request was
        // made against is gone and the pipe closes with nothing.
        select(&shared, &["image/png"]);
        drop(pipe);
    });

    assert!(matches!(clipboard.read("text/plain"), Err(Error::SelectionChanged)));
    source.join().unwrap();
}

#[test]
fn a_read_fails_when_the_selection_changes_while_the_data_arrives() {
    let (clipboard, shared, commands) = clipboard_offering(&["text/plain"]);
    let source = std::thread::spawn(move || {
        let mut pipe = std::fs::File::from(receive_request(&commands));
        pipe.write_all(b"he").unwrap();
        select(&shared, &["text/plain"]);
        pipe.write_all(b"llo").unwrap();
    });

    assert!(matches!(clipboard.read("text/plain"), Err(Error::SelectionChanged)));
    source.join().unwrap();
}

/// Raise a transfer for `image/png` through the callback the handle installed, as the worker does for a paste.
fn raise_transfer(shared: &Mutex<Shared>, serial: u32) {
    let on_transfer = shared
        .lock()
        .unwrap()
        .on_transfer
        .clone()
        .expect("the handle installed a transfer callback");
    on_transfer(serial, "image/png".to_owned());
}

/// The answer a transfer request sent to the worker.
fn transfer_answer(commands: &mpsc::Receiver<Command>) -> (u32, Option<Vec<u8>>) {
    let Command::CompleteTransfer { serial, data } = commands.try_recv().unwrap() else {
        unreachable!("a transfer request answers with CompleteTransfer");
    };
    (serial, data)
}

#[test]
fn a_dropped_transfer_request_closes_its_paste() {
    let (clipboard, shared, commands) = clipboard_offering(&["image/png"]);
    clipboard.on_transfer(|_request| {});

    raise_transfer(&shared, 7);

    assert_eq!(transfer_answer(&commands), (7, None));
    assert!(commands.try_recv().is_err(), "one answer only");
}

#[test]
fn a_completed_transfer_request_answers_once() {
    let (clipboard, shared, commands) = clipboard_offering(&["image/png"]);
    clipboard.on_transfer(|request| request.complete("png bytes").unwrap());

    raise_transfer(&shared, 7);

    assert_eq!(transfer_answer(&commands), (7, Some(b"png bytes".to_vec())));
    assert!(
        commands.try_recv().is_err(),
        "dropping the answered request must not answer again"
    );
}

#[test]
fn a_failed_transfer_request_answers_once() {
    let (clipboard, shared, commands) = clipboard_offering(&["image/png"]);
    clipboard.on_transfer(|request| request.fail().unwrap());

    raise_transfer(&shared, 7);

    assert_eq!(transfer_answer(&commands), (7, None));
    assert!(
        commands.try_recv().is_err(),
        "dropping the failed request must not answer again"
    );
}
