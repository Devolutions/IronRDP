//! Server-supplied RDPDR filesystem paths, resolved beneath the shared directory.
//!
//! The server names files with paths relative to the redirected drive root.
//! Each path is split into validated components and opened one component at a
//! time relative to a directory descriptor, refusing symbolic links and mount
//! points, so no spelling, link or mount inside the share resolves outside it.
//!
//! UNTESTED: every filesystem call on a server-supplied path works on a descriptor from `Root::open_at`,
//! `RelativePath::open_parent` or `RelativePath::open_directory`, never on a joined path string.
//! A path string is resolved again by the kernel, so a symbolic link swapped in after validation would lead outside the share.
//! A new operation resolves its parent with `open_parent` and opens with `Root::open_at`, or uses an `*at` system call that does not follow the final component.
//!
//! UNTESTED: `Root::open_at` refuses a descriptor on another mount.
//! The tests cannot create mounts, so after changing it, check by hand with a bind mount and a tmpfs mounted below a share, with and without `openat2`.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::{Mode, dev_t, fstat};

/// Flags for every directory descriptor taken while walking a path.
const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY.union(OFlag::O_DIRECTORY);

/// The opened shared directory, which every server-supplied path resolves beneath.
#[derive(Debug)]
pub(crate) struct Root {
    directory: OwnedFd,
    /// The filesystem holding the shared directory.
    device: dev_t,
}

impl Root {
    /// Opens the shared directory.
    ///
    /// The shared directory is configured locally, so symbolic links in its own path are followed.
    pub(crate) fn open(file_base: &str) -> nix::Result<Self> {
        let directory = nix::fcntl::open(
            file_base,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
        let device = fstat(directory.as_fd())?.st_dev;
        Ok(Self { directory, device })
    }

    /// Opens `name` in `parent`, a directory beneath this root.
    ///
    /// Refuses a symbolic link and a mount point, so the result stays inside the share.
    /// Without `openat2`, as on macOS and Linux before 5.6, only a mount of another filesystem is refused, not a bind mount within the same one.
    /// Never blocks, so a FIFO in the share cannot stall the client.
    pub(crate) fn open_at(&self, parent: BorrowedFd<'_>, name: &str, flags: OFlag, mode: Mode) -> nix::Result<OwnedFd> {
        let flags = flags | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
        let file = openat_same_mount(parent, name, flags, mode)?;
        // Refuses a mount of another filesystem where `openat2` is unavailable.
        if fstat(file.as_fd())?.st_dev != self.device {
            return Err(Errno::EXDEV);
        }
        Ok(file)
    }
}

/// Opens `name` in `parent` without crossing a mount point, bind mounts included.
#[cfg(target_os = "linux")]
fn openat_same_mount(parent: BorrowedFd<'_>, name: &str, flags: OFlag, mode: Mode) -> nix::Result<OwnedFd> {
    use nix::fcntl::{OpenHow, ResolveFlag, openat2};

    // `openat2` rejects a nonzero mode without `O_CREAT`.
    let mode = if flags.contains(OFlag::O_CREAT) {
        mode
    } else {
        Mode::empty()
    };
    let how = OpenHow::new()
        .flags(flags)
        .mode(mode)
        .resolve(ResolveFlag::RESOLVE_NO_XDEV | ResolveFlag::RESOLVE_NO_SYMLINKS);
    match openat2(parent, name, how) {
        // Kernels before 5.6 lack `openat2`, and some sandboxes deny it.
        Err(Errno::ENOSYS | Errno::EPERM) => nix::fcntl::openat(parent, name, flags, mode),
        result => result,
    }
}

/// Opens `name` in `parent`; the caller rejects a descriptor on another filesystem.
#[cfg(not(target_os = "linux"))]
fn openat_same_mount(parent: BorrowedFd<'_>, name: &str, flags: OFlag, mode: Mode) -> nix::Result<OwnedFd> {
    nix::fcntl::openat(parent, name, flags, mode)
}

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
    /// For the shared directory itself, the directory is the root and the component is `.`.
    pub(crate) fn open_parent(&self, root: &Root) -> nix::Result<(OwnedFd, &str)> {
        let Some((file_name, parents)) = self.components.split_last() else {
            return Ok((root.directory.try_clone().map_err(io_to_errno)?, "."));
        };

        let parent = open_directories(root, parents)?;
        Ok((parent, file_name))
    }

    /// Opens this path as a directory.
    pub(crate) fn open_directory(&self, root: &Root) -> nix::Result<OwnedFd> {
        open_directories(root, &self.components)
    }
}

/// A path the backend refuses to resolve beneath the shared directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidPath;

fn open_directories(root: &Root, components: &[String]) -> nix::Result<OwnedFd> {
    let mut directory = root.directory.try_clone().map_err(io_to_errno)?;
    for component in components {
        directory = root.open_at(directory.as_fd(), component, DIRECTORY_FLAGS, Mode::empty())?;
    }
    Ok(directory)
}

pub(crate) fn io_to_errno(error: std::io::Error) -> Errno {
    error.raw_os_error().map_or(Errno::EIO, Errno::from_raw)
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
