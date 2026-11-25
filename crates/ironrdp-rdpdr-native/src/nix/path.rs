//! Server-supplied RDPDR filesystem paths, resolved beneath the root of a drive.
//!
//! The server names files with paths relative to the root of the redirected drive, the directory that drive serves.
//! Each path is split into validated components and opened one component at a time relative to a directory descriptor.
//! Symbolic links and mount points are refused, so no spelling, link or mount inside the root resolves outside it.
//!
//! The path of a root is local configuration, so `Root::resolve` follows the symbolic links in it, and reports the directories in which it followed a link or stepped back up through `..`, which `DriveRoots::refuse_roots_a_server_could_redirect` checks against the directories of the configured roots.
//!
//! UNTESTED: every filesystem call on a server-supplied path works on a descriptor from `Root::open_at`,
//! `RelativePath::open_parent` or `RelativePath::open_directory`, never on a joined path string.
//! A path string is resolved again by the kernel, so a symbolic link swapped in after validation would lead outside the root.
//! A new operation resolves its parent with `open_parent` and opens with `Root::open_at`, or uses an `*at` system call that does not follow the final component.
//!
//! UNTESTED: `Root::open_at` refuses a descriptor on another mount.
//! The tests cannot create mounts, so after changing it, check by hand with a bind mount and a tmpfs mounted below a root, with and without `openat2`.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsStr;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;

use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, readlinkat};
use nix::sys::stat::{FileStat, Mode, SFlag, dev_t, fstat, fstatat};

/// Flags for every directory descriptor taken while walking a path.
const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY.union(OFlag::O_DIRECTORY);

/// Opens a directory to search it without reading it.
///
/// The kernel needs no more of a directory a path leads through, so a root beneath a directory that cannot be read still resolves.
#[cfg(target_os = "linux")]
const SEARCH: OFlag = OFlag::O_PATH;
#[cfg(target_os = "macos")]
const SEARCH: OFlag = OFlag::from_bits_retain(nix::libc::O_SEARCH);

/// Flags for each directory opened on the way to a root, which never follow a symbolic link.
const WALK_FLAGS: OFlag = SEARCH
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

/// The most symbolic links followed while resolving the path of a root, as many as Linux follows in one path.
const MAX_LINKS: usize = 40;

/// The most components walked while resolving the path of a root, those of followed links included.
const MAX_COMPONENTS: usize = 1_024;

/// Identifies a file independently of its name, so a rename does not lose track of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FileIdentity {
    device: dev_t,
    inode: nix::libc::ino_t,
}

impl FileIdentity {
    pub(crate) fn of(stat: &FileStat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }
}

/// The opened root of a drive, which every server-supplied path for that drive resolves beneath.
#[derive(Debug)]
pub(crate) struct Root {
    directory: OwnedFd,
    /// The directory itself, which names the root however its path is spelled, and the filesystem it lies on.
    identity: FileIdentity,
}

/// The directory that the path a root was configured with leads to.
#[derive(Debug)]
pub(crate) struct ResolvedRoot {
    /// The directory reached, as the descriptor the walk reached it with identifies it.
    pub(crate) identity: FileIdentity,
    /// The directory opened to be read, or the error that kept it from being opened so.
    ///
    /// A directory needs only to be searched to be reached, so one that cannot be read is known all the same.
    pub(crate) root: nix::Result<Root>,
    /// The directory in which each symbolic link was followed or each `..` step taken on the way to the root, and every directory above it up to `/`.
    ///
    /// Whoever can rename entries in one of these directories can change which directory the path names.
    pub(crate) deciding_directories: HashSet<FileIdentity>,
}

impl Root {
    /// Opens the directory at `path` as the root of a drive, as the kernel resolves the path.
    ///
    /// Only the shared directory of a backend whose drives all serve it is opened so, for each request.
    pub(crate) fn open(path: &str) -> nix::Result<Self> {
        let directory = nix::fcntl::open(path, DIRECTORY_FLAGS | OFlag::O_CLOEXEC, Mode::empty())?;
        let identity = FileIdentity::of(&fstat(directory.as_fd())?);
        Ok(Self { directory, identity })
    }

