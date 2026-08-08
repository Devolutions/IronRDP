//! Validation for server-supplied RDPDR filesystem paths.
//!
//! [MS-RDPEFS] 2.2.3.3.1 identifies the drive through the device ID, so a
//! server-supplied path must name only a descendant of the selected volume.
//! This module validates that namespace before native handle-relative opens.
//!
//! [MS-RDPEFS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs

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
    /// RDPDR represents the selected volume root by an empty path or one
    /// leading backslash. The leading separator is protocol syntax only; it is
    /// never passed to a Win32 path join operation.
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
        for component in &components {
            validate_component(component)?;
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
    /// A path component is a Windows DOS device alias.
    ReservedDevice,
    /// The path or a component exceeds the bounded native namespace policy.
    TooLong,
}

fn validate_component(component: &str) -> Result<(), PathPolicyError> {
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
        assert!(
            RelativePath::parse(r"\")
                .expect("redirected root")
                .components()
                .next()
                .is_none()
        );
    }

    #[test]
    fn rejects_host_namespace_paths_traversal_and_streams() {
        for path in [
            r"\\server\share",
            r"\??\C:\outside",
            r"C:\outside",
            r"..\outside",
            r"one\\two",
            "file:stream",
        ] {
            assert!(matches!(
                RelativePath::parse(path),
                Err(PathPolicyError::Rooted | PathPolicyError::InvalidComponent)
            ));
        }
    }

    #[test]
    fn rejects_dos_device_aliases_in_every_component() {
        for path in ["CON", "con.txt", r"folder\LPT9", "COM\u{00B9}", "AUX", "clock$"] {
            assert_eq!(RelativePath::parse(path), Err(PathPolicyError::ReservedDevice));
        }
    }
}
