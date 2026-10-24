//! Server-supplied RDPDR filesystem paths, resolved beneath the shared directory.
//!
//! The server names files with paths relative to the redirected drive root.
//! Each path is split into validated components and opened one component at a
//! time relative to a directory descriptor, refusing symbolic links, so no
//! spelling or link inside the share resolves outside it.
//!
//! UNTESTED: every filesystem call on a server-supplied path works on a descriptor from `open_parent` or
//! `open_directory`, never on a joined path string.
//! A path string is resolved again by the kernel, so a symbolic link swapped in after validation would lead outside the share.
//! A new operation resolves its parent with `open_parent` and uses the `*at` system call with `O_NOFOLLOW` or its equivalent.

use std::os::fd::{AsFd, OwnedFd};

use nix::fcntl::{OFlag, openat};
use nix::sys::stat::Mode;

/// Flags for every directory descriptor taken while walking a path.
const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

/// A validated path relative to the shared directory.
///
/// INVARIANT: no component is empty, `.` or `..`, or contains `/` or NUL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RelativePath {
    components: Vec<String>,
}

impl RelativePath {
    /// Parses an RDPDR path without touching the host filesystem.
    ///
    /// A single leading backslash is the protocol's root-relative spelling and is stripped.
    /// An empty remainder names the shared directory itself.
    pub(crate) fn parse(path: &str) -> Result<Self, InvalidPath> {
        let path = path.strip_prefix('\\').unwrap_or(path);
        if path.is_empty() {
            return Ok(Self { components: Vec::new() });
        }

        let components = path
            .split('\\')
            .map(|component| {
                let is_valid = !matches!(component, "" | "." | "..") && !component.contains(['/', '\0']);
                is_valid.then(|| component.to_owned()).ok_or(InvalidPath)
            })
            .collect::<Result<_, _>>()?;

        Ok(Self { components })
    }

    /// The final component, or `None` for the shared directory itself.
    pub(crate) fn file_name(&self) -> Option<&str> {
        self.components.last().map(String::as_str)
    }

    /// The path without its final component, or `None` for the shared directory itself.
    pub(crate) fn parent(&self) -> Option<Self> {
        let (_, parent) = self.components.split_last()?;
        Some(Self {
            components: parent.to_vec(),
        })
    }

    /// Opens the directory that holds the final component, and returns it with that component.
    ///
    /// For the shared directory itself, the directory is `root` and the component is `.`.
    pub(crate) fn open_parent(&self, root: &OwnedFd) -> nix::Result<(OwnedFd, &str)> {
        let Some((file_name, parents)) = self.components.split_last() else {
            return Ok((root.try_clone().map_err(io_to_errno)?, "."));
        };

        let parent = open_directories(root, parents)?;
        Ok((parent, file_name))
    }

    /// Opens this path as a directory.
    pub(crate) fn open_directory(&self, root: &OwnedFd) -> nix::Result<OwnedFd> {
        open_directories(root, &self.components)
    }
}

/// A path the backend refuses to resolve beneath the shared directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidPath;

/// Opens the shared directory, the anchor every server-supplied path resolves beneath.
///
/// The shared directory is configured locally, so symbolic links in its own path are followed.
pub(crate) fn open_root(file_base: &str) -> nix::Result<OwnedFd> {
    nix::fcntl::open(
        file_base,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
}

fn open_directories(root: &OwnedFd, components: &[String]) -> nix::Result<OwnedFd> {
    let mut directory = root.try_clone().map_err(io_to_errno)?;
    for component in components {
        directory = openat(directory.as_fd(), component.as_str(), DIRECTORY_FLAGS, Mode::empty())?;
    }
    Ok(directory)
}

pub(crate) fn io_to_errno(error: std::io::Error) -> nix::errno::Errno {
    error
        .raw_os_error()
        .map_or(nix::errno::Errno::EIO, nix::errno::Errno::from_raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_protocol_root_relative_paths() {
        let path = RelativePath::parse(r"\Documents\report.txt").expect("valid RDPDR path");

        assert_eq!(path.components, ["Documents", "report.txt"]);
    }

    #[test]
    fn accepts_the_shared_directory_itself() {
        for path in ["", r"\"] {
            let path = RelativePath::parse(path).expect("shared directory");

            assert!(path.file_name().is_none());
        }
    }

    #[test]
    fn rejects_traversal_rooted_and_empty_components() {
        for path in [
            r"\..",
            r"\..\..\.bashrc",
            r"\dir\..\..\outside",
            r"\.\file",
            r"\\server\share",
            r"\dir\\file",
            r"\dir\",
            r"\/etc/passwd",
            r"\dir/../../outside",
            "\\embedded\0nul",
        ] {
            assert_eq!(RelativePath::parse(path), Err(InvalidPath), "{path:?}");
        }
    }
}