    /// Resolves `path` to the directory it names and opens it, reporting the directories in which it followed a symbolic link or took a `..` step.
    ///
    /// `path` must be absolute and hold no `.` or `..` component; empty components are skipped.
    /// It is walked one component at a time from `/`, and a symbolic link is read and followed here rather than by the kernel, so the directory holding it is known.
    /// A relative target continues from that directory, an absolute one starts again at `/`, and a `..` in a target leads to the parent of the directory reached, as when the kernel resolves the path.
    /// At most `MAX_LINKS` links are followed and `MAX_COMPONENTS` components walked, so a tree built for it cannot make the walk run long.
    /// No failure is retried another way, and [`names_no_directory`] tells whether one shows that the path names no directory.
    /// A directory that is reached but cannot be opened to be read is no failure of the walk: it is returned with the error in place of the root, so the other roots can still be checked against it.
    ///
    /// Each link followed and each `..` step taken is reported with the directory it happens in and every directory above it, which `DriveRoots::refuse_roots_a_server_could_redirect` checks against the directories of the configured roots.
    ///
    /// UNTESTED: no component is entered or followed on the strength of an earlier look at it.
    /// `fstatat` only chooses between opening a directory and reading a link, and each of those fails on anything else: the open takes `O_NOFOLLOW | O_DIRECTORY`, and `readlinkat` reads nothing but a link.
    /// Every identity is taken from an opened descriptor, never from `fstatat`.
    /// Otherwise a component replaced between the two calls would be followed without the directory holding it being reported, so a root could be reached through a link below another root.
    /// The tests cannot replace a component between two system calls, so after changing the walk, check that it handles no error by trying something else and takes no identity from a path.
    ///
    /// UNTESTED: the walk holds two descriptors at most, the directory reached and the one being opened.
    /// It runs while the backend is configured, so any more would come out of the descriptors `max_open_files` leaves to the rest of the process.
    /// The tests share one process, where descriptors cannot be counted reliably, so after changing the walk, check that it keeps no descriptor for the directories it has left.
    pub(crate) fn resolve(path: &str) -> nix::Result<ResolvedRoot> {
        if !path.starts_with('/') {
            return Err(Errno::EINVAL);
        }
        let mut pending = VecDeque::new();
        for component in path.split('/').filter(|component| !component.is_empty()) {
            if matches!(component, "." | "..") {
                return Err(Errno::EINVAL);
            }
            pending.push_back(component.as_bytes().to_vec());
        }

        let (mut directory, identity) = open_filesystem_root()?;
        // INVARIANT: `chain` holds the directories from `/` down to `directory`, so it is never empty and ends with the identity of `directory`.
        let mut chain = vec![identity];
        let mut deciding_directories = HashSet::new();
        let mut links = 0_usize;
        let mut walked = 0_usize;
        while let Some(component) = pending.pop_front() {
            walked = walked.saturating_add(1);
            if MAX_COMPONENTS < walked {
                return Err(Errno::ENAMETOOLONG);
            }
            if component == b".." {
                // The step is reported for the directory it is taken in, which `chain` is about to lose.
                deciding_directories.extend(chain.iter().copied());
                let parent = nix::fcntl::openat(directory.as_fd(), "..", WALK_FLAGS, Mode::empty())?;
                if 1 < chain.len() {
                    chain.pop();
                }
                // UNTESTED: a parent that is not the directory walked through fails the walk, as the parent of a directory moved since it was entered.
                // `chain` would otherwise name directories the walk is no longer below, and a link followed or a `..` step taken next would be reported in the wrong ones.
                // The tests cannot move a directory during the walk, so after changing this, check that `chain` is compared with the parent actually opened.
                if chain.last() != Some(&FileIdentity::of(&fstat(parent.as_fd())?)) {
                    return Err(Errno::ESTALE);
                }
                directory = parent;
                continue;
            }
            let name = OsStr::from_bytes(&component);
            let stat = fstatat(directory.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
            match SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT {
                SFlag::S_IFDIR => {
                    directory = nix::fcntl::openat(directory.as_fd(), name, WALK_FLAGS, Mode::empty())?;
                    chain.push(FileIdentity::of(&fstat(directory.as_fd())?));
                }
                SFlag::S_IFLNK => {
                    links = links.saturating_add(1);
                    if MAX_LINKS < links {
                        return Err(Errno::ELOOP);
                    }
                    let target = readlinkat(directory.as_fd(), name)?;
                    deciding_directories.extend(chain.iter().copied());
                    let target = target.as_bytes();
                    if target.is_empty() {
                        return Err(Errno::ENOENT);
                    }
                    if target.starts_with(b"/") {
                        let (root, identity) = open_filesystem_root()?;
                        directory = root;
                        chain = vec![identity];
                    }
                    let components = target
                        .split(|byte| *byte == b'/')
                        .filter(|component| !component.is_empty() && *component != b".");
                    for (index, component) in components.enumerate() {
                        pending.insert(index, component.to_vec());
                    }
                    if MAX_COMPONENTS < pending.len() {
                        return Err(Errno::ENAMETOOLONG);
                    }
                }
                _ => return Err(Errno::ENOTDIR),
            }
        }

        // `chain` is never empty, so the walk always has the directory it reached to report.
        let Some(&identity) = chain.last() else {
            return Err(Errno::ESTALE);
        };
        return Ok(ResolvedRoot {
            identity,
            root: open_to_read(directory.as_fd(), identity),
            deciding_directories,
        });

        fn open_filesystem_root() -> nix::Result<(OwnedFd, FileIdentity)> {
            let directory = nix::fcntl::open("/", WALK_FLAGS, Mode::empty())?;
            let identity = FileIdentity::of(&fstat(directory.as_fd())?);
            Ok((directory, identity))
        }

        /// Opens `reached`, the directory the walk ended at, which it opened only to search, to be read as a root is.
        fn open_to_read(reached: BorrowedFd<'_>, identity: FileIdentity) -> nix::Result<Root> {
            let directory = nix::fcntl::openat(
                reached,
                ".",
                DIRECTORY_FLAGS | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )?;
            if FileIdentity::of(&fstat(directory.as_fd())?) != identity {
                return Err(Errno::ESTALE);
            }
            Ok(Root { directory, identity })
        }
    }

    /// The directory this root is, however its path is spelled.
    pub(crate) fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Opens `name` in `parent`, a directory beneath this root.
    ///
    /// Refuses a symbolic link and a mount point, so the result stays inside the root.
    /// Without `openat2`, as on macOS and Linux before 5.6, only a mount of another filesystem is refused, not a bind mount within the same one.
    /// Never blocks, so a FIFO in the root cannot stall the client.
    /// Takes no `O_TRUNC`: without `openat2` it would empty a file on another mount before the device check refuses it, so callers truncate the result instead.
    pub(crate) fn open_at(&self, parent: BorrowedFd<'_>, name: &str, flags: OFlag, mode: Mode) -> nix::Result<OwnedFd> {
        debug_assert!(!flags.contains(OFlag::O_TRUNC), "callers truncate after the open");
        let flags = flags | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;

        // `openat2` refuses every mount point, bind mounts included.
        // The device is not compared when it succeeds, because a Btrfs subvolume or an overlay file reports another device without being a mount.
        #[cfg(target_os = "linux")]
        if openat2::is_available(self.directory.as_fd()) {
            // `openat2` rejects a nonzero mode without `O_CREAT`.
            let mode = if flags.contains(OFlag::O_CREAT) {
                mode
            } else {
                Mode::empty()
            };
            return nix::fcntl::openat2(parent, name, openat2::how(flags).mode(mode));
        }

        let file = nix::fcntl::openat(parent, name, flags, mode)?;
        if fstat(file.as_fd())?.st_dev != self.identity.device {
            return Err(Errno::EXDEV);
        }
        Ok(file)
    }
}

/// Whether `error`, with which [`Root::resolve`] failed, shows that the path names no directory a drive could serve.
///
/// Each error listed says that the path names nothing: a missing component, a component that is not a directory, a loop or the bounds of the walk, or a path of the wrong form.
/// A server cannot bring one about for a root that lies in no other root of its backend, because it can change nothing on the path of such a root.
/// For a root that lies in another root it can, but whatever it changed through either then lies in or below the directory of the enclosing root, which the other roots are checked against.
/// Either way, no directory that a server was served goes unchecked because the path now names none.
///
/// Any other error says nothing of the path: the process ran out of descriptors or memory, the filesystem failed, a directory moved during the walk, or a directory on the path may not be searched.
/// The path may then name a directory that an earlier backend built from the same configuration served, as one beneath a directory that could be searched then, and nothing shows which.
/// The errors of the first kind are listed rather than those of the second, so an error that nobody foresaw leaves the directory undetermined.
///
/// UNTESTED: an error that is not listed leaves the directory undetermined.
/// The backend then refuses every root reached through a symbolic link or a `..` step, and would otherwise serve one reached through a link below the directory it does not know.
/// The tests try a sample of such errors, not every errno of every platform, so after changing the list, check that it names only errors that show the path names no directory and that every other error still falls outside it.
pub(crate) fn names_no_directory(error: Errno) -> bool {
    matches!(
        error,
        Errno::ENOENT | Errno::ENOTDIR | Errno::ELOOP | Errno::EINVAL | Errno::ENAMETOOLONG
    )
}

#[cfg(target_os = "linux")]
mod openat2 {
    use std::os::fd::BorrowedFd;
    use std::sync::OnceLock;

