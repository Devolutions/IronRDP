//! Tests for the Wayland data-control clipboard client that need no compositor.

use std::{
    io::Read as _,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};

use ironrdp_cliprdr_native::data_control::state::{State, is_own_selection};
use ironrdp_cliprdr_native::data_control::{Options, Preference, Protocol, find_mime_match};

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
fn own_selection_is_recognised_while_our_source_is_live() {
    let ours = mimes(&["text/plain;charset=utf-8"]);
    assert!(is_own_selection(true, &ours, &mimes(&["text/plain;charset=utf-8"])));
}

#[test]
fn a_selection_after_our_source_was_cancelled_is_foreign() {
    let ours = mimes(&["text/plain;charset=utf-8"]);
    assert!(!is_own_selection(false, &ours, &mimes(&["text/plain;charset=utf-8"])));
}

#[test]
fn a_different_offer_while_our_source_is_live_is_foreign() {
    let ours = mimes(&["text/plain;charset=utf-8"]);
    let other = mimes(&[
        "text/plain;charset=utf-8",
        "text/plain",
        "TEXT",
        "STRING",
        "UTF8_STRING",
    ]);
    assert!(!is_own_selection(true, &ours, &other));
}

#[test]
fn own_selection_ignores_offer_order() {
    let ours = mimes(&["image/png", "text/html"]);
    assert!(is_own_selection(true, &ours, &mimes(&["text/html", "image/png"])));
}

