use core::fmt;

use ironrdp_error::Error;

#[derive(Debug)]
struct TestKind(&'static str);

impl fmt::Display for TestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl core::error::Error for TestKind {}

#[derive(Debug)]
struct NestedKind {
    source: Error<TestKind>,
}

impl fmt::Display for NestedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "outer")
    }
}

impl core::error::Error for NestedKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[test]
fn display_omits_location_by_default() {
    let error = Error::new("context", TestKind("kind"));

    assert_eq!(error.to_string(), "[context] kind");
}

#[test]
fn alternate_display_includes_location() {
    let error = Error::new("context", TestKind("kind"));

    assert_eq!(
        format!("{error:#}"),
        format!(
            "[context @ {}:{}] kind",
            error.location().file(),
            error.location().line()
        )
    );
}

#[test]
fn report_can_enable_locations_explicitly_or_with_alternate_formatting() {
    let error = Error::new("context", TestKind("kind"));
    let expected = format!(
        "[context @ {}:{}] kind",
        error.location().file(),
        error.location().line()
    );

    assert_eq!(error.report().to_string(), "[context] kind");
    assert_eq!(error.report().with_locations().to_string(), expected);
    assert_eq!(format!("{:#}", error.report()), expected);
}

#[test]
fn report_formats_nested_ironrdp_errors_with_locations() {
    let source = Error::new("inner", TestKind("inner"));
    let source_with_location = format!(
        "[inner @ {}:{}] inner",
        source.location().file(),
        source.location().line()
    );
    let error = Error::new("outer", NestedKind { source });
    let error_with_location = format!(
        "[outer @ {}:{}] outer",
        error.location().file(),
        error.location().line()
    );

    assert_eq!(error.report().to_string(), "[outer] outer, caused by: [inner] inner");
    assert_eq!(
        error.report().with_locations().to_string(),
        format!("{error_with_location}, caused by: {source_with_location}")
    );
    assert_eq!(
        format!("{:#}", error.report()),
        format!("{error_with_location}, caused by: {source_with_location}")
    );
}

#[test]
fn report_preserves_foreign_source_errors() {
    let error = Error::new("context", TestKind("kind")).with_source(std::io::Error::other("foreign"));
    let error_with_location = format!(
        "[context @ {}:{}] kind",
        error.location().file(),
        error.location().line()
    );

    assert_eq!(error.report().to_string(), "[context] kind, caused by: foreign");
    assert_eq!(
        error.report().with_locations().to_string(),
        format!("{error_with_location}, caused by: foreign")
    );
}