    use nix::errno::Errno;
    use nix::fcntl::{OFlag, OpenHow, ResolveFlag};

    use super::DIRECTORY_FLAGS;

    pub(super) fn how(flags: OFlag) -> OpenHow {
        OpenHow::new()
            .flags(flags)
            .resolve(ResolveFlag::RESOLVE_NO_XDEV | ResolveFlag::RESOLVE_NO_SYMLINKS)
    }

    /// Whether the process can call `openat2`, probed once by opening `directory` itself.
    ///
    /// Kernels before 5.6 lack it, and some sandboxes deny it with EPERM.
    /// That is a property of the process, so once the probe succeeds, an error from `openat2` is the answer for that path and never falls back to the weaker device check.
    ///
    /// UNTESTED: an `openat2` error after a successful probe is returned, never retried with `openat`.
    /// A retry would let a bind mount inside the root through for any path whose open fails with EPERM, such as an immutable file opened for writing.
    /// The tests cannot make `openat2` fail, so after changing this, check that `Root::open_at` returns the `openat2` result unchanged.
    pub(super) fn is_available(directory: BorrowedFd<'_>) -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();

        *AVAILABLE.get_or_init(|| match nix::fcntl::openat2(directory, ".", how(DIRECTORY_FLAGS | OFlag::O_CLOEXEC)) {
            Err(error @ (Errno::ENOSYS | Errno::EPERM)) => {
                tracing::warn!(%error, "openat2 is unavailable, so drive redirection refuses mounts of other filesystems only");
                false
            }
            _ => true,
        })
    }
}

/// The longest name Windows accepts, in UTF-16 code units.
const MAX_NAME_CODE_UNITS: usize = 255;

/// A validated path relative to the root of a drive.
///
/// INVARIANT: every component is a name Windows can hold, as [`is_valid_name`] checks, so none is empty, `.` or `..`, or contains `/` or NUL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RelativePath {
    components: Vec<String>,
}

