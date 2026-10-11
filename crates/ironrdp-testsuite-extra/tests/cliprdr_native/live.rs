//! Tests against a running Wayland compositor.
//!
//! They change the clipboard of the compositor they connect to, so they are ignored by default and do
//! nothing unless `IRONRDP_DATA_CONTROL_LIVE=1` is set. Run them inside an isolated compositor, for
//! example `kwin_wayland --virtual --no-lockscreen --socket ironrdp-live`, one at a time because they
//! share its clipboard:
//!
//! ```text
//! WAYLAND_DISPLAY=ironrdp-live IRONRDP_DATA_CONTROL_LIVE=1 \
//!     cargo test -p ironrdp-testsuite-extra -- --ignored --test-threads=1 cliprdr_native::live
//! ```
//!
//! They use whichever data-control protocol the compositor offers. One of them pastes with `wl-paste`
//! (from wl-clipboard) and fails without it, because the paste has to wait longer than the reader of
//! this library does.

use core::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use std::{
    process::Command,
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};

use ironrdp_cliprdr_native::data_control::{Content, DataControl, TransferRequest};

const TEXT: &str = "text/plain;charset=utf-8";

fn enabled() -> bool {
    std::env::var("IRONRDP_DATA_CONTROL_LIVE").as_deref() == Ok("1")
}

fn wait_for_change(rx: &mpsc::Receiver<Vec<String>>) -> Vec<String> {
    rx.recv_timeout(Duration::from_secs(5)).expect("the selection changed")
}

