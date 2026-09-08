use ironrdp_cliprdr::loop_detector::{ClipboardSource, LoopDetectionConfig, LoopDetector};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, ClipboardFormatName};

fn unicode_text() -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)
}

fn html() -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::CF_TEXT).with_name(ClipboardFormatName::HTML)
}

#[test]
fn no_loop_different_formats() {
    let mut detector = LoopDetector::new();

    detector.record_formats(&[unicode_text()], ClipboardSource::Remote, 0);

    assert!(!detector.would_cause_loop(&[html()], ClipboardSource::Local, 10));
}

#[test]
fn loop_same_formats_opposite_source() {
    let mut detector = LoopDetector::new();
    let formats = [unicode_text()];

    detector.record_formats(&formats, ClipboardSource::Remote, 0);

    assert!(detector.would_cause_loop(&formats, ClipboardSource::Local, 10));
}

#[test]
fn no_loop_same_source() {
    let mut detector = LoopDetector::new();
    let formats = [unicode_text()];

    // Recorded from Local; checking as Local (i.e. against Remote history) must not match
    // an operation that was itself recorded from Local.
    detector.record_formats(&formats, ClipboardSource::Local, 0);

    assert!(!detector.would_cause_loop(&formats, ClipboardSource::Local, 10));
}

#[test]
fn loop_detection_respects_time_window() {
    let mut detector = LoopDetector::with_config(LoopDetectionConfig {
        window_ms: 100,
        ..LoopDetectionConfig::default()
    });
    let formats = [unicode_text()];

    detector.record_formats(&formats, ClipboardSource::Remote, 0);

    // Inside the window: still a loop.
    assert!(detector.would_cause_loop(&formats, ClipboardSource::Local, 100));
    // Outside the window: a later, unrelated copy of the same content is not suppressed.
    assert!(!detector.would_cause_loop(&formats, ClipboardSource::Local, 101));
}

#[test]
fn content_hash_detects_loop_and_is_source_direction_aware() {
    let mut detector = LoopDetector::new();
    let data = b"Hello, World!";

    detector.record_content(data, ClipboardSource::Remote, 0);

    assert!(detector.would_cause_content_loop(data, ClipboardSource::Local, 10));
    assert!(!detector.would_cause_content_loop(b"Different", ClipboardSource::Local, 10));
    // Same source as the recording: not a loop.
    assert!(!detector.would_cause_content_loop(data, ClipboardSource::Remote, 10));
}

#[test]
fn content_hashing_can_be_disabled() {
    let mut detector = LoopDetector::with_config(LoopDetectionConfig {
        enable_content_hashing: false,
        ..LoopDetectionConfig::default()
    });
    let data = b"Hello, World!";

    detector.record_content(data, ClipboardSource::Remote, 0);

    assert!(!detector.would_cause_content_loop(data, ClipboardSource::Local, 10));
}

#[test]
fn clear_resets_history_and_rate_limit_state() {
    let config = LoopDetectionConfig::with_rate_limit(200);
    let mut detector = LoopDetector::with_config(config);
    let formats = [unicode_text()];

    detector.record_formats(&formats, ClipboardSource::Remote, 0);
    detector.record_sync(ClipboardSource::Remote, 0);
    assert!(detector.is_rate_limited(ClipboardSource::Remote, 50));

    detector.clear();

    assert!(!detector.would_cause_loop(&formats, ClipboardSource::Local, 10));
    assert!(!detector.is_rate_limited(ClipboardSource::Remote, 50));
}

#[test]
fn compute_hash_is_stable_within_a_process_and_content_sensitive() {
    let hash1 = LoopDetector::compute_hash(b"test");
    let hash2 = LoopDetector::compute_hash(b"test");
    let hash3 = LoopDetector::compute_hash(b"different");

    assert_eq!(hash1, hash2);
    assert_ne!(hash1, hash3);
}

#[test]
fn rate_limit_disabled_by_default() {
    let detector = LoopDetector::new();

    assert!(!detector.is_rate_limited(ClipboardSource::Remote, 0));
    assert!(!detector.is_rate_limited(ClipboardSource::Local, 0));
}

#[test]
fn rate_limit_is_per_source() {
    let config = LoopDetectionConfig::with_rate_limit(200);
    let mut detector = LoopDetector::with_config(config);

    assert!(!detector.is_rate_limited(ClipboardSource::Remote, 0));

    detector.record_sync(ClipboardSource::Remote, 0);

    assert!(detector.is_rate_limited(ClipboardSource::Remote, 100));
    assert!(!detector.is_rate_limited(ClipboardSource::Local, 100));
    // Outside the rate-limit window: no longer limited.
    assert!(!detector.is_rate_limited(ClipboardSource::Remote, 200));
}

#[test]
fn should_skip_sync_combines_rate_limit_and_loop_detection() {
    let config = LoopDetectionConfig::with_rate_limit(200);
    let mut detector = LoopDetector::with_config(config);
    let formats = [unicode_text()];

    // Initially: not rate limited, no loop.
    assert!(!detector.should_skip_sync(&formats, ClipboardSource::Remote, 0));

    detector.record_formats(&formats, ClipboardSource::Remote, 0);
    detector.record_sync(ClipboardSource::Remote, 0);

    // Skipped for Local: would echo the just-recorded Remote operation.
    assert!(detector.should_skip_sync(&formats, ClipboardSource::Local, 10));
    // Skipped for Remote: rate limited.
    assert!(detector.should_skip_sync(&formats, ClipboardSource::Remote, 10));
}

#[test]
fn history_is_bounded_by_max_history() {
    let mut detector = LoopDetector::with_config(LoopDetectionConfig {
        max_history: 2,
        window_ms: 10_000,
        ..LoopDetectionConfig::default()
    });

    let first = [ClipboardFormat::new(ClipboardFormatId::new(1))];
    let second = [ClipboardFormat::new(ClipboardFormatId::new(2))];
    let third = [ClipboardFormat::new(ClipboardFormatId::new(3))];

    detector.record_formats(&first, ClipboardSource::Remote, 0);
    detector.record_formats(&second, ClipboardSource::Remote, 1);
    detector.record_formats(&third, ClipboardSource::Remote, 2);

    // The oldest entry (`first`) was evicted once max_history was exceeded.
    assert!(!detector.would_cause_loop(&first, ClipboardSource::Local, 3));
    assert!(detector.would_cause_loop(&second, ClipboardSource::Local, 3));
    assert!(detector.would_cause_loop(&third, ClipboardSource::Local, 3));
}