impl RelativePath {
    /// Parses an RDPDR path without touching the host filesystem.
    ///
    /// A single leading backslash is the protocol's root-relative spelling and is stripped.
    /// An empty remainder names the root itself.
    pub(crate) fn parse(path: &str) -> Result<Self, InvalidPath> {
        Self::parse_components(path.strip_prefix('\\').unwrap_or(path))
    }

    /// Parses a query-directory path into the directory it names and its final component, the pattern to match.
    ///
    /// The pattern may hold the wildcards `*`, `?` and the DOS wildcards `<`, `>` and `"` ([MS-FSA] 2.1.5.6.3 Directory Information Queries), and an empty path enumerates the root.
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/fa8194e0-53ec-413b-8315-e8fa85396fd8
    pub(crate) fn parse_query(path: &str) -> Result<(Self, &str), InvalidPath> {
        let path = path.strip_prefix('\\').unwrap_or(path);
        let (directory, pattern) = if path.is_empty() {
            ("", "*")
        } else {
            path.rsplit_once('\\').unwrap_or(("", path))
        };
        let is_valid_pattern = !matches!(pattern, "" | "." | "..")
            && pattern.encode_utf16().count() <= MAX_NAME_CODE_UNITS
            && !pattern
                .chars()
                .any(|character| character <= '\u{1f}' || matches!(character, ':' | '/' | '|'));
        if !is_valid_pattern {
            return Err(InvalidPath);
        }
        Ok((Self::parse_components(directory)?, pattern))
    }

