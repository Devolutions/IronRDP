#![allow(unused_crate_dependencies)]

use std::sync::{Arc, Mutex};

use ironrdp_mstsgu::GwErrorKind;
use ironrdp_mstsgu::test_support::evaluate_consent_message;

fn consent_message(message: &str) -> Vec<u8> {
    message
        .encode_utf16()
        .chain(core::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[test]
fn no_consent_preserves_existing_behavior() {
    let callback_count = Arc::new(Mutex::new(0));
    let callback_count_for_callback = Arc::clone(&callback_count);
    let mut callback = move |_message: &str| {
        *callback_count_for_callback.lock().expect("callback count lock") += 1;
        false
    };

    evaluate_consent_message(&[], Some(&mut callback)).expect("no consent");
    assert_eq!(*callback_count.lock().expect("callback count lock"), 0);
}

#[test]
fn default_accepts_gateway_consent() {
    evaluate_consent_message(&consent_message("Accept"), None).expect("default consent acceptance");
}

#[test]
fn callback_receives_decoded_consent_message_once() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let callback_messages = Arc::clone(&messages);
    let mut callback = move |message: &str| {
        callback_messages
            .lock()
            .expect("callback messages lock")
            .push(message.to_owned());
        true
    };

    evaluate_consent_message(&consent_message("Accept"), Some(&mut callback)).expect("accepted consent");
    assert_eq!(*messages.lock().expect("callback messages lock"), ["Accept"]);
}

#[test]
fn callback_rejection_returns_consent_declined() {
    let mut callback = |_message: &str| false;
    let error =
        evaluate_consent_message(&consent_message("Accept"), Some(&mut callback)).expect_err("declined consent");

    assert!(matches!(error.kind(), GwErrorKind::ConsentDeclined));
}

#[test]
fn malformed_consent_payload_is_rejected_before_callback() {
    let callback_count = Arc::new(Mutex::new(0));
    let callback_count_for_callback = Arc::clone(&callback_count);
    let mut callback = move |_message: &str| {
        *callback_count_for_callback.lock().expect("callback count lock") += 1;
        true
    };
    let error = evaluate_consent_message(&[0x41], Some(&mut callback)).expect_err("malformed consent");

    assert!(matches!(error.kind(), GwErrorKind::Decode));
    assert_eq!(*callback_count.lock().expect("callback count lock"), 0);
}
