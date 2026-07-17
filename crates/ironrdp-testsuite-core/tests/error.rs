use expect_test::expect;
use ironrdp_error::Error;

#[derive(Debug)]
struct DummyKind;

impl core::fmt::Display for DummyKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "something went wrong")
    }
}

impl core::error::Error for DummyKind {}

#[derive(Debug)]
struct RootCause;

impl core::fmt::Display for RootCause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "root cause")
    }
}

impl core::error::Error for RootCause {}

#[test]
fn display_omits_source_location_by_default() {
    let error = Error::new("context", DummyKind);
    // No `@ file:line` decoration by default: the output stays stable for
    // `Display`-based snapshot testing.
    expect!["[context] something went wrong"].assert_eq(&error.to_string());
}

#[test]
fn report_omits_source_location_by_default() {
    let error = Error::new("context", DummyKind).with_source(RootCause);
    expect!["[context] something went wrong, caused by: root cause"].assert_eq(&error.report().to_string());
}

#[test]
fn report_with_source_location_includes_it() {
    let error = Error::new("context", DummyKind).with_source(RootCause);
    let report = error.report().with_source_location().to_string();
    // The location is opt-in. The concrete line number is volatile, so the
    // structure is asserted rather than snapshotted.
    assert!(report.starts_with("[context @ "), "unexpected report: {report}");
    assert!(report.contains("error.rs:"), "missing file:line in report: {report}");
    assert!(
        report.ends_with("] something went wrong, caused by: root cause"),
        "unexpected report: {report}"
    );
}