    fn parse_components(path: &str) -> Result<Self, InvalidPath> {
        if path.is_empty() {
            return Ok(Self { components: Vec::new() });
        }

        let components = path
            .split('\\')
            .map(|component| {
                is_valid_name(component)
                    .then(|| component.to_owned())
                    .ok_or(InvalidPath)
            })
            .collect::<Result<_, _>>()?;

        Ok(Self { components })
    }

    /// The final component, or `None` for the root itself.
    pub(crate) fn file_name(&self) -> Option<&str> {
        self.components.last().map(String::as_str)
    }

    /// Opens the directory that holds the final component, and returns it with that component.
    ///
    /// For the root itself, the directory is the root and the component is `.`.
    pub(crate) fn open_parent(&self, root: &Root) -> nix::Result<(OwnedFd, &str)> {
        let Some((file_name, parents)) = self.components.split_last() else {
            return Ok((root.directory.try_clone().map_err(io_to_errno)?, "."));
        };

        let parent = open_directories(root, parents, None)?;
        Ok((parent, file_name))
    }

    /// Opens this path as a directory to read.
    ///
    /// The root itself is opened anew, because a duplicate of a root held between requests shares its read position, so a second listing would start where the first ended.
    pub(crate) fn open_directory(&self, root: &Root) -> nix::Result<OwnedFd> {
        if self.components.is_empty() {
            return root.open_at(root.directory.as_fd(), ".", DIRECTORY_FLAGS, Mode::empty());
        }
        open_directories(root, &self.components, None)
    }

    /// Like [`Self::open_parent`], and also returns the status of each directory opened on the way, from the root down to the one returned.
    ///
    /// The root itself lies in no directory, so for it the list is empty.
    pub(crate) fn open_parent_with_ancestors(&self, root: &Root) -> nix::Result<(OwnedFd, &str, Vec<FileStat>)> {
        let Some((file_name, parents)) = self.components.split_last() else {
            return Ok((root.directory.try_clone().map_err(io_to_errno)?, ".", Vec::new()));
        };

        let mut ancestors = Vec::with_capacity(self.components.len());
        let parent = open_directories(root, parents, Some(&mut ancestors))?;
        Ok((parent, file_name, ancestors))
    }
}