#[test]
fn clearing_the_selection_notifies_and_advances_the_serial() {
    let mut state = State::default();
    let called = Arc::new(Mutex::new(false));
    let seen = Arc::clone(&called);
    state
        .shared()
        .lock()
        .unwrap()
        .set_on_change(Arc::new(move |types: Vec<String>| {
            assert!(types.is_empty());
            *seen.lock().unwrap() = true;
        }));

    state.on_selection_cleared();

    assert!(*called.lock().unwrap());
    let shared = state.shared().lock().unwrap();
    assert!(shared.mime_types().is_empty());
    assert_eq!(shared.serial(), 1);
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

type TransferLog = Arc<Mutex<Vec<(u32, String)>>>;

/// State advertising `image/png` with a transfer callback that records what it was asked for.
fn state_with_transfer_callback() -> (State, TransferLog) {
    let mut state = State::default();
    state.set_advertised(mimes(&["image/png"]));
    let requests: TransferLog = Arc::default();
    let seen = Arc::clone(&requests);
    state
        .shared()
        .lock()
        .unwrap()
        .set_on_transfer(Arc::new(move |serial, mime| {
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
fn a_paste_of_an_unadvertised_type_raises_no_transfer() {
    let (mut state, requests) = state_with_transfer_callback();
    let (read, write) = pipe();
    state.on_source_send("text/html", write);
    assert!(read_all(read).is_empty());
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn the_change_callback_may_lock_the_shared_state() {
    let mut state = State::default();
    let shared = Arc::clone(state.shared());
    let relocked = Arc::new(Mutex::new(false));
    let seen = Arc::clone(&relocked);
    state
        .shared()
        .lock()
        .unwrap()
        .set_on_change(Arc::new(move |_types: Vec<String>| {
            // A callback that reads the handle locks this state; it must not be held.
            *seen.lock().unwrap() = shared.try_lock().is_ok();
        }));

    state.on_selection_cleared();

    assert!(*relocked.lock().unwrap());
}

#[test]
fn held_paste_descriptors_are_bounded_and_released_on_cancellation() {
    let (mut state, requests) = state_with_transfer_callback();
    let mut readers = Vec::new();
    for _ in 0..16 {
        let (reader, writer) = pipe();
        state.on_source_send("image/png", writer);
        readers.push(reader);
    }
    assert_eq!(state.paste_count(), 16);
    let (rejected, writer) = pipe();
    state.on_source_send("image/png", writer);
    assert!(read_all(rejected).is_empty());
    assert_eq!(requests.lock().unwrap().len(), 1);

    state.on_source_cancelled();
    assert_eq!(state.paste_count(), 0);
    for reader in readers {
        assert!(read_all(reader).is_empty());
    }
    state.set_advertised(mimes(&["image/png"]));
    let (reader, writer) = pipe();
    state.on_source_send("image/png", writer);
    // The previous response cannot answer the next selection's paste.
    state.complete_transfer(1, Some(b"stale".to_vec()));
    assert_eq!(state.paste_count(), 1);
    state.complete_transfer(2, Some(b"current".to_vec()));
    assert_eq!(read_all(reader), b"current");
}

#[test]
fn unanswered_pastes_expire_without_replacing_the_selection() {
    use core::time::Duration;
    use std::time::Instant;

    let (mut state, requests) = state_with_transfer_callback();
    let (reader, writer) = pipe();
    state.on_source_send("image/png", writer);
    state.expire_transfers(Instant::now() + Duration::from_secs(6));
    assert!(read_all(reader).is_empty());
    assert_eq!(state.paste_count(), 0);
    state.complete_transfer(1, Some(b"late".to_vec()));
    assert!(!state.has_source_data());

    let (reader, writer) = pipe();
    state.on_source_send("image/png", writer);
    assert_eq!(requests.lock().unwrap().len(), 2);
    state.complete_transfer(2, Some(b"new".to_vec()));
    assert_eq!(read_all(reader), b"new");
}

#[test]
fn cached_pastes_share_the_descriptor_and_writer_limit() {
    let mut state = State::default();
    state.insert_source_data("image/png", vec![0x55; 1024 * 1024]);
    let mut readers = Vec::new();
    for _ in 0..16 {
        let (reader, writer) = pipe();
        state.on_source_send("image/png", writer);
        readers.push(reader);
    }
    assert_eq!(state.paste_count(), 16);
    let (rejected, writer) = pipe();
    state.on_source_send("image/png", writer);
    assert!(read_all(rejected).is_empty());
    // Breaking each blocked pipe releases its thread and its permit.
    drop(readers);
    wait_for_pastes_to_finish(&state);
    let (reader, writer) = pipe();
    state.insert_source_data("image/png", b"available again".to_vec());
    state.on_source_send("image/png", writer);
    assert_eq!(read_all(reader), b"available again");
}

#[test]
fn a_blocked_paste_writer_times_out_and_closes_its_descriptor() {
    use core::time::Duration;

    let mut state = State::default();
    state.set_transfer_timeout(Duration::from_millis(30));
    state.insert_source_data("image/png", vec![0x55; 1024 * 1024]);
    let (reader, writer) = pipe();
    state.on_source_send("image/png", writer);
    // Keep the reader open without draining its pipe: write_all used to block forever.
    wait_for_pastes_to_finish(&state);
    assert!(read_all(reader).len() < 1024 * 1024);
}

fn wait_for_pastes_to_finish(state: &State) {
    use core::time::Duration;
    use std::time::Instant;

    let deadline = Instant::now() + Duration::from_secs(2);
    while state.paste_count() != 0 {
        assert!(Instant::now() < deadline, "clipboard writer did not release its permit");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_read_deadline_is_not_extended_by_slow_progress() {
    use core::time::Duration;
    use std::io::Write as _;
    use std::sync::mpsc;

    use ironrdp_cliprdr_native::data_control::{Error, read_pipe};

    let (mut reader, mut writer) = std::io::pipe().unwrap();
    let (started, ready) = mpsc::sync_channel(0);
    let thread = std::thread::spawn(move || {
        writer.write_all(b"a").unwrap();
        started.send(()).unwrap();
        for _ in 0..1000 {
            std::thread::sleep(Duration::from_millis(1));
            if writer.write_all(b"a").is_err() {
                break;
            }
        }
    });
    ready.recv().unwrap();
    let result = read_pipe(&mut reader, 1024 * 1024, Duration::from_millis(30));
    drop(reader);
    thread.join().unwrap();
    assert!(matches!(result, Err(Error::Timeout)));
}

#[test]
fn same_format_selection_before_cancellation_still_notifies_ownership_loss() {
    let mut state = State::default();
    let formats = mimes(&["text/plain"]);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&observed);
    let shared = Arc::clone(state.shared());
    state.shared().lock().unwrap().set_on_change(Arc::new(move |mimes| {
        assert!(
            shared.try_lock().is_ok(),
            "callback must run after releasing shared state"
        );
        seen.lock().unwrap().push(mimes);
    }));
    state.set_advertised(formats.clone());
    state.publish_selection(formats.clone(), true);
    // Without the cancellation yet, the existing ownership heuristic cannot
    // distinguish this external selection from our own same-format echo.
    let assumed_own = is_own_selection(true, &formats, &formats);
    state.publish_selection(formats.clone(), assumed_own);
    assert!(observed.lock().unwrap().is_empty());

    state.on_source_cancelled();
    assert_eq!(*observed.lock().unwrap(), vec![formats]);
    assert_eq!(state.shared().lock().unwrap().serial(), 3);
}

#[test]
fn cancellation_before_external_selection_converges_on_the_new_formats() {
    let mut state = State::default();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&observed);
    state
        .shared()
        .lock()
        .unwrap()
        .set_on_change(Arc::new(move |mimes| seen.lock().unwrap().push(mimes)));
    state.publish_selection(mimes(&["text/plain"]), true);
    state.on_source_cancelled();
    state.publish_selection(mimes(&["image/png"]), false);
    assert_eq!(
        *observed.lock().unwrap(),
        vec![mimes(&["text/plain"]), mimes(&["image/png"])]
    );
    assert_eq!(state.shared().lock().unwrap().mime_types(), mimes(&["image/png"]));
}

#[test]
fn cancellation_after_a_foreign_selection_does_not_repeat_its_notification() {
    let mut state = State::default();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&observed);
    state
        .shared()
        .lock()
        .unwrap()
        .set_on_change(Arc::new(move |mimes| seen.lock().unwrap().push(mimes)));
    state.publish_selection(mimes(&["text/plain"]), true);
    state.publish_selection(mimes(&["image/png"]), false);
    state.on_source_cancelled();
    assert_eq!(*observed.lock().unwrap(), vec![mimes(&["image/png"])]);
}
