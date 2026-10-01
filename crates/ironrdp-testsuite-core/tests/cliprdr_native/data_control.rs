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