/// Whether Windows can create a file named `name`, so the server cannot reach a file it could not have created.
///
/// [MS-FSCC] 2.1.5 Pathname forbids the characters and lengths checked here, and a trailing dot or space is refused because Win32 strips it, as the Windows backend does.
/// Unlike there, `:` never introduces a data stream, which Unix files lack, and DOS device names such as `CON` are ordinary names.
///
/// [MS-FSCC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ffb795f3-027d-4a3c-997d-3085f2332f6f
fn is_valid_name(name: &str) -> bool {
    !matches!(name, "" | "." | "..")
        && name.encode_utf16().count() <= MAX_NAME_CODE_UNITS
        && !name.ends_with([' ', '.'])
        && !name.chars().any(|character| {
            character <= '\u{1f}' || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
}

/// A path the backend refuses to resolve beneath the root of a drive.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvalidPath;

fn open_directories(
    root: &Root,
    components: &[String],
    mut ancestors: Option<&mut Vec<FileStat>>,
) -> nix::Result<OwnedFd> {
    let mut directory = root.directory.try_clone().map_err(io_to_errno)?;
    if let Some(ancestors) = &mut ancestors {
        ancestors.push(fstat(directory.as_fd())?);
    }
    for component in components {
        directory = root.open_at(directory.as_fd(), component, DIRECTORY_FLAGS, Mode::empty())?;
        if let Some(ancestors) = &mut ancestors {
            ancestors.push(fstat(directory.as_fd())?);
        }
    }
    Ok(directory)
}

pub(crate) fn io_to_errno(error: std::io::Error) -> Errno {
    error.raw_os_error().map_or(Errno::EIO, Errno::from_raw)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
    #[rstest]
    #[case::empty("", &[])]
    #[case::single_backslash(r"\", &[])]
    #[case::root_relative(r"\Documents\report.txt", &["Documents", "report.txt"])]
    #[case::without_leading_backslash(r"Documents\report.txt", &["Documents", "report.txt"])]
    #[case::leading_dot(r"\.hidden", &[".hidden"])]
    // Unix has no DOS device aliases, so unlike on the Windows backend, these name ordinary files.
    // oracle: mutant DOS device names refused
    #[case::device_names(r"\CON\aux.c", &["CON", "aux.c"])]
    fn accepts_protocol_root_relative_paths(#[case] path: &str, #[case] components: &[&str]) {
        let path = RelativePath::parse(path).expect("valid RDPDR path");

        assert_eq!(path.components, components);
    }

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
    #[rstest]
    #[case::parent(r"\..")]
    #[case::parents_then_name(r"\..\..\.bashrc")]
    #[case::parents_below_a_directory(r"\dir\..\..\outside")]
    #[case::current_directory(r"\.\file")]
    #[case::unc_prefix(r"\\server\share")]
    #[case::empty_component(r"\dir\\file")]
    #[case::trailing_backslash(r"\dir\")]
    #[case::forward_slash_root(r"\/etc/passwd")]
    #[case::forward_slash_traversal(r"\dir/../../outside")]
    #[case::embedded_nul("\\embedded\0nul")]
    #[case::parent_without_leading_backslash(r"..\outside")]
    #[case::trailing_current_directory(r"\dir\.")]
    #[case::trailing_parent(r"\dir\..")]
    fn rejects_traversal_rooted_and_empty_components(#[case] path: &str) {
        assert_eq!(RelativePath::parse(path), Err(InvalidPath));
    }

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/directory.rs
    #[rstest]
    #[case::wildcard_in_the_share(r"\*", &[], "*")]
    #[case::empty_path_enumerates_the_share("", &[], "*")]
    #[case::wildcard_in_a_directory(r"\folder\*", &["folder"], "*")]
    #[case::single_name(r"\folder\report.txt", &["folder"], "report.txt")]
    fn splits_query_paths_into_a_directory_and_a_pattern(
        #[case] path: &str,
        #[case] directory: &[&str],
        #[case] pattern: &str,
    ) {
        let (parsed, parsed_pattern) = RelativePath::parse_query(path).expect("valid query path");

        assert_eq!(parsed.components, directory);
        assert_eq!(parsed_pattern, pattern);
    }

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/directory.rs
    #[rstest]
    #[case::parent_directory(r"\..\*")]
    #[case::unc_prefix(r"\\server\share")]
    #[case::forward_slash("outside/anything")]
    #[case::empty_pattern(r"\folder\")]
    #[case::dot_pattern(r"\folder\.")]
    #[case::invalid_pattern_character(r"\a|b")]
    #[case::invalid_directory_name(r"\a*b\*")]
    fn rejects_invalid_query_paths(#[case] path: &str) {
        assert_eq!(RelativePath::parse_query(path), Err(InvalidPath));
    }

    // oracle: mutant an error that shows the path names no directory left out of the list
    #[rstest]
    #[case::missing(Errno::ENOENT)]
    #[case::not_a_directory(Errno::ENOTDIR)]
    #[case::too_many_links(Errno::ELOOP)]
    #[case::malformed(Errno::EINVAL)]
    #[case::too_many_components(Errno::ENAMETOOLONG)]
    fn an_error_of_the_path_itself_shows_that_it_names_no_directory(#[case] error: Errno) {
        assert!(names_no_directory(error));
    }

    // oracle: mutant an error that says nothing of the path taken to show that it names no directory
    #[rstest]
    #[case::no_permission_to_search_a_directory(Errno::EACCES)]
    #[case::no_descriptor_left_to_the_process(Errno::EMFILE)]
    #[case::no_descriptor_left_to_the_system(Errno::ENFILE)]
    #[case::out_of_memory(Errno::ENOMEM)]
    #[case::input_output_error(Errno::EIO)]
    #[case::directory_moved_during_the_walk(Errno::ESTALE)]
    #[case::bad_descriptor(Errno::EBADF)]
    fn an_error_that_says_nothing_of_the_path_does_not_show_that_it_names_no_directory(#[case] error: Errno) {
        assert!(!names_no_directory(error));
    }
}
