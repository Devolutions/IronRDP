use core::{error::Error as _, fmt};

use ironrdp_error::{Error, ErrorMapping, ResultExt as _};

#[derive(Debug)]
struct TestKind(&'static str);

impl fmt::Display for TestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl core::error::Error for TestKind {}

#[derive(Debug)]
struct InnerKind;

impl fmt::Display for InnerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "inner")
    }
}

impl core::error::Error for InnerKind {}

#[derive(Debug)]
enum OuterKind {
    Canonical,
    Wrapped(Error<InnerKind>),
    Source,
}

impl fmt::Display for OuterKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canonical => write!(f, "canonical"),
            Self::Wrapped(_) => write!(f, "wrapped"),
            Self::Source => write!(f, "source"),
        }
    }
}

impl core::error::Error for OuterKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Wrapped(error) => Some(error),
            Self::Canonical | Self::Source => None,
        }
    }
}

impl ErrorMapping<InnerKind> for OuterKind {
    #[track_caller]
    fn map_error(_: Error<InnerKind>) -> Error<Self> {
        Error::new("canonical", Self::Canonical)
    }
}

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

fn inner_result() -> Result<(), Error<InnerKind>> {
    Err(Error::new("inner", InnerKind))
}

#[track_caller]
fn map_canonical() -> Error<OuterKind> {
    inner_result().map_err_as::<OuterKind>().unwrap_err()
}

#[track_caller]
fn map_kind() -> Error<OuterKind> {
    inner_result().map_err_kind("wrapped", OuterKind::Wrapped).unwrap_err()
}

#[track_caller]
fn map_source() -> Error<OuterKind> {
    inner_result().map_err_source("source", OuterKind::Source).unwrap_err()
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

#[test]
fn map_err_as_uses_the_canonical_mapping() {
    let error = map_canonical();

    assert!(matches!(error.kind(), OuterKind::Canonical));
    assert_eq!(error.to_string(), "[canonical] canonical");
}

#[test]
fn map_err_kind_embeds_the_complete_inner_error() {
    let error = map_kind();

    let OuterKind::Wrapped(inner) = error.kind() else {
        panic!("expected wrapped error kind");
    };
    assert_eq!(error.to_string(), "[wrapped] wrapped");
    assert_eq!(inner.to_string(), "[inner] inner");
    assert_eq!(error.source().unwrap().to_string(), "[inner] inner");
}

#[test]
fn map_err_source_attaches_the_inner_error_as_source() {
    let error = map_source();

    assert!(matches!(error.kind(), OuterKind::Source));
    assert_eq!(error.to_string(), "[source] source");
    assert_eq!(error.source().unwrap().to_string(), "[inner] inner");
    assert_eq!(error.report().to_string(), "[source] source, caused by: [inner] inner");
}

#[test]
fn error_mapping_preserves_the_callers_location() {
    let canonical_line = line!() + 1;
    let canonical = map_canonical();
    assert_eq!(canonical.location().file(), file!());
    assert_eq!(canonical.location().line(), canonical_line);

    let kind_line = line!() + 1;
    let kind = map_kind();
    assert_eq!(kind.location().file(), file!());
    assert_eq!(kind.location().line(), kind_line);

    let source_line = line!() + 1;
    let source = map_source();
    assert_eq!(source.location().file(), file!());
    assert_eq!(source.location().line(), source_line);
}