/// Wait until the handle has seen a selection newer than `serial`, which is how it learns about one it set
/// itself.
fn wait_for_serial(clipboard: &DataControl, serial: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while clipboard.serial() == serial {
        assert!(Instant::now() < deadline, "no selection event");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn set_then_read_back() {
    if !enabled() {
        return;
    }
    let clipboard = DataControl::connect().unwrap();
    let before = clipboard.serial();

    clipboard
        .set_selection(Content::new().data(TEXT, "round trip"))
        .unwrap();
    wait_for_serial(&clipboard, before);

    assert_eq!(clipboard.read("text/plain").unwrap(), Some(b"round trip".to_vec()));
    assert!(clipboard.owns_selection());
    assert_eq!(clipboard.selection_mime_types(), vec![TEXT.to_owned()]);
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn a_second_client_reads_our_selection() {
    if !enabled() {
        return;
    }
    let owner = DataControl::connect().unwrap();
    let reader = DataControl::connect().unwrap();
    let (tx, rx) = mpsc::channel();
    reader.on_change(move |types| {
        let _ = tx.send(types);
    });

    owner
        .set_selection(Content::new().data(TEXT, "seen by another client"))
        .unwrap();

    // The marker of the owner is not among the types the reader is told about.
    assert_eq!(wait_for_change(&rx), vec![TEXT.to_owned()]);
    assert_eq!(reader.read(TEXT).unwrap(), Some(b"seen by another client".to_vec()));
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn delayed_rendering_produces_data_on_paste() {
    if !enabled() {
        return;
    }
    let owner = DataControl::connect().unwrap();
    let reader = DataControl::connect().unwrap();
    let (tx, rx) = mpsc::channel();
    reader.on_change(move |types| {
        let _ = tx.send(types);
    });
    owner.on_transfer(|request| request.complete("rendered late").unwrap());

    owner.set_selection(Content::new().advertise(TEXT)).unwrap();
    wait_for_change(&rx);

    assert_eq!(reader.read(TEXT).unwrap(), Some(b"rendered late".to_vec()));
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn a_replaced_selection_is_not_served_from_our_cache() {
    if !enabled() {
        return;
    }
    let first = DataControl::connect().unwrap();
    let second = DataControl::connect().unwrap();
    let (tx, rx) = mpsc::channel();
    first.on_change(move |types| {
        let _ = tx.send(types);
    });
    let before = first.serial();

    first.set_selection(Content::new().data(TEXT, "old")).unwrap();
    wait_for_serial(&first, before);
    second.set_selection(Content::new().data(TEXT, "new")).unwrap();
    wait_for_change(&rx);

    assert_eq!(first.read(TEXT).unwrap(), Some(b"new".to_vec()));
    assert!(!first.owns_selection());
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn our_own_selection_is_not_reported_but_a_copy_with_the_same_types_is() {
    if !enabled() {
        return;
    }
    let owner = DataControl::connect().unwrap();
    let other = DataControl::connect().unwrap();
    let (tx, rx) = mpsc::channel();
    owner.on_change(move |types| {
        let _ = tx.send(types);
    });
    let before = owner.serial();

    owner.set_selection(Content::new().data(TEXT, "ours")).unwrap();
    wait_for_serial(&owner, before);
    // The callback would run right after the serial moves, so a short wait is enough to see it.
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(500)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "our own selection was reported"
    );

    other.set_selection(Content::new().data(TEXT, "theirs")).unwrap();

    // The other client offers the same types as we did, and its marker is not among them.
    assert_eq!(wait_for_change(&rx), vec![TEXT.to_owned()]);
    assert_eq!(owner.read(TEXT).unwrap(), Some(b"theirs".to_vec()));
    assert!(!owner.owns_selection());
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn our_own_clear_is_reported_as_an_empty_selection() {
    if !enabled() {
        return;
    }
    let owner = DataControl::connect().unwrap();
    let (tx, rx) = mpsc::channel();
    owner.on_change(move |types| {
        let _ = tx.send(types);
    });
    let before = owner.serial();
    owner.set_selection(Content::new().data(TEXT, "ours")).unwrap();
    wait_for_serial(&owner, before);

    owner.clear_selection().unwrap();

    assert_eq!(wait_for_change(&rx), Vec::<String>::new());
    assert!(!owner.owns_selection());
}

#[test]
#[ignore = "changes the clipboard of the running compositor, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn a_dropped_request_closes_the_paste_at_once_and_the_next_paste_raises_a_new_request() {
    if !enabled() {
        return;
    }
    let owner = DataControl::connect().unwrap();
    let reader = DataControl::connect().unwrap();
    let (tx, rx) = mpsc::channel();
    reader.on_change(move |types| {
        let _ = tx.send(types);
    });
    let raised = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&raised);
    owner.on_transfer(move |_request| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    owner.set_selection(Content::new().advertise(TEXT)).unwrap();
    wait_for_change(&rx);

    for round in 1..=2 {
        let started = Instant::now();
        assert_eq!(reader.read(TEXT).unwrap(), Some(Vec::new()));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "paste {round} waited too long"
        );
        assert_eq!(raised.load(Ordering::SeqCst), round);
    }
}

#[test]
#[ignore = "changes the clipboard of the running compositor, needs wl-paste, set IRONRDP_DATA_CONTROL_LIVE=1 to run"]
fn an_unanswered_paste_is_closed_after_five_seconds_and_a_late_answer_is_kept() {
    if !enabled() {
        return;
    }
    let paste = || {
        let started = Instant::now();
        let output = Command::new("wl-paste")
            .args(["--no-newline", "--type", TEXT])
            .output()
            .expect("wl-paste (from wl-clipboard) is needed to paste in this test");
        (output.stdout, started.elapsed())
    };

    let owner = DataControl::connect().unwrap();
    let held: Arc<Mutex<Vec<TransferRequest>>> = Arc::default();
    let raised = Arc::new(AtomicUsize::new(0));
    {
        let held = Arc::clone(&held);
        let raised = Arc::clone(&raised);
        owner.on_transfer(move |request| {
            raised.fetch_add(1, Ordering::SeqCst);
            held.lock().unwrap().push(request);
        });
    }
    let before = owner.serial();
    owner.set_selection(Content::new().advertise(TEXT)).unwrap();
    wait_for_serial(&owner, before);

    // Nobody answers the first paste, so it is closed with no data after about five seconds.
    let (data, waited) = paste();
    assert!(data.is_empty());
    assert!(
        Duration::from_millis(4500) < waited && waited < Duration::from_secs(8),
        "waited {waited:?}"
    );
    assert_eq!(raised.load(Ordering::SeqCst), 1);

    // The answer arrives late and is kept for the next paste, which raises no new request.
    let request = held.lock().unwrap().pop().expect("a request was held");
    request.complete("late answer").unwrap();
    let (data, waited) = paste();
    assert_eq!(data, b"late answer");
    assert!(waited < Duration::from_secs(2), "waited {waited:?}");
    assert_eq!(raised.load(Ordering::SeqCst), 1);
}
