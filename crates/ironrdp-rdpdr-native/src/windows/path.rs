//! Validation for server-supplied RDPDR filesystem paths.
//!
//! RDPDR carries Unicode path strings, but the local Windows namespace has
//! additional aliases and rooted forms that must never be resolved outside an
//! announced volume root.

const MAX_PATH_CODE_UNITS: usize = 32_767;
const MAX_COMPONENT_CODE_UNITS: usize = 255;

/// A validated path relative to a redirected volume root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RelativePath {
    components: Vec<String>,
}

impl RelativePath {
    /// Parses an RDPDR path without resolving it in the host filesystem.
    ///
    /// A single leading backslash is the protocol's root-relative spelling and
    /// is stripped before later handle-relative opens. It is not passed to a
    /// Win32 path join operation.
    pub(crate) fn parse(path: &str) -> Result<Self, PathPolicyError> {
        if path.encode_utf16().count() > MAX_PATH_CODE_UNITS {
            return Err(PathPolicyError::TooLong);
        }

        if path.starts_with('/') || path.starts_with(r"\\") {
            return Err(PathPolicyError::Rooted);
        }

        let path = path.strip_prefix('\\').unwrap_or(path);
        if path.is_empty() {
            return Ok(Self { components: Vec::new() });
        }

        let components = path.split('\\').collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            if index + 1 == components.len() {
                validate_final_component(component)?;
            } else {
                validate_filename(component)?;
            }
        }

        Ok(Self {
            components: components.into_iter().map(str::to_owned).collect(),
        })
    }

    pub(crate) fn components(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.components.iter().map(String::as_str)
    }
}

/// A rejected RDPDR path category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathPolicyError {
    /// The path uses a rooted or namespace form rather than a volume-relative path.
    Rooted,
    /// A path component is empty, a traversal marker, or contains an invalid character.
    InvalidComponent,
    /// A component is a Windows DOS device alias.
    ReservedDevice,
    /// The path or a component exceeds the bounded native namespace policy.
    TooLong,
}

fn validate_final_component(component: &str) -> Result<(), PathPolicyError> {
    let mut components = component.split(':');
    let file_name = components.next().expect("split always returns an initial component");
    validate_filename(file_name)?;

    let Some(stream_name) = components.next() else {
        return Ok(());
    };
    let stream_type = components.next();
    if components.next().is_some() {
        return Err(PathPolicyError::InvalidComponent);
    }
    if stream_name.is_empty() && stream_type.is_none() {
        return Err(PathPolicyError::InvalidComponent);
    }
    validate_stream_name(stream_name)?;
    if let Some(stream_type) = stream_type {
        if stream_type.is_empty() {
            return Err(PathPolicyError::InvalidComponent);
        }
        validate_stream_type(stream_type)?;
    }

    Ok(())
}

fn validate_filename(component: &str) -> Result<(), PathPolicyError> {
    if component.is_empty() || matches!(component, "." | "..") {
        return Err(PathPolicyError::InvalidComponent);
    }
    if component.encode_utf16().count() > MAX_COMPONENT_CODE_UNITS {
        return Err(PathPolicyError::TooLong);
    }
    if component.ends_with([' ', '.']) {
        return Err(PathPolicyError::InvalidComponent);
    }
    if component.chars().any(|character| {
        character <= '\u{1f}' || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return Err(PathPolicyError::InvalidComponent);
    }
    if is_reserved_device_name(component) {
        return Err(PathPolicyError::ReservedDevice);
    }

    Ok(())
}

fn validate_stream_name(component: &str) -> Result<(), PathPolicyError> {
    if component.encode_utf16().count() > MAX_COMPONENT_CODE_UNITS {
        return Err(PathPolicyError::TooLong);
    }
    if component
        .chars()
        .any(|character| matches!(character, '\0' | '\\' | '/' | ':'))
    {
        return Err(PathPolicyError::InvalidComponent);
    }

    Ok(())
}

fn validate_stream_type(component: &str) -> Result<(), PathPolicyError> {
    if component
        .chars()
        .any(|character| matches!(character, '\0' | '\\' | '/' | ':'))
    {
        return Err(PathPolicyError::InvalidComponent);
    }

    Ok(())
}

fn is_reserved_device_name(component: &str) -> bool {
    let base_name = component.split('.').next().unwrap_or_default();

    base_name.eq_ignore_ascii_case("CON")
        || base_name.eq_ignore_ascii_case("NUL")
        || base_name.eq_ignore_ascii_case("AUX")
        || base_name.eq_ignore_ascii_case("PRN")
        || base_name.eq_ignore_ascii_case("CLOCK$")
        || is_numbered_device(base_name, "COM")
        || is_numbered_device(base_name, "LPT")
}

fn is_numbered_device(component: &str, prefix: &str) -> bool {
    let mut component = component.chars();

    if !prefix.chars().all(|expected| {
        component
            .next()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(&expected))
    }) {
        return false;
    }

    matches!(component.next(), Some('1'..='9' | '\u{00B9}' | '\u{00B2}' | '\u{00B3}')) && component.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_protocol_root_relative_paths() {
        let path = RelativePath::parse(r"\Documents\report.txt").expect("valid RDPDR path");

        assert_eq!(path.components().collect::<Vec<_>>(), ["Documents", "report.txt"]);
    }

    #[test]
    fn accepts_the_redirected_volume_root() {
        let path = RelativePath::parse(r"\").expect("redirected root");

        assert!(path.components().next().is_none());
    }

    #[test]
    fn rejects_rooted_host_paths_and_traversal() {
        for path in [
            r"\\server\share",
            r"\??\C:\outside",
            r"C:\outside",
            r"..\outside",
            r"one\\two",
        ] {
            assert!(matches!(
                RelativePath::parse(path),
                Err(PathPolicyError::Rooted | PathPolicyError::InvalidComponent)
            ));
        }
    }

    #[test]
    fn rejects_dos_device_aliases_in_every_component() {
        for path in [
            "CON",
            "con.txt",
            "CON:stream",
            r"folder\LPT9",
            "COM\u{00B9}",
            "AUX",
            "clock$",
        ] {
            assert_eq!(RelativePath::parse(path), Err(PathPolicyError::ReservedDevice));
        }
    }

    #[test]
    fn rejects_aliasing_and_invalid_characters() {
        for path in ["name. ", "name.", "file:", "file:stream:", "embedded\0nul", "one/two"] {
            assert_eq!(RelativePath::parse(path), Err(PathPolicyError::InvalidComponent));
        }
    }

    #[test]
    fn accepts_alternate_data_streams_on_the_final_component_only() {
        let path = RelativePath::parse(r"\folder\file:Zone.Identifier").expect("valid alternate data stream pathname");
        assert_eq!(
            path.components().collect::<Vec<_>>(),
            ["folder", "file:Zone.Identifier"]
        );
        assert!(RelativePath::parse(r"\file::$DATA").is_ok());
        assert_eq!(
            RelativePath::parse(r"\folder:stream\file"),
            Err(PathPolicyError::InvalidComponent)
        );
        assert_eq!(
            RelativePath::parse(r"\file:stream:type:extra"),
            Err(PathPolicyError::InvalidComponent)
        );
    }

    #[test]
    fn accepts_non_ascii_components_without_panicking() {
        assert!(RelativePath::parse("CO\u{80}").is_ok());
    }
}
