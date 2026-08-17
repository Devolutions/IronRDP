use core::time::Duration;
use ironrdp_rdpeudp::*;
#[test]
fn duration_since_measures_forward_difference() {
    let earlier = MonotonicInstant::from_millis(1_000);
    let later = MonotonicInstant::from_millis(1_750);

    assert_eq!(later.duration_since(earlier), Duration::from_millis(750));
}

#[test]
fn duration_since_saturates_when_transposed() {
    let earlier = MonotonicInstant::from_millis(1_000);
    let later = MonotonicInstant::from_millis(1_750);

    assert_eq!(earlier.duration_since(later), Duration::ZERO);
}

#[test]
fn add_advances_by_the_duration() {
    let base = MonotonicInstant::from_millis(500);

    assert_eq!(base + Duration::from_millis(250), MonotonicInstant::from_millis(750));
}

#[test]
fn add_saturates_at_the_end_of_the_range() {
    let base = MonotonicInstant::from_millis(u64::MAX - 1);

    assert_eq!(base + Duration::from_secs(60), MonotonicInstant::from_millis(u64::MAX));
}

#[test]
fn add_saturates_when_the_duration_itself_overflows() {
    let base = MonotonicInstant::from_millis(0);

    assert_eq!(base + Duration::MAX, MonotonicInstant::from_millis(u64::MAX));
}

#[test]
fn instants_order_by_reading() {
    assert!(MonotonicInstant::from_millis(1) < MonotonicInstant::from_millis(2));
}
