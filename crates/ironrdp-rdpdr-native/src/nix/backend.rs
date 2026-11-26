use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::{
    RdpdrBackend, RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive, RdpdrPrinter,
};
use ironrdp_svc::SvcMessage;
use nix::dir::{Dir, OwningIter};
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, renameat};
use nix::sys::resource::{Resource, getrlimit};
use nix::sys::stat::{FileStat, Mode, SFlag, fstat, fstatat, mkdirat};
use nix::unistd::{UnlinkatFlags, ftruncate, unlinkat};
use tracing::{debug, trace, warn};

use super::locks;
use super::locks::{ByteRangeLocks, file_lock_conflict};
pub(crate) use super::path::FileIdentity;
use super::path::{InvalidPath, RelativePath, ResolvedRoot, Root, io_to_errno, names_no_directory};
use super::printer::{PrintTarget, PrinterSpooler, unsupported_response, write_response};
use super::wildcard::{contains_wildcards, is_name_in_expression};

/// The most files the server may hold open at once, however high the descriptor limit is.
const MAX_OPEN_FILES: usize = 1_024;

/// The largest read the server may request, so a single request cannot make the client allocate gigabytes.
const MAX_READ_LENGTH: usize = 1_024 * 1_024;

/// The most names kept for one file marked for deletion, however many roots it was marked through.
const MAX_PENDING_DELETE_NAMES: usize = 4;

/// Serves redirected drives, and optionally a printer, on macOS and Linux.
///
/// Each drive serves one directory, its root: the shared directory given to [`Self::new`], or a directory of its own given to [`Self::with_drive_root`].
/// Server-supplied paths are resolved beneath the root of the drive the request addresses and never follow symbolic links, so the server cannot reach files outside it.
///
/// A file ID names a file only through the drive it was opened through.
/// Through another drive it is answered like a file ID that was never opened, and the request changes nothing.
///
/// The limits on open files and byte-range locks apply to all drives together.
///
/// [Drive roots on macOS and Linux](crate#drive-roots-on-macos-and-linux) describes how roots are resolved, when a root is unusable, and what holds between drives that reach the same files.
#[derive(Debug)]
pub struct NixRdpdrBackend {
    file_id: u32,
    roots: DriveRoots,
    /// The open files of every drive, by file ID.
    ///
    /// INVARIANT: a file is added only while the entries and the roots that hold their directory open together number fewer than `max_open_files`.
    open_files: HashMap<u32, OpenFile>,
    /// Bounds the open files and the roots that hold their directory open together, so the server cannot exhaust the descriptors of the client process.
    ///
    /// A quarter of the soft descriptor limit, at most `MAX_OPEN_FILES`.
    /// An open file holds up to three descriptors: the file, and a directory being enumerated with its stream.
    /// A root that holds its directory open holds one, and takes the place of one open file.
    /// A quarter of the limit therefore leaves at least a quarter of the descriptors to the rest of the process.
    /// The roots take at most half the places, as [`DriveRoots::record_root`] bounds them, so at least half are left to open files.
    max_open_files: usize,
    /// Files to delete when their last open handle closes ([MS-FSA] 2.1.5.5 Server Requests Closing an Open), each with the names it was marked through, from the one marked first to the one marked last.
    ///
    /// A marked name is removed rather than the path of whichever handle closes last, so a file with several hard links loses a link the server marked.
    /// [MS-FSA] marks each link separately, while a file loses one name here, and every link of a marked file refuses new opens.
    /// While every drive serves the shared directory, one name is kept per file: marking through another link or another drive replaces it.
    /// Once a drive has a root of its own, one name is kept for each root the file was marked through, because the root it was marked through last may be unable to remove a file that another root can.
    /// A mark then replaces only the name marked before through the same root, as [`DriveRoots::use_the_same_root`] decides, and becomes the name marked last.
    /// A file that already has `MAX_PENDING_DELETE_NAMES` names of other roots loses the one marked first, so it keeps the names marked last.
    /// The names are tried from the last to the first when the last handle closes, and the first that removes the file ends the delete.
    ///
    /// INVARIANT: a file is kept only with a name, so a file is marked exactly while it has an entry.
    ///
    /// INVARIANT: a file has at most `MAX_PENDING_DELETE_NAMES` names, each of a different root: at most one of the shared directory, and one of each drive with a root of its own.
    /// One open handle keeps every name of its file, each with a path and the directories on it, so a name for every root would let the server keep as many names as there are roots for each file it holds open.
    /// A marked file has an open handle, so the names of all files together number at most `MAX_PENDING_DELETE_NAMES` times the limit on open files.
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/d142c93a-72bc-4b05-9d96-8e00371c3308
    pending_deletes: HashMap<FileIdentity, Vec<PendingDelete>>,
    /// The locks on the files of every drive, so one drive sees the locks taken through another on the same file.
    ///
    /// UNTESTED: one table holds the locks of every drive, so its limit on held locks bounds them together.
    /// A table for each drive would let the server hold that many locks for each drive it is given.
    /// Filling the table takes seconds, so the tests show only that the limit on waiting requests is shared; after splitting the table, check the limit on held locks across two drives by hand.
    locks: ByteRangeLocks,
    /// Print-job handling for the virtual printer, when one is announced.
    printer: Option<Printer>,
}

impl Default for NixRdpdrBackend {
    /// Creates a backend without a shared directory, which serves only the drives given a root with [`Self::with_drive_root`].
    fn default() -> Self {
        Self::new(String::new())
    }
}

/// The directories the drives serve.
///
/// While no drive has a root of its own, every drive serves the shared directory, which is opened by path for each request.
/// From the first such root on, every root is resolved once and holds the directory it names open, the shared directory included.
/// Roots that name the same directory each hold it open, through their own path.
/// A root beyond the bound on roots held open, which [`Self::record_root`] sets, is resolved too, but holds nothing open, like a root whose directory cannot be read.
///
/// UNTESTED: a root that holds its directory open keeps one descriptor between requests, the one in its `directory`.
/// A root that did not resolve keeps none, like the shared directory while no drive has a root of its own.
/// Neither does a root whose directory could not be opened to be read or one beyond the bound on roots held open, whose walks close their descriptors before the root is stored.
/// `max_open_files` counts one open file for each root that holds its directory open, so any other descriptor kept for a root would come out of the quarter it leaves to the rest of the process.
/// The tests count the places taken under the limit, not descriptors, which cannot be counted reliably in the one process they share.
/// After changing how roots are opened or held, check that each root that holds its directory open keeps exactly the descriptor in its `directory`, and that a root that cannot be read or lies beyond the bound keeps none.
#[derive(Debug)]
struct DriveRoots {
    /// The path of the shared directory, which is the root of every device without a root of its own, or empty when there is none.
    ///
    /// Opening an empty path fails, so such a device is served nothing rather than the working directory.
    shared_path: String,
    /// The shared directory as resolved when the first drive was given a root of its own, or `None` before that and when there is no shared directory.
    shared: Option<ConfiguredRoot>,
    /// The root of each device that does not serve the shared directory.
    drives: HashMap<u32, ConfiguredRoot>,
}

/// A root as its path resolved when it was configured.
#[derive(Debug)]
struct ConfiguredRoot {
    /// The path the root was configured with, which is only logged once it is resolved.
    path: String,
    /// The directory the path named, as far as its walk showed.
    ///
    /// A root that does not hold its directory open has one too, because the paths of the other roots are checked against it all the same.
    identity: RootIdentity,
    /// The directory the path named, opened through that path, or `None` when the walk failed, the directory could not be opened to be read, or the root lies beyond the bound on roots held open.
    ///
    /// A refused root keeps its directory open too, because replacing the root that refuses it makes it usable again without resolving its path again.
    directory: Option<Root>,
    /// The directories whose entries decide what the path names, because a symbolic link was followed or a `..` step taken in or below them.
    deciding_directories: HashSet<FileIdentity>,
    /// Whether a server could have changed what the path names.
    ///
    /// One of `deciding_directories` is the directory of a configured root, or `deciding_directories` is not empty while the directory of a configured root is undetermined.
    is_refused: bool,
}

/// The directory a root path names, as far as its walk showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootIdentity {
    /// The path named this directory.
    Known(FileIdentity),
    /// The walk showed that the path names no directory a drive could serve, as [`names_no_directory`] tells.
    NoDirectory,
    /// The walk failed without showing which directory the path names, or whether it names one.
    ///
    /// The directory may exist all the same, and an earlier backend built from the same configuration may have served it.
    Undetermined,
}

impl DriveRoots {
    /// Whether no drive has a root of its own, so every drive serves the shared directory, which is opened by path for each request.
    fn serve_only_the_shared_directory(&self) -> bool {
        self.drives.is_empty()
    }

    /// The root of `device_id`, which paths in requests for that device resolve beneath.
    ///
    /// Once a drive has a root of its own, this resolves no path, and fails for a device that is served nothing.
    fn of(&self, device_id: u32) -> nix::Result<RootRef<'_>> {
        if self.serve_only_the_shared_directory() {
            return Root::open(&self.shared_path).map(RootRef::Opened);
        }
        self.held(device_id).map(RootRef::Held).ok_or(Errno::ENOENT)
    }

    /// The directory `device_id` serves, as its own root holds it open, once a drive has a root of its own.
    ///
    /// UNTESTED: the requests of a drive use the descriptor that was opened through the root path of that drive, never the descriptor of another root that names the same directory.
    /// A directory can be reached through several mounts, such as a read-only bind mount and its writable source, which report the same device and inode.
    /// A descriptor shared between such roots would let a drive configured through the read-only mount write through the writable one.
    /// The tests cannot create mounts, so after changing how descriptors are held, check that a drive given a read-only bind mount of the root of another drive refuses writes.
    fn held(&self, device_id: u32) -> Option<&Root> {
        let root = self.drives.get(&device_id).or(self.shared.as_ref())?;
        root.directory.as_ref().filter(|_| !root.is_refused)
    }

    /// Whether `first` and `second` serve the same directory, so a path beneath the root of one names the same file beneath the root of the other.
    fn serve_the_same_directory(&self, first: u32, second: u32) -> bool {
        self.serve_only_the_shared_directory()
            || self
                .held(first)
                .is_some_and(|root| self.held(second).map(Root::identity) == Some(root.identity()))
    }

    /// Whether `first` and `second` use the same root, so a name beneath it removes a file through one exactly when it does through the other.
    ///
    /// Every device without a root of its own uses the one root of the shared directory.
    /// Devices with roots of their own use different roots even when those name the same directory, which each holds open through its own path.
    fn use_the_same_root(&self, first: u32, second: u32) -> bool {
        first == second || !self.drives.contains_key(&first) && !self.drives.contains_key(&second)
    }

    /// How many roots hold their directory open, each of which takes the place of one open file.
    fn held_count(&self) -> usize {
        self.shared
            .iter()
            .chain(self.drives.values())
            .filter(|root| root.directory.is_some())
            .count()
    }

    /// Resolves `path` as the root of `device_id` and holds the directory it names from now on, in place of an earlier root of that device.
    ///
    /// The shared directory is resolved and held with the first such root.
    /// `max_open_files` is the limit of the backend on open files, which bounds the roots held open as [`Self::record_root`] describes.
    fn set_drive_root(&mut self, max_open_files: usize, device_id: u32, path: &str) {
        if self.shared.is_none() && !self.shared_path.is_empty() {
            let shared_path = self.shared_path.clone();
            self.record_root(max_open_files, None, &shared_path, Root::resolve(&shared_path));
        }
        // The earlier root of the device is released first, so the root that replaces it can take its place.
        self.drives.remove(&device_id);
        self.record_root(max_open_files, Some(device_id), path, Root::resolve(path));
    }

    /// Records `walk`, the outcome of resolving `path`, as the root of `device_id`, and judges every root again.
    ///
    /// The outcome is given rather than taken here, so a test can give one that no path produces on demand.
    /// A walk that failed leaves the root unusable, and its directory undetermined unless the failure shows that `path` names none, as [`names_no_directory`] tells.
    /// A walk that reached a directory it could not open to be read leaves the root unusable too, but its directory known.
    /// A server may have been served that directory while it could be read, so the paths of the other roots are checked against it as against any other.
    /// A root that resolved holds the directory `path` names open.
    /// The roots hold at most half of `max_open_files` directories open, so at least half the limit is left to open files however many roots are given.
    /// A root beyond that is recorded all the same, but holds nothing open and is unusable.
    /// Its directory has to be known, because a server may have been served it by an earlier backend built from the same configuration, in which a root given before it named no directory yet and took no place.
    /// The paths of the other roots are therefore checked against it as against any other.
    /// It stays unusable when a place is freed later, because a root is resolved only when it is given.
    /// `device_id` is `None` for the shared directory.
    fn record_root(
        &mut self,
        max_open_files: usize,
        device_id: Option<u32>,
        path: &str,
        walk: nix::Result<ResolvedRoot>,
    ) {
        let max_held_roots = max_open_files / 2;
        let (identity, directory, deciding_directories) = match walk {
            Ok(ResolvedRoot {
                identity,
                root: Ok(root),
                deciding_directories,
            }) => {
                let is_held = self.held_count() < max_held_roots;
                if !is_held {
                    warn!(
                        device_id,
                        root = path,
                        max_held_roots,
                        "Reached the limit on drive roots held open, so nothing is served from this root"
                    );
                }
                // A root that is not held is dropped here, which closes the last descriptor of its walk.
                (
                    RootIdentity::Known(identity),
                    is_held.then_some(root),
                    deciding_directories,
                )
            }
            // The walk reached the directory, so it is known although nothing can be served from it.
            Ok(ResolvedRoot {
                identity,
                root: Err(error),
                deciding_directories,
            }) => {
                warn!(
                    %error,
                    device_id,
                    root = path,
                    "Failed to open the directory of a drive root to read it, so nothing is served from it"
                );
                (RootIdentity::Known(identity), None, deciding_directories)
            }
            Err(error) if names_no_directory(error) => {
                warn!(%error, device_id, root = path, "Failed to resolve a drive root, so nothing is served from it");
                (RootIdentity::NoDirectory, None, HashSet::new())
            }
            // Any other failure says nothing of the path, which may name a directory that an earlier backend served.
            // The other roots cannot be checked against a directory that is not known, so while this root is configured, each of them that followed a link or took a `..` step is refused.
            Err(error) => {
                warn!(
                    %error,
                    device_id,
                    root = path,
                    "Could not determine the directory of a drive root, so nothing is served from it or from a root whose path follows a symbolic link or steps back up"
                );
                (RootIdentity::Undetermined, None, HashSet::new())
            }
        };
        let root = ConfiguredRoot {
            path: path.to_owned(),
            identity,
            directory,
            deciding_directories,
            is_refused: false,
        };
        match device_id {
            Some(device_id) => {
                self.drives.insert(device_id, root);
            }
            None => self.shared = Some(root),
        }
        self.refuse_roots_a_server_could_redirect();
    }

    /// Refuses every root whose path followed a symbolic link or took a `..` step in or below the directory of a configured root, its own included.
    ///
    /// In a directory that a drive serves, and below it, a server decides what every entry is.
    /// A root path that only descends from there ends at or below that directory whatever the server does.
    /// A link read there, or a step back up from there, makes the end of the path depend on those entries, so the next time it is resolved it can lie anywhere.
    /// A root is therefore usable only if its path did nothing but descend once it was in or below a configured root.
    ///
    /// The check needs the directory of every root a server may have been served.
    /// While the directory of a configured root is undetermined, a link followed or a `..` step taken anywhere may lie in or below it.
    /// Every root whose path did either is then refused too, wherever it did.
    /// A root whose path only descended from `/` stays usable, because it ends in or below every directory it passed through, the undetermined one included.
    ///
    /// Every root is judged again against all of them, refused ones included, so the outcome does not depend on the order the roots were given in.
    ///
    /// The refusal while a directory is undetermined is tested with failures given to `record_root`, and in `tests/nix_root_descriptors.rs` with walks that run out of descriptors, in a process that test has to itself.
    ///
    /// UNTESTED: a link or a `..` step counts as lying below a root only when the directories walked from `/` to it include the directory of that root.
    /// A bind mount, or on macOS a firmlink such as `/Users` for `/System/Volumes/Data/Users`, shows a directory inside a root at another path.
    /// A link there has other directories above it, so a root reached through that link is not refused, although a server can replace the link.
    /// The tests cannot create mounts or firmlinks, so the rule assumes there is none; after changing it, check by hand what a root given through a link below a bind mount of another root is served.
    ///
    /// UNTESTED: a link or a `..` step is checked against the roots of this backend only.
    /// A link in a directory that another backend redirects under another configuration, before or beside this one, is followed like any link in local configuration, although the server of that backend can replace it.
    /// No test runs two configurations against one tree, so after changing which directories a link is checked against, check that a root path outside the roots of the backend is still resolved as the kernel resolves it.
    fn refuse_roots_a_server_could_redirect(&mut self) {
        let configured = || self.shared.iter().chain(self.drives.values());
        let directories = configured()
            .filter_map(|root| match root.identity {
                RootIdentity::Known(identity) => Some(identity),
                RootIdentity::NoDirectory | RootIdentity::Undetermined => None,
            })
            .collect::<HashSet<_>>();
        let is_any_undetermined = configured().any(|root| root.identity == RootIdentity::Undetermined);
        let roots = self
            .shared
            .iter_mut()
            .map(|root| (None, root))
            .chain(self.drives.iter_mut().map(|(device_id, root)| (Some(*device_id), root)));
        for (device_id, root) in roots {
            let is_redirectable = !root.deciding_directories.is_disjoint(&directories);
            // A directory that is not known cannot be told from the ones in `deciding_directories`, so it counts as one of them.
            let may_be_redirectable = is_any_undetermined && !root.deciding_directories.is_empty();
            let is_refused = is_redirectable || may_be_redirectable;
            if is_refused && !root.is_refused {
                if is_redirectable {
                    warn!(
                        device_id,
                        root = root.path,
                        "Refused a drive root whose path follows a symbolic link or steps back up inside a redirected directory, so nothing is served from it"
                    );
                } else {
                    warn!(
                        device_id,
                        root = root.path,
                        "Refused a drive root whose path follows a symbolic link or steps back up while the directory of another root is not determined, so nothing is served from it"
                    );
                }
            }
            root.is_refused = is_refused;
        }
    }
}

/// The root a request resolves beneath.
enum RootRef<'a> {
    /// A root held since it was configured.
    Held(&'a Root),
    /// The shared directory, opened for this request.
    Opened(Root),
}

impl core::ops::Deref for RootRef<'_> {
    type Target = Root;

    fn deref(&self) -> &Root {
        match self {
            Self::Held(root) => root,
            Self::Opened(root) => root,
        }
    }
}

#[derive(Debug)]
struct OpenFile {
    /// The device the file was opened through, whose root `path` is relative to.
    device_id: u32,
    file: std::fs::File,
    path: RelativePath,
    identity: FileIdentity,
    /// The directories from the root of the drive down to the one holding this file, as the server last reached it.
    ///
    /// A directory holding open files is found by these rather than by path, so another spelling of a name or a local rename does not hide one.
    ancestors: Vec<FileIdentity>,
    /// Whether the handle was granted DELETE access, which renaming and marking for deletion need.
    may_delete: bool,
    /// Whether closing this handle marks the file for deletion, as FILE_DELETE_ON_CLOSE requests.
    delete_on_close: bool,
    /// The enumeration the last initial query-directory request started on this file.
    listing: Option<DirectoryListing>,
}

/// A name a file marked for deletion can be removed by: the one marked last through a root.
#[derive(Debug)]
struct PendingDelete {
    /// The device the file was marked through, whose root `path` is relative to.
    device_id: u32,
    path: RelativePath,
    /// The directories `path` led through when the file was marked, from the root of `device_id` down.
    ///
    /// INVARIANT: once a drive has a root of its own, the requests a server sends rename none of them while the delete is pending, so `path` goes on naming the file.
    ancestors: Vec<FileIdentity>,
}

#[derive(Debug)]
struct DirectoryListing {
    /// The enumerated directory, which entry names are resolved against.
    directory: OwnedFd,
    /// `.` and `..`, still to be returned first, which every directory but the root lists ([MS-FSA] 2.1.5.6.3 Directory Information Queries).
    /// The pair is listed together, when `.` matches the pattern.
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/fa8194e0-53ec-413b-8315-e8fa85396fd8
    dot_entries: &'static [&'static str],
    entries: OwningIter,
    /// The pattern entry names must match, or `None` for every entry.
    pattern: Option<String>,
}

impl DirectoryListing {
    fn open(directory: OwnedFd, is_root: bool, pattern: Option<String>) -> nix::Result<Self> {
        let entries = Dir::from_fd(directory.try_clone().map_err(io_to_errno)?)?.into_iter();
        let lists_dot_entries = !is_root
            && pattern
                .as_deref()
                .is_none_or(|pattern| is_name_in_expression(".", pattern));
        Ok(Self {
            directory,
            dot_entries: if lists_dot_entries { &[".", ".."] } else { &[] },
            entries,
            pattern,
        })
    }

    /// Returns the next entry matching the pattern, or `None` once enumeration ends, skipping entries removed since it started.
    fn next_entry(&mut self) -> nix::Result<Option<DirectoryEntry>> {
        let pattern = self.pattern.as_deref();
        let is_match = |name: &str| pattern.is_none_or(|pattern| is_name_in_expression(name, pattern));
        if let Some((name, rest)) = self.dot_entries.split_first() {
            self.dot_entries = rest;
            let stat = fstatat(self.directory.as_fd(), *name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
            return Ok(Some(DirectoryEntry {
                name: (*name).to_owned(),
                stat,
            }));
        }
        for entry in self.entries.by_ref() {
            let entry = entry?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            let lossy_name = name.to_string_lossy();
            if !is_match(&lossy_name) {
                continue;
            }
            // UNTESTED: only an entry removed since enumeration started is skipped, and any other error fails the listing.
            // Skipping an entry that cannot be inspected, as in a directory the process may not search, would make the directory look empty, so a delete could be marked on it.
            // The tests can run as root, which no permission stops, so after changing this, check that only ENOENT is skipped.
            let stat = match fstatat(self.directory.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(Errno::ENOENT) => continue,
                Err(error) => return Err(error),
            };
            return Ok(Some(DirectoryEntry {
                name: lossy_name.into_owned(),
                stat,
            }));
        }
        Ok(None)
    }
}

struct DirectoryEntry {
    name: String,
    /// The entry itself, not the target of a symbolic link.
    stat: FileStat,
}

#[derive(Debug)]
struct Printer {
    device_id: u32,
    spooler: PrinterSpooler,
}

impl NixRdpdrBackend {
    /// Creates a backend that serves drive requests from the `file_base` directory.
    ///
    /// Server-supplied paths are resolved beneath `file_base` and never follow symbolic links, so the server cannot reach files outside it.
    ///
    /// `file_base` is the shared directory: the root of every drive not given its own with [`Self::with_drive_root`].
    /// It is opened by path for each request and never checked, so it may be relative, or missing until a request needs it.
    /// Once a drive is given a root of its own, `file_base` is resolved and held as that root is, and has to be usable as a root is.
    /// An empty `file_base` is no directory, so creates through such a drive fail as they do when the shared directory cannot be opened.
    pub fn new(file_base: String) -> Self {
        Self {
            file_id: 0,
            roots: DriveRoots {
                shared_path: file_base,
                shared: None,
                drives: HashMap::new(),
            },
            open_files: HashMap::new(),
            max_open_files: match getrlimit(Resource::RLIMIT_NOFILE) {
                Ok((soft_limit, _)) => usize::try_from(soft_limit / 4)
                    .unwrap_or(usize::MAX)
                    .min(MAX_OPEN_FILES),
                Err(error) => {
                    warn!(%error, "Failed to read the descriptor limit");
                    MAX_OPEN_FILES
                }
            },
            pending_deletes: HashMap::new(),
            locks: ByteRangeLocks::default(),
            printer: None,
        }
    }

    /// Closes `file_id`, deleting the file when it is marked for deletion and no other handle holds it.
    ///
    /// Returns the responses to lock requests that closing the file cancels or lets proceed.
    fn close_file(&mut self, file_id: u32) -> Vec<SvcMessage> {
        let responses = self.locks.close(file_id);
        let Some(open_file) = self.open_files.remove(&file_id) else {
            return responses;
        };
        // [MS-FSA] 2.1.5.5 marks a file opened with FILE_DELETE_ON_CLOSE for deletion when that handle closes, but a directory only while it is empty.
        if open_file.delete_on_close {
            match holds_entries(&open_file.file) {
                Ok(false) => {
                    mark_for_deletion(
                        &self.roots,
                        &mut self.pending_deletes,
                        open_file.identity,
                        PendingDelete {
                            device_id: open_file.device_id,
                            path: open_file.path,
                            ancestors: open_file.ancestors,
                        },
                    );
                }
                Ok(true) => trace!("Kept a directory opened with delete on close that is not empty"),
                Err(error) => warn!(%error, "Delete on close check error"),
            }
        }
        let still_open = self
            .open_files
            .values()
            .any(|other| other.identity == open_file.identity);
        if !still_open && let Some(mut names) = self.pending_deletes.remove(&open_file.identity) {
            // The name marked last is tried first, and an earlier one only after every name marked since failed, so the file loses one name.
            // Each name is resolved beneath the root of its own device, and removed only while it still names this file.
            //
            // UNTESTED: a name that cannot be removed beneath the root of its device leaves the delete to the names marked before it through other roots.
            // One directory can be the root of a drive through a read-only mount and of another drive through a writable one, and a file marked through both has to be deleted even when the read-only drive marked it last.
            // The tests cannot create mounts, so they make a name fail by renaming a directory on its path locally, which fails before the name would be unlinked.
            // After changing this loop, check that a file marked through a writable root and then through a read-only bind mount of it is removed when its last handle closes.
            //
            // UNTESTED: a file that no name removes logs one warning, and a failed name with earlier ones left to try is only traced.
            // The server decides how many drives it marks a file through, so a warning for each name would let it fill the log.
            // No test reads the log, so after changing this loop, check that only the failure of the last name tried warns.
            while let Some(name) = names.pop() {
                let removed = self
                    .roots
                    .of(name.device_id)
                    .and_then(|root| remove_file(&root, &name.path, open_file.identity));
                match removed {
                    Ok(()) => break,
                    Err(error) if !names.is_empty() => {
                        trace!(%error, device_id = name.device_id, "Failed to delete a file by one of its marked names");
                    }
                    Err(error) => warn!(%error, device_id = name.device_id, "Delete on close error"),
                }
            }
        }
        responses
    }

    /// Returns an ID no open file holds, counting up and wrapping around after the last one.
    ///
    /// At most `max_open_files` files are open, far fewer than there are IDs, so the search ends.
    fn next_file_id(&mut self) -> u32 {
        loop {
            let file_id = self.file_id;
            self.file_id = self.file_id.wrapping_add(1);
            if !self.open_files.contains_key(&file_id) {
                return file_id;
            }
        }
    }

    /// Closes every open file as if the server had closed it, so pending deletes run.
    fn close_all_files(&mut self) {
        let file_ids = self.open_files.keys().copied().collect::<Vec<_>>();
        for file_id in file_ids {
            self.close_file(file_id);
        }
    }

    /// Serves the drive announced as `device_id` from the `root` directory instead of the shared directory.
    ///
    /// `root` is resolved here, once, and the directory it names is held open until the backend is dropped, so every root has to be given before the backend serves a request.
    /// The drive then serves that directory whatever becomes of its path.
    /// `root` must be an absolute path without `.` or `..` components, and may lead through symbolic links that lie outside every root of the backend.
    /// A `..` in the target of such a link must be taken outside every root as well.
    /// A root reached through a link is unusable all the same while the directory of another root could not be determined.
    /// An unusable root, as [Drive roots on macOS and Linux](crate#drive-roots-on-macos-and-linux) defines it, is logged and leaves the drive configured but served nothing.
    /// Roots are checked against each other in whichever order they are given, so a root given later can make an earlier one unusable.
    /// A root given again for the same device replaces the earlier one.
    #[must_use]
    pub fn with_drive_root(mut self, device_id: u32, root: String) -> Self {
        // UNTESTED: every root is resolved before the backend serves its first request, because the factory gives the roots before it returns the backend.
        // A root resolved between requests would be walked through whatever a server had made of its path through an enclosing drive, and could race requests through that drive.
        // Nothing stops a caller that holds the backend from giving a root later, so when adding a way to configure roots, check that it runs before the backend is handed to the channel.
        self.roots.set_drive_root(self.max_open_files, device_id, &root);
        self
    }

    /// Accepts print jobs for the virtual printer announced as `device_id` and sends them to
    /// `target`.
    #[must_use]
    pub fn with_printer(mut self, device_id: u32, target: PrintTarget) -> Self {
        self.printer = Some(Printer {
            device_id,
            spooler: PrinterSpooler::new(target),
        });
        self
    }
}

impl Drop for NixRdpdrBackend {
    /// The session is over, so the handles the server left open are closed and their pending deletes run.
    fn drop(&mut self) {
        self.close_all_files();
    }
}

impl_as_any!(NixRdpdrBackend);

/// Builds a [`NixRdpdrBackend`] for every RDPDR channel lifetime.
///
/// `drives` are announced as redirected folders; `printer` announces a virtual
/// printer named after the client, whose jobs go to the given target. Drive
/// device IDs must be unique and nonzero, and the printer takes the highest ID
/// no drive uses; [`RdpdrBackendFactory::build_rdpdr_backend`] reports a
/// configuration that breaks this with a [`NixRdpdrBackendFactoryError`].
///
/// A drive added with [`Self::with_drive`] serves the shared directory given to [`Self::new`], and a drive added with [`Self::with_drive_root`] serves a directory of its own.
/// The default factory has no shared directory, for a configuration where every drive has its own root.
///
/// The roots are resolved each time a backend is built, as [`NixRdpdrBackend::with_drive_root`] describes, and the shared directory with them once a drive has its own root.
/// An unusable root does not fail the build: its drive is announced and served nothing.
/// Without a drive that has its own root, no directory is opened or checked when a backend is built.
#[derive(Debug, Clone, Default)]
pub struct NixRdpdrBackendFactory {
    file_base: String,
    drives: Vec<Drive>,
    printer: Option<(String, PrintTarget)>,
}

#[derive(Debug, Clone)]
struct Drive {
    device_id: u32,
    name: String,
    /// The directory the drive serves, or `None` for the shared directory.
    root: Option<String>,
}

impl NixRdpdrBackendFactory {
    /// Creates a factory whose drives serve the `file_base` directory unless they are given their own root.
    ///
    /// An empty `file_base` is no directory, as for [`NixRdpdrBackend::new`].
    pub fn new(file_base: String) -> Self {
        Self {
            file_base,
            drives: Vec::new(),
            printer: None,
        }
    }

    /// Announces a drive called `name` that serves the shared directory.
    #[must_use]
    pub fn with_drive(mut self, device_id: u32, name: String) -> Self {
        self.drives.push(Drive {
            device_id,
            name,
            root: None,
        });
        self
    }

    /// Announces a drive called `name` that serves the `root` directory instead of the shared directory.
    ///
    /// `root` is resolved when a backend is built and has to be usable as [`NixRdpdrBackend::with_drive_root`] requires, or the drive is served nothing.
    /// Several drives may serve the same directory, or directories inside one another.
    #[must_use]
    pub fn with_drive_root(mut self, device_id: u32, name: String, root: String) -> Self {
        self.drives.push(Drive {
            device_id,
            name,
            root: Some(root),
        });
        self
    }

    #[must_use]
    pub fn with_printer(mut self, name: String, target: PrintTarget) -> Self {
        self.printer = Some((name, target));
        self
    }
}

/// The device ID the virtual printer takes unless a drive uses it.
const DEFAULT_PRINTER_DEVICE_ID: u32 = u32::MAX - 1;

impl RdpdrBackendFactory for NixRdpdrBackendFactory {
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        let mut device_ids = HashSet::with_capacity(self.drives.len());
        for drive in &self.drives {
            if drive.device_id == 0 {
                return Err(Box::new(NixRdpdrBackendFactoryError::ReservedDeviceId));
            }
            if !is_valid_device_name(&drive.name) {
                return Err(Box::new(NixRdpdrBackendFactoryError::InvalidDriveName(drive.device_id)));
            }
            if !device_ids.insert(drive.device_id) {
                return Err(Box::new(NixRdpdrBackendFactoryError::DuplicateDeviceId(
                    drive.device_id,
                )));
            }
        }

        let mut backend = NixRdpdrBackend::new(self.file_base.clone());
        for drive in &self.drives {
            if let Some(root) = &drive.root {
                backend = backend.with_drive_root(drive.device_id, root.clone());
            }
        }
        let mut printer = None;
        if let Some((name, target)) = &self.printer {
            if !is_valid_device_name(name) {
                return Err(Box::new(NixRdpdrBackendFactoryError::InvalidPrinterName));
            }
            let mut device_id = DEFAULT_PRINTER_DEVICE_ID;
            while device_ids.contains(&device_id) {
                device_id = device_id
                    .checked_sub(1)
                    .filter(|device_id| *device_id != 0)
                    .ok_or(NixRdpdrBackendFactoryError::NoPrinterDeviceId)?;
            }
            backend = backend.with_printer(device_id, target.clone());
            printer = Some(RdpdrPrinter::new(
                device_id,
                name.clone(),
                DEFAULT_PRINTER_DRIVER_NAME.to_owned(),
            ));
        }

        let drives = self
            .drives
            .iter()
            .map(|drive| RdpdrDrive::new(drive.device_id, drive.name.clone()))
            .collect();
        let mut product = RdpdrBackendProduct::new(Box::new(backend), drives);
        if let Some(printer) = printer {
            product = product.with_printer(printer);
        }
        Ok(product)
    }
}

/// Device names are announced as NUL-terminated strings: an empty name would reach the server
/// empty, and a NUL cannot be part of one.
fn is_valid_device_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\0')
}

/// Invalid [`NixRdpdrBackendFactory`] configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NixRdpdrBackendFactoryError {
    /// A drive used device ID 0.
    ReservedDeviceId,
    /// More than one drive used the same device ID.
    DuplicateDeviceId(u32),
    /// The drives use every device ID the printer could take.
    NoPrinterDeviceId,
    /// The drive with this device ID has an empty name or one with an embedded NUL.
    InvalidDriveName(u32),
    /// The printer has an empty name or one with an embedded NUL.
    InvalidPrinterName,
}

impl core::fmt::Display for NixRdpdrBackendFactoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ReservedDeviceId => f.write_str("RDPDR device ID 0 is reserved"),
            Self::DuplicateDeviceId(device_id) => write!(f, "duplicate RDPDR device ID {device_id}"),
            Self::NoPrinterDeviceId => f.write_str("no RDPDR device ID is available for the printer"),
            Self::InvalidDriveName(device_id) => {
                write!(
                    f,
                    "RDPDR drive {device_id} has an empty name or one with an embedded NUL"
                )
            }
            Self::InvalidPrinterName => f.write_str("the RDPDR printer has an empty name or one with an embedded NUL"),
        }
    }
}

impl core::error::Error for NixRdpdrBackendFactoryError {}

impl RdpdrBackend for NixRdpdrBackend {
    fn reset(&mut self) -> PduResult<()> {
        if let Some(printer) = self.printer.as_mut() {
            printer.spooler.reset();
        }
        // The server forgets its handles when the channel restarts, so they are closed as if it had closed them, which releases their locks and lock requests.
        self.close_all_files();
        Ok(())
    }
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        if self
            .printer
            .as_ref()
            .is_some_and(|printer| printer.device_id == pdu.device_id)
        {
            if pdu.result_code == NtStatus::SUCCESS {
                tracing::info!("The server accepted the redirected printer");
            } else {
                warn!(?pdu.result_code, "The server rejected the redirected printer");
            }
        }
        Ok(())
    }
    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        match self.printer.as_mut() {
            Some(printer) => printer.spooler.handle(req),
            None => Ok(vec![SvcMessage::from(unsupported_response(req))]),
        }
    }
    fn reject_printer_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        match self.printer.as_mut() {
            Some(printer) => printer.spooler.reject_write(req),
            None => Ok(vec![SvcMessage::from(write_response(req, 0, NtStatus::NOT_SUPPORTED))]),
        }
    }
    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        debug!("handle_drive_io_request:{:?}", req);
        match req {
            ServerDriveIoRequest::DeviceWriteRequest(req_inner) => write_device(self, req_inner),
            ServerDriveIoRequest::ServerCreateDriveRequest(req_inner) => create_drive(self, req_inner),
            ServerDriveIoRequest::DeviceReadRequest(req_inner) => read_device(self, req_inner),
            ServerDriveIoRequest::DeviceCloseRequest(req_inner) => close_device(self, req_inner),
            ServerDriveIoRequest::DeviceFlushBuffersRequest(req_inner) => Ok(vec![SvcMessage::from(
                RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NOT_SUPPORTED),
                }),
            )]),
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_) => {
                // TODO
                Ok(Vec::new())
            }
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(req_inner) => query_directory(self, req_inner),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(req_inner) => query_information(self, req_inner),
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(req_inner) => {
                query_volume_information(self, req_inner)
            }
            ServerDriveIoRequest::ServerDriveSetInformationRequest(req_inner) => set_information(self, req_inner),
            ServerDriveIoRequest::DeviceControlRequest(req_inner) => Ok(vec![SvcMessage::from(
                RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
                    device_io_reply: DeviceIoResponse::new(req_inner.header, NtStatus::SUCCESS),
                    output_buffer: None,
                }),
            )]),
            ServerDriveIoRequest::ServerDriveLockControlRequest(req_inner) => lock_control(self, req_inner),
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(req_inner) => Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NOT_SUPPORTED),
                    security_descriptor: None,
                }),
            )]),
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(req_inner) => {
                let response = ClientDriveSetSecurityResponse::new(&req_inner, NtStatus::NOT_SUPPORTED)
                    .map_err(|error| encode_err!(error))?;
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetSecurityResponse(
                    response,
                ))])
            }
        }
    }
}

pub(crate) fn write_device(backend: &mut NixRdpdrBackend, req_inner: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
    let file_id = req_inner.device_io_request.file_id;
    let length = u64::try_from(req_inner.write_data.len()).unwrap_or(u64::MAX);
    // [MS-FSA] 2.1.5.4 refuses a write into a range another handle locked, or that any shared lock covers, after a write of no bytes has already succeeded.
    if length != 0
        && file_for_request(&backend.open_files, &req_inner.device_io_request).is_some_and(|open_file| {
            backend
                .locks
                .conflicts(open_file.identity, file_id, req_inner.offset, length, true, false)
        })
    {
        return Ok(vec![SvcMessage::from(write_response(
            req_inner.device_io_request,
            0,
            file_lock_conflict(),
        ))]);
    }
    return process_dependent_file(
        backend,
        req_inner.device_io_request,
        |request| {
            let res = RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(request, NtStatus::NO_SUCH_FILE),
                length: 0u32,
            });
            Ok(vec![SvcMessage::from(res)])
        },
        |file, request| match write_inner(file, req_inner.offset, &req_inner.write_data) {
            Ok(length) => {
                if length == req_inner.write_data.len() {
                    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
                        DeviceWriteResponse {
                            device_io_reply: DeviceIoResponse::new(request, NtStatus::SUCCESS),
                            length: u32::try_from(req_inner.write_data.len()).unwrap(),
                        },
                    ))])
                } else {
                    warn!(
                        "Written content len:{} is not equal to {}",
                        length,
                        req_inner.write_data.len()
                    );
                    let res = RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                        device_io_reply: DeviceIoResponse::new(request, NtStatus::UNSUCCESSFUL),
                        length: 0u32,
                    });
                    Ok(vec![SvcMessage::from(res)])
                }
            }
            Err(error) => {
                warn!(%error, "Write error");
                let res = RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                    device_io_reply: DeviceIoResponse::new(request, status_from_errno(io_to_errno(error))),
                    length: 0u32,
                });
                Ok(vec![SvcMessage::from(res)])
            }
        },
    );
    fn write_inner(file: &mut std::fs::File, offset: u64, write_data: &[u8]) -> std::io::Result<usize> {
        let sf = SeekFrom::Start(offset);
        file.seek(sf)?;
        let length = file.write(write_data)?;
        file.flush()?;
        Ok(length)
    }
}

pub(crate) fn read_device(backend: &mut NixRdpdrBackend, req_inner: DeviceReadRequest) -> PduResult<Vec<SvcMessage>> {
    let file_id = req_inner.device_io_request.file_id;
    // [MS-FSA] 2.1.5.3 refuses a read from a range another handle locked exclusively, after a read of no bytes has already succeeded.
    if req_inner.length != 0
        && file_for_request(&backend.open_files, &req_inner.device_io_request).is_some_and(|open_file| {
            backend.locks.conflicts(
                open_file.identity,
                file_id,
                req_inner.offset,
                u64::from(req_inner.length),
                false,
                false,
            )
        })
    {
        let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
            device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, file_lock_conflict()),
            read_data: Vec::new(),
        });
        return Ok(vec![SvcMessage::from(res)]);
    }
    return process_dependent_file(
        backend,
        req_inner.device_io_request,
        |request| {
            let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(request, NtStatus::NO_SUCH_FILE),
                read_data: Vec::new(),
            });
            Ok(vec![SvcMessage::from(res)])
        },
        |file, request| {
            let length = usize::try_from(req_inner.length).unwrap();
            let (status, read_data) = if MAX_READ_LENGTH < length {
                warn!(length, "Rejected an oversized read");
                (NtStatus::INVALID_PARAMETER, Vec::new())
            } else {
                match read_inner(file, req_inner.offset, length) {
                    Ok(buf) => (NtStatus::SUCCESS, buf),
                    Err(error) => {
                        warn!(?error, "Read error");
                        (status_from_errno(io_to_errno(error)), Vec::new())
                    }
                }
            };
            let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(request, status),
                read_data,
            });
            Ok(vec![SvcMessage::from(res)])
        },
    );
    fn read_inner(file: &mut std::fs::File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
        let sf = SeekFrom::Start(offset);
        file.seek(sf)?;
        let mut buf = vec![0; length];

        let length = file.read(&mut buf)?;
        buf.resize(length, 0u8);
        Ok(buf)
    }
}

/// Takes or releases byte-range locks on the file `req_inner` names, as MS-FSA [2.1.5.8] Server Requests a Byte-Range Lock and [2.1.5.9] Server Requests an Unlock of a Byte-Range describe.
///
/// [2.1.5.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/369103ec-a8af-452b-8006-aff07b925b61
/// [2.1.5.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/84e68de6-31a6-4cba-afd4-00b1bac6d1a2
pub(crate) fn lock_control(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveLockControlRequest,
) -> PduResult<Vec<SvcMessage>> {
    let file_id = req_inner.device_io_request.file_id;
    let checked = match file_for_request(&backend.open_files, &req_inner.device_io_request) {
        None => {
            warn!("No such file");
            Err(NtStatus::NO_SUCH_FILE)
        }
        // [MS-FSA] 2.1.5.8 and 2.1.5.9 do not permit byte-range locks on a directory.
        Some(open_file) => match fstat(open_file.file.as_fd()) {
            Ok(stat) if is_directory(&stat) => Err(NtStatus::INVALID_PARAMETER),
            Ok(_) => Ok(open_file.identity),
            Err(error) => Err(status_from_errno(error)),
        },
    };
    let identity = match checked {
        Ok(identity) => identity,
        Err(status) => return Ok(vec![locks::response(req_inner.device_io_request, status)]),
    };
    match req_inner.operation {
        LockOperation::Shared | LockOperation::Exclusive => {
            let exclusive = req_inner.operation == LockOperation::Exclusive;
            let status = backend.locks.lock(
                &req_inner.device_io_request,
                identity,
                &req_inner.locks,
                exclusive,
                req_inner.wait,
            );
            // A waiting request is answered when the conflicting lock goes away.
            Ok(status
                .map(|status| locks::response(req_inner.device_io_request, status))
                .into_iter()
                .collect())
        }
        LockOperation::Unlock | LockOperation::UnlockMultiple => {
            let (status, granted) = backend.locks.unlock(file_id, identity, &req_inner.locks);
            let mut messages = vec![locks::response(req_inner.device_io_request, status)];
            messages.extend(granted);
            Ok(messages)
        }
    }
}

pub(crate) fn close_device(backend: &mut NixRdpdrBackend, req_inner: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
    // A failed delete on close is only logged, and the close still reports success.
    // [MS-FSA] 2.1.5.5 Server Requests Closing an Open defines no failure status, and Windows does not report a failed delete on close to the caller closing the handle either.
    // A file ID that names no file of this device closes nothing, so it cannot close a file of another device.
    let lock_responses = if file_for_request(&backend.open_files, &req_inner.device_io_request).is_some() {
        backend.close_file(req_inner.device_io_request.file_id)
    } else {
        Vec::new()
    };
    let res = RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
    });
    let mut messages = vec![SvcMessage::from(res)];
    messages.extend(lock_responses);
    Ok(messages)
}

pub(crate) fn query_information(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveQueryInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match file_for_request(&backend.open_files, &req_inner.device_io_request) {
        Some(OpenFile {
            file, path, identity, ..
        }) => match file.metadata() {
            Ok(meta) => {
                let name = path.file_name().unwrap_or_default();
                let file_attribute = get_file_attributes(meta.is_dir(), meta.permissions().readonly(), name);
                if FileInformationClassLevel::FILE_BASIC_INFORMATION == req_inner.file_info_class_lvl {
                    let basic_info = FileBasicInformation {
                        creation_time: transform_to_filetime(meta.ctime()),
                        last_access_time: transform_to_filetime(meta.atime()),
                        last_write_time: transform_to_filetime(meta.mtime()),
                        change_time: transform_to_filetime(meta.ctime()),
                        file_attributes: file_attribute,
                    };
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                        buffer: Some(FileInformationClass::Basic(basic_info)),
                    });
                    Ok(vec![SvcMessage::from(res)])
                } else if FileInformationClassLevel::FILE_STANDARD_INFORMATION == req_inner.file_info_class_lvl {
                    let dir = if meta.is_dir() { Boolean::True } else { Boolean::False };
                    let is_delete_pending = backend.pending_deletes.contains_key(identity);
                    let standard_info = FileStandardInformation {
                        allocation_size: i64::try_from(meta.size()).unwrap(),
                        end_of_file: i64::try_from(meta.size()).unwrap(),
                        // A directory has one link, whatever Unix counts for `.` and subdirectories, and the link marked for deletion is not counted ([MS-FSA] 2.1.5.12.27 FileStandardInformation).
                        number_of_links: if meta.is_dir() {
                            1
                        } else {
                            u32::try_from(meta.nlink()).unwrap()
                        }
                        .saturating_sub(u32::from(is_delete_pending)),
                        delete_pending: if is_delete_pending {
                            Boolean::True
                        } else {
                            Boolean::False
                        },
                        directory: dir,
                    };
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                        buffer: Some(FileInformationClass::Standard(standard_info)),
                    });
                    Ok(vec![SvcMessage::from(res)])
                } else if FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION == req_inner.file_info_class_lvl {
                    let info = FileAttributeTagInformation {
                        file_attributes: file_attribute,
                        reparse_tag: 0,
                    };
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                        buffer: Some(FileInformationClass::AttributeTag(info)),
                    });
                    Ok(vec![SvcMessage::from(res)])
                } else {
                    warn!("unsupported file class");
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL),
                        buffer: None,
                    });
                    Ok(vec![SvcMessage::from(res)])
                }
            }
            Err(error) => {
                warn!(?error, "Get file metadata error");
                let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL),
                    buffer: None,
                });
                Ok(vec![SvcMessage::from(res)])
            }
        },
        None => {
            warn!("no such file");
            let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                buffer: None,
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}

pub(crate) fn query_volume_information(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveQueryVolumeInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match file_for_request(&backend.open_files, &req_inner.device_io_request) {
        Some(OpenFile { file, .. }) => {
            if let Ok(statvfs) = nix::sys::statvfs::fstatvfs(file.as_fd()) {
                if FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION == req_inner.fs_info_class_lvl {
                    #[cfg_attr(target_os = "macos", expect(clippy::unnecessary_fallible_conversions))]
                    let info = FileFsFullSizeInformation {
                        total_alloc_units: i64::try_from(statvfs.blocks()).unwrap(),
                        caller_available_alloc_units: i64::try_from(statvfs.blocks_available()).unwrap(),
                        actual_available_alloc_units: i64::try_from(statvfs.blocks_available()).unwrap(),
                        sectors_per_alloc_unit: u32::try_from(statvfs.fragment_size()).unwrap(),
                        bytes_per_sector: 1,
                    };

                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsFullSizeInformation(info)),
                            },
                        ),
                    )])
                } else if FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION == req_inner.fs_info_class_lvl
                {
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsAttributeInformation(
                                    FileFsAttributeInformation {
                                        file_system_attributes: FileSystemAttributes::FILE_CASE_SENSITIVE_SEARCH
                                            | FileSystemAttributes::FILE_CASE_PRESERVED_NAMES
                                            | FileSystemAttributes::FILE_UNICODE_ON_DISK,
                                        max_component_name_len: 260,
                                        file_system_name: "FAT32".to_owned(),
                                    },
                                )),
                            },
                        ),
                    )])
                } else if FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION == req_inner.fs_info_class_lvl {
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsVolumeInformation(
                                    FileFsVolumeInformation {
                                        volume_creation_time: transform_to_filetime(file.metadata().unwrap().ctime()),
                                        // blocks_available() may have different integer type on different platforms.
                                        // so we need to cast it to u32 uniformly. so if it is u32, it will emit 'useless conversion'
                                        // warning, i choose to mute it.
                                        #[expect(
                                            clippy::allow_attributes,
                                            reason = "we have to use allow as the useless_conversion isn't triggered on some platforms"
                                        )]
                                        #[allow(clippy::useless_conversion)]
                                        volume_serial_number: u32::try_from(statvfs.blocks_available()).unwrap(),
                                        supports_objects: Boolean::False,
                                        volume_label: "IRON_RDP".to_owned(),
                                    },
                                )),
                            },
                        ),
                    )])
                } else if FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION == req_inner.fs_info_class_lvl {
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                #[cfg_attr(target_os = "macos", expect(clippy::unnecessary_fallible_conversions))]
                                buffer: Some(FileSystemInformationClass::FileFsSizeInformation(
                                    FileFsSizeInformation {
                                        total_alloc_units: i64::try_from(statvfs.blocks()).unwrap(),
                                        available_alloc_units: i64::try_from(statvfs.blocks_free()).unwrap(),
                                        sectors_per_alloc_unit: u32::try_from(statvfs.fragment_size()).unwrap(),
                                        bytes_per_sector: 1,
                                    },
                                )),
                            },
                        ),
                    )])
                } else {
                    warn!("unsupported volume class");
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(
                                    req_inner.device_io_request,
                                    NtStatus::UNSUCCESSFUL,
                                ),
                                buffer: None,
                            },
                        ),
                    )])
                }
            } else {
                warn!("no such file");
                let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                    buffer: None,
                });
                Ok(vec![SvcMessage::from(res)])
            }
        }
        None => {
            warn!("no such file");
            let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                buffer: None,
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}

pub(crate) fn set_information(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveSetInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    // A path names a file only beneath the root of its drive, so a rename cannot give the new name to a handle of a drive that serves another directory.
    // The file is therefore not renamed while such a drive holds it open, through any of its hard links.
    // A handle on the root of its own drive is no reason to refuse: its path is empty and names that root whatever the directory is called, so the rename leaves it no stale name.
    // Remote file managers keep such handles open, and refusing would keep the enclosing drive from renaming the root of an inner drive while only that root is open.
    let is_open_through_another_directory = matches!(req_inner.set_buffer, FileInformationClass::Rename(_))
        && file_for_request(&backend.open_files, &req_inner.device_io_request).is_some_and(|open_file| {
            backend.open_files.values().any(|other| {
                other.identity == open_file.identity
                    && other.path.file_name().is_some()
                    && !backend
                        .roots
                        .serve_the_same_directory(other.device_id, open_file.device_id)
            })
        });
    // [MS-FSA] 2.1.5.15.12 FileRenameInformation refuses to rename a file marked for deletion or a directory holding open files.
    // A directory that led to a file marked for deletion is refused like one holding open files, because the file is deleted by a name it was marked through, each of which has to go on naming it.
    // Only a backend with a drive that has a root of its own refuses it.
    // With only a shared directory the open handles alone decide, because a handle left on the marked link keeps those directories in place and a handle left on another hard link does not.
    let rename_denied = matches!(req_inner.set_buffer, FileInformationClass::Rename(_))
        && file_for_request(&backend.open_files, &req_inner.device_io_request).is_some_and(|open_file| {
            backend.pending_deletes.contains_key(&open_file.identity)
                || backend
                    .open_files
                    .values()
                    .any(|other| other.ancestors.contains(&open_file.identity))
                || !backend.roots.serve_only_the_shared_directory()
                    && backend
                        .pending_deletes
                        .values()
                        .flatten()
                        .any(|name| name.ancestors.contains(&open_file.identity))
        });
    // A rename target that is open cannot be replaced, so the open files are collected before one of them is borrowed mutably.
    let open_identities = if matches!(req_inner.set_buffer, FileInformationClass::Rename(_)) {
        backend
            .open_files
            .values()
            .map(|open_file| open_file.identity)
            .collect()
    } else {
        HashSet::new()
    };
    let mut renamed = None;
    let status = match file_for_request_mut(&mut backend.open_files, &req_inner.device_io_request) {
        Some(open_file) => match &req_inner.set_buffer {
            // [MS-FSA] 2.1.5.15.3 FileDispositionInformation and 2.1.5.15.12 FileRenameInformation refuse a handle without DELETE access before any other check.
            FileInformationClass::Rename(_) | FileInformationClass::Disposition(_) if !open_file.may_delete => {
                NtStatus::ACCESS_DENIED
            }
            FileInformationClass::Rename(_) if is_open_through_another_directory => NtStatus::SHARING_VIOLATION,
            FileInformationClass::Rename(_) if rename_denied => NtStatus::ACCESS_DENIED,
            // The root of a drive is no name to rename to.
            FileInformationClass::Rename(info) => match RelativePath::parse(&info.file_name)
                .and_then(|to| to.file_name().is_some().then_some(to).ok_or(InvalidPath))
            {
                Ok(to) => match backend
                    .roots
                    .of(open_file.device_id)
                    .map_err(status_from_errno)
                    .and_then(|root| {
                        rename(
                            &root,
                            open_file,
                            &to,
                            info.replace_if_exists == Boolean::True,
                            &backend.pending_deletes,
                            &open_identities,
                        )
                    }) {
                    Ok(ancestors) => {
                        renamed = Some((
                            open_file.device_id,
                            open_file.identity,
                            open_file.path.clone(),
                            to,
                            ancestors,
                        ));
                        NtStatus::SUCCESS
                    }
                    Err(status) => {
                        trace!(?status, "Rename file failed");
                        status
                    }
                },
                Err(InvalidPath) => {
                    trace!(file_name = ?info.file_name, "Rejected an invalid rename target");
                    NtStatus::OBJECT_NAME_INVALID
                }
            },
            // The allocation size is only a hint, so there is nothing to do.
            FileInformationClass::Allocation(_) => NtStatus::SUCCESS,
            FileInformationClass::Disposition(info) => {
                match set_delete_pending(
                    &backend.roots,
                    open_file,
                    &mut backend.pending_deletes,
                    info.delete_pending != 0,
                ) {
                    Ok(()) => NtStatus::SUCCESS,
                    Err(status) => {
                        trace!(?status, "Set delete pending failed");
                        status
                    }
                }
            }
            FileInformationClass::EndOfFile(info) => match ftruncate(open_file.file.as_fd(), info.end_of_file) {
                Ok(()) => NtStatus::SUCCESS,
                Err(error) => {
                    warn!(%error, "Failed to set end of file");
                    status_from_errno(error)
                }
            },
            _ => {
                // TODO
                NtStatus::SUCCESS
            }
        },
        None => {
            warn!("No such file");
            NtStatus::NO_SUCH_FILE
        }
    };
    // Every handle opened through the renamed link follows it, so a later delete removes this file and not whatever takes the old name.
    // A handle whose path no longer names the file was opened through that link, possibly spelled differently on a case-insensitive filesystem.
    // A handle opened through another hard link of the file still finds it at its path, and keeps that link.
    // Only the handles of drives that serve the same directory as the renaming drive take the new path, which names the file beneath the root of each of them.
    // The only other handle a rename leaves on the file is one on the root of a drive that serves another directory, since a rename is refused while such a drive holds the file open any other way.
    // That handle keeps its empty path, and takes none of the directories that lead to the new name, which lie beneath the root of the renaming drive and not beneath its own.
    if let Some((device_id, identity, from, to, ancestors)) = renamed {
        let root = backend.roots.of(device_id);
        for open_file in backend.open_files.values_mut().filter(|open_file| {
            open_file.identity == identity && backend.roots.serve_the_same_directory(open_file.device_id, device_id)
        }) {
            if open_file.path == from
                || !root
                    .as_ref()
                    .is_ok_and(|root| names_file(root, &open_file.path, identity))
            {
                open_file.path = to.clone();
                open_file.ancestors = ancestors.clone();
            }
        }
    }
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
        ClientDriveSetInformationResponse::new(&req_inner, status).map_err(|e| encode_err!(e))?,
    ))])
}

/// Renames the file `open_file` holds to `to` ([MS-FSA] 2.1.5.15.12 FileRenameInformation), and returns the directories leading to `to`.
///
/// The handle's path is resolved again, so it fails with STATUS_NO_SUCH_FILE once that name refers to another file than the handle's.
/// An existing `to` is a collision unless `replace_if_exists` is set.
/// A directory `to`, or one with open handles, is never replaced, and one marked for deletion fails with STATUS_DELETE_PENDING.
/// A `to` naming the source itself, as a case-only rename on a case-insensitive filesystem does, is not a collision, and a `to` naming another hard link of the source removes the source link.
/// The existence check and the rename are separate calls, so a local process creating `to` in between is overwritten.
/// The target directory is opened as a create opens it, so a path through a directory marked for deletion is refused with STATUS_DELETE_PENDING.
///
/// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa
fn rename(
    root: &Root,
    open_file: &OpenFile,
    to: &RelativePath,
    replace_if_exists: bool,
    pending_deletes: &HashMap<FileIdentity, Vec<PendingDelete>>,
    open_identities: &HashSet<FileIdentity>,
) -> Result<Vec<FileIdentity>, NtStatus> {
    let (from_parent, from_name) = open_file.path.open_parent(root).map_err(status_from_parent_errno)?;
    let (to_parent, to_name, to_ancestors) = to.open_parent_with_ancestors(root).map_err(status_from_parent_errno)?;
    let to_ancestors = to_ancestors.iter().map(FileIdentity::of).collect::<Vec<_>>();
    if to_ancestors
        .iter()
        .any(|ancestor| pending_deletes.contains_key(ancestor))
    {
        return Err(NtStatus::DELETE_PENDING);
    }
    let to_parent_identity = FileIdentity::of(&fstat(to_parent.as_fd()).map_err(status_from_errno)?);
    let source = fstatat(from_parent.as_fd(), from_name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(status_from_errno)?;
    if FileIdentity::of(&source) != open_file.identity {
        return Err(NtStatus::NO_SUCH_FILE);
    }
    match fstatat(to_parent.as_fd(), to_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(target) if FileIdentity::of(&target) == open_file.identity => {
            // `to` is either the source link under another spelling, as a case- or normalization-insensitive filesystem resolves it, or another hard link of the same file.
            // POSIX rename leaves two hard links of one file both in place, while [MS-FSA] removes the source link.
            // A link lives in one directory, so in another directory `to` is another link; in the same one it is only if an entry is stored under exactly that name.
            // Otherwise the rename goes ahead, which at most changes the spelling of the only link and never removes it.
            let same_directory =
                FileIdentity::of(&fstat(from_parent.as_fd()).map_err(status_from_errno)?) == to_parent_identity;
            let is_another_link = source.st_nlink >= 2
                && (!same_directory
                    || from_name != to_name
                        && contains_entry(to_parent.as_fd(), to_name).map_err(status_from_errno)?);
            if is_another_link {
                unlinkat(from_parent.as_fd(), from_name, UnlinkatFlags::NoRemoveDir).map_err(status_from_errno)?;
                return Ok(to_ancestors);
            }
        }
        Ok(_) if !replace_if_exists => return Err(NtStatus::OBJECT_NAME_COLLISION),
        Ok(target) if is_directory(&target) => return Err(NtStatus::ACCESS_DENIED),
        Ok(target) if pending_deletes.contains_key(&FileIdentity::of(&target)) => {
            return Err(NtStatus::DELETE_PENDING);
        }
        Ok(target) if open_identities.contains(&FileIdentity::of(&target)) => return Err(NtStatus::ACCESS_DENIED),
        Ok(_) | Err(Errno::ENOENT) => {}
        Err(error) => return Err(status_from_errno(error)),
    }
    renameat(from_parent.as_fd(), from_name, to_parent.as_fd(), to_name).map_err(status_from_errno)?;
    Ok(to_ancestors)
}

/// Whether `directory` holds an entry stored under exactly the bytes of `name`, whatever spellings the filesystem treats as equal.
fn contains_entry(directory: BorrowedFd<'_>, name: &str) -> nix::Result<bool> {
    // A fresh descriptor reads the directory from its start without moving the caller's read position.
    let directory = nix::fcntl::openat(
        directory,
        ".",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    for entry in Dir::from_fd(directory)? {
        if entry?.file_name().to_bytes() == name.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Marks `open_file` for deletion when its last handle closes, or clears the mark.
///
/// A read-only file is not marked, and a directory is marked only when it is empty ([MS-FSA] 2.1.5.15.3 FileDispositionInformation).
///
/// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/386d9ec5-e0f6-4853-b175-c05be01419e0
fn set_delete_pending(
    roots: &DriveRoots,
    open_file: &OpenFile,
    pending_deletes: &mut HashMap<FileIdentity, Vec<PendingDelete>>,
    delete_pending: bool,
) -> Result<(), NtStatus> {
    if !delete_pending {
        pending_deletes.remove(&open_file.identity);
        return Ok(());
    }
    // The root of a drive cannot be removed through the drive, and a mark on it would refuse every create in it until its handles close.
    if open_file.path.file_name().is_none() {
        return Err(NtStatus::ACCESS_DENIED);
    }
    if is_read_only(&fstat(open_file.file.as_fd()).map_err(status_from_errno)?) {
        return Err(cannot_delete());
    }
    if holds_entries(&open_file.file).map_err(status_from_errno)? {
        return Err(NtStatus::DIRECTORY_NOT_EMPTY);
    }
    mark_for_deletion(
        roots,
        pending_deletes,
        open_file.identity,
        PendingDelete {
            device_id: open_file.device_id,
            path: open_file.path.clone(),
            ancestors: open_file.ancestors.clone(),
        },
    );
    Ok(())
}

/// Marks the file `identity` for deletion by `name`, which becomes the name it was marked through last.
fn mark_for_deletion(
    roots: &DriveRoots,
    pending_deletes: &mut HashMap<FileIdentity, Vec<PendingDelete>>,
    identity: FileIdentity,
    name: PendingDelete,
) {
    let names = pending_deletes.entry(identity).or_default();
    // A name marked through the same root is replaced, because the new name is as removable as it is.
    // A different root may be able to remove the file where this one cannot, so the names marked through other roots are kept.
    // With only a shared directory every device uses the one root, so the new name is the only one.
    names.retain(|earlier| !roots.use_the_same_root(earlier.device_id, name.device_id));
    // The names are kept from the one marked first to the one marked last, so the first makes room for the new one.
    if MAX_PENDING_DELETE_NAMES <= names.len() {
        names.remove(0);
    }
    names.push(name);
}

/// Whether `file` is a directory with entries, which cannot be marked for deletion.
fn holds_entries(file: &std::fs::File) -> nix::Result<bool> {
    if !is_directory(&fstat(file.as_fd())?) {
        return Ok(false);
    }
    // The open directory itself is read, not whatever its path names now.
    // A duplicate of the handle would share its read position, so a second check would read on from the end of the first.
    let directory = nix::fcntl::openat(
        file.as_fd(),
        ".",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let mut listing = DirectoryListing::open(directory, true, None)?;
    Ok(listing.next_entry()?.is_some())
}

fn is_directory(stat: &FileStat) -> bool {
    SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFDIR
}

/// Whether the file is reported with FILE_ATTRIBUTE_READONLY, which a file without any write permission is.
fn is_read_only(stat: &FileStat) -> bool {
    !Mode::from_bits_truncate(stat.st_mode).intersects(Mode::S_IWUSR | Mode::S_IWGRP | Mode::S_IWOTH)
}

/// STATUS_CANNOT_DELETE, which [`NtStatus`] does not name.
fn cannot_delete() -> NtStatus {
    NtStatus::from(0xC000_0121)
}

/// Maps a filesystem error to the status the Windows drive backend reports for the same condition.
fn status_from_errno(error: Errno) -> NtStatus {
    match error {
        Errno::ENOENT => NtStatus::NO_SUCH_FILE,
        Errno::EEXIST => NtStatus::OBJECT_NAME_COLLISION,
        Errno::ENOTDIR => NtStatus::NOT_A_DIRECTORY,
        Errno::EISDIR => NtStatus::FILE_IS_A_DIRECTORY,
        Errno::ENOTEMPTY => NtStatus::DIRECTORY_NOT_EMPTY,
        Errno::ENAMETOOLONG => NtStatus::OBJECT_NAME_INVALID,
        // A refused symbolic link or mount point is reported like a refused reparse point.
        Errno::EACCES | Errno::EPERM | Errno::ELOOP | Errno::EXDEV => NtStatus::ACCESS_DENIED,
        Errno::EROFS => NtStatus::MEDIA_WRITE_PROTECTED,
        Errno::ENOSPC | Errno::EDQUOT => NtStatus::DISK_FULL,
        Errno::EMFILE | Errno::ENFILE => NtStatus::INSUFFICIENT_RESOURCES,
        // A read or write through a handle opened without the access it needs.
        Errno::EBADF => NtStatus::ACCESS_DENIED,
        _ => NtStatus::UNSUCCESSFUL,
    }
}

/// Maps an error opening the directories that lead to a file, where a missing one is a missing path.
fn status_from_parent_errno(error: Errno) -> NtStatus {
    match error {
        Errno::ENOENT | Errno::ENOTDIR => NtStatus::OBJECT_PATH_NOT_FOUND,
        error => status_from_errno(error),
    }
}

/// Whether `path` still names the file `identity`.
fn names_file(root: &Root, path: &RelativePath, identity: FileIdentity) -> bool {
    path.open_parent(root)
        .and_then(|(parent, name)| fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW))
        .is_ok_and(|stat| FileIdentity::of(&stat) == identity)
}

/// Removes the file or empty directory `path` names, unless that name now refers to another file than `identity`.
fn remove_file(root: &Root, path: &RelativePath, identity: FileIdentity) -> nix::Result<()> {
    let (parent, name) = path.open_parent(root)?;
    let stat = fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if FileIdentity::of(&stat) != identity {
        return Err(Errno::ESTALE);
    }
    let flags = if is_directory(&stat) {
        UnlinkatFlags::RemoveDir
    } else {
        UnlinkatFlags::NoRemoveDir
    };
    unlinkat(parent.as_fd(), name, flags)
}

// in fact, it is time in secs which is very small
#[expect(clippy::arithmetic_side_effects)]
pub(crate) fn transform_to_filetime(time_in_secs: i64) -> i64 {
    let mut time = time_in_secs * 10000000;
    time += 116444736000000000;
    time
}

pub(crate) fn get_file_attributes(is_dir: bool, readonly: bool, file_name: &str) -> FileAttributes {
    let mut file_attribute = FileAttributes::empty();
    if is_dir {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_DIRECTORY;
    }
    if file_attribute.is_empty() {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_ARCHIVE;
    }

    if file_name.len() > 1 && file_name.starts_with('.') && file_name.as_bytes()[1] != b'.' {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_HIDDEN;
    }
    if readonly {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_READONLY;
    }
    file_attribute
}

fn make_query_dir_resp(
    entry: Result<DirectoryEntry, NtStatus>,
    device_io_request: DeviceIoRequest,
    file_class: FileInformationClassLevel,
) -> PduResult<Vec<SvcMessage>> {
    match entry {
        Err(status) => Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
            ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(device_io_request, status),
                buffer: None,
            },
        ))]),
        Ok(DirectoryEntry { name, stat }) => {
            if file_class == FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION {
                let is_dir = is_directory(&stat);
                let file_attribute = get_file_attributes(is_dir, is_read_only(&stat), &name);
                let info = FileBothDirectoryInformation::new(
                    transform_to_filetime(stat.st_ctime),
                    transform_to_filetime(stat.st_ctime),
                    transform_to_filetime(stat.st_atime),
                    transform_to_filetime(stat.st_mtime),
                    stat.st_size,
                    file_attribute,
                    name,
                );
                let info2 = FileInformationClass::BothDirectory(info);
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                    ClientDriveQueryDirectoryResponse {
                        device_io_reply: DeviceIoResponse::new(device_io_request, NtStatus::SUCCESS),
                        buffer: Some(info2),
                    },
                ))])
            } else {
                warn!("Unsupported file class for query directory");
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                    ClientDriveQueryDirectoryResponse {
                        device_io_reply: DeviceIoResponse::new(device_io_request, NtStatus::NOT_SUPPORTED),
                        buffer: None,
                    },
                ))])
            }
        }
    }
}

pub(crate) fn query_directory(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveQueryDirectoryRequest,
) -> PduResult<Vec<SvcMessage>> {
    let entry = match file_for_request_mut(&mut backend.open_files, &req_inner.device_io_request) {
        None => {
            warn!("No file to query directory");
            Err(NtStatus::NO_SUCH_FILE)
        }
        Some(open_file) if req_inner.initial_query == 0 => {
            match open_file
                .listing
                .as_mut()
                .map_or(Ok(None), DirectoryListing::next_entry)
            {
                Ok(entry) => entry.ok_or(NtStatus::NO_MORE_FILES),
                // An error is not the end of the listing, so the server does not take a partial listing for a complete one.
                Err(error) => {
                    warn!(%error, "Query directory error");
                    Err(status_from_errno(error))
                }
            }
        }
        Some(open_file) => {
            // An initial query restarts enumeration, so a failed or single-entry query must not resume an earlier listing.
            open_file.listing = None;
            match RelativePath::parse_query(&req_inner.path) {
                // Only a missing name or an empty listing is no match; any other error is reported, so the server does not take it for an empty directory.
                Ok((directory, pattern)) => {
                    match backend
                        .roots
                        .of(open_file.device_id)
                        .and_then(|root| start_query(&root, open_file, &directory, pattern))
                    {
                        Ok(entry) => entry.ok_or(NtStatus::NO_SUCH_FILE),
                        Err(error) => {
                            trace!(%error, "Initial query directory failed");
                            Err(status_from_errno(error))
                        }
                    }
                }
                Err(InvalidPath) => {
                    trace!(path = ?req_inner.path, "Rejected an invalid query path");
                    Err(NtStatus::OBJECT_NAME_INVALID)
                }
            }
        }
    };
    return make_query_dir_resp(entry, req_inner.device_io_request, req_inner.file_info_class_lvl);

    /// Returns the first match for `pattern` in `directory`, where a pattern with wildcards enumerates the matching entries.
    fn start_query(
        root: &Root,
        open_file: &mut OpenFile,
        directory: &RelativePath,
        pattern: &str,
    ) -> nix::Result<Option<DirectoryEntry>> {
        let opened = directory.open_directory(root)?;
        if contains_wildcards(pattern) {
            let filter = (pattern != "*").then(|| pattern.to_owned());
            let mut listing = DirectoryListing::open(opened, directory.file_name().is_none(), filter)?;
            let entry = listing.next_entry()?;
            open_file.listing = Some(listing);
            return Ok(entry);
        }

        let stat = fstatat(opened.as_fd(), pattern, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        Ok(Some(DirectoryEntry {
            name: pattern.to_owned(),
            stat,
        }))
    }
}

fn make_create_drive_resp(
    device_io_request: DeviceIoRequest,
    create_disposition: CreateDisposition,
    file_id: u32,
) -> PduResult<Vec<SvcMessage>> {
    let io_response = DeviceIoResponse::new(device_io_request, NtStatus::SUCCESS);
    let information = if create_disposition == CreateDisposition::FILE_OPEN_IF {
        Information::FILE_OPENED
    } else if create_disposition == CreateDisposition::FILE_OVERWRITE_IF {
        Information::FILE_OVERWRITTEN
    } else {
        Information::FILE_SUPERSEDED
    };
    let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
        device_io_reply: io_response,
        file_id,
        information,
    });
    Ok(vec![SvcMessage::from(res)])
}
pub(crate) fn create_drive(
    backend: &mut NixRdpdrBackend,
    req_inner: DeviceCreateRequest,
) -> PduResult<Vec<SvcMessage>> {
    let file_id = backend.next_file_id();

    let opened = if backend.max_open_files <= backend.open_files.len().saturating_add(backend.roots.held_count()) {
        warn!("Rejected a create request beyond the open file limit");
        Err(NtStatus::INSUFFICIENT_RESOURCES)
    } else {
        match RelativePath::parse(&req_inner.path) {
            Ok(path) => backend
                .roots
                .of(req_inner.device_io_request.device_id)
                .map_err(|error| {
                    if backend.roots.serve_only_the_shared_directory() {
                        warn!(%error, "Open drive root error");
                    } else {
                        // UNTESTED: once roots are held, a create through a drive that is served nothing logs nothing above `trace!`.
                        // An unusable root was logged with a warning when it was configured, and the server decides how many creates it sends, so a warning here would let it fill the log.
                        // No test reads the log, so after changing this, check that only a shared directory opened for each request warns.
                        trace!(
                            device_id = req_inner.device_io_request.device_id,
                            "Rejected a create through a drive that is served nothing"
                        );
                    }
                    NtStatus::UNSUCCESSFUL
                })
                .and_then(|root| open_file(&root, &req_inner, &path, &backend.pending_deletes))
                .map(|(file, identity, ancestors)| (file, path, identity, ancestors)),
            Err(InvalidPath) => {
                trace!(path = ?req_inner.path, "Rejected an invalid create path");
                Err(NtStatus::OBJECT_NAME_INVALID)
            }
        }
    };

    match opened {
        Ok((file, path, identity, ancestors)) => {
            trace!(file_id, ?path, "Opened drive file");
            backend.open_files.insert(
                file_id,
                OpenFile {
                    device_id: req_inner.device_io_request.device_id,
                    file,
                    path,
                    identity,
                    ancestors,
                    // GENERIC_ALL and MAXIMUM_ALLOWED include DELETE ([MS-SMB2] 2.2.13.1).
                    // A MAXIMUM_ALLOWED open that fell back to read-only keeps it, as Windows lets a read-only file be renamed: the parent directory's permissions decide a rename, and the read-only check before a delete decides a delete.
                    may_delete: req_inner.desired_access.intersects(
                        DesiredAccess::DELETE | DesiredAccess::GENERIC_ALL | DesiredAccess::MAXIMUM_ALLOWED,
                    ),
                    delete_on_close: req_inner.create_options.contains(CreateOptions::FILE_DELETE_ON_CLOSE),
                    listing: None,
                },
            );
            make_create_drive_resp(req_inner.device_io_request, req_inner.create_disposition, file_id)
        }
        Err(status) => {
            let io_response = DeviceIoResponse::new(req_inner.device_io_request, status);
            let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                device_io_reply: io_response,
                file_id,
                information: Information::empty(),
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}

/// Opens or creates `path` beneath `root` as `req` asks, and returns it with its identity and the directories leading to it.
///
/// A file marked for deletion is refused before it is overwritten, so a failed open leaves its contents intact.
fn open_file(
    root: &Root,
    req: &DeviceCreateRequest,
    path: &RelativePath,
    pending_deletes: &HashMap<FileIdentity, Vec<PendingDelete>>,
) -> Result<(std::fs::File, FileIdentity, Vec<FileIdentity>), NtStatus> {
    // [MS-SMB2] 2.2.13 SMB2 CREATE Request makes any disposition but the six it lists invalid.
    if !matches!(
        req.create_disposition,
        CreateDisposition::FILE_SUPERSEDE
            | CreateDisposition::FILE_OPEN
            | CreateDisposition::FILE_CREATE
            | CreateDisposition::FILE_OPEN_IF
            | CreateDisposition::FILE_OVERWRITE
            | CreateDisposition::FILE_OVERWRITE_IF
    ) {
        warn!(disposition = ?req.create_disposition, "Rejected an invalid create disposition");
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let is_directory_request = req.create_options.contains(CreateOptions::FILE_DIRECTORY_FILE);
    // It also makes a directory request invalid with FILE_NON_DIRECTORY_FILE or a replacing disposition.
    if is_directory_request
        && (req.create_options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE)
            || !matches!(
                req.create_disposition,
                CreateDisposition::FILE_CREATE | CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OPEN_IF
            ))
    {
        warn!(disposition = ?req.create_disposition, "Rejected an invalid directory create request");
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let delete_on_close = req.create_options.contains(CreateOptions::FILE_DELETE_ON_CLOSE);
    // [MS-SMB2] 2.2.13 requires the DELETE bit itself with FILE_DELETE_ON_CLOSE, which GENERIC_ALL or MAXIMUM_ALLOWED does not replace.
    if delete_on_close && !req.desired_access.contains(DesiredAccess::DELETE) {
        warn!("Rejected FILE_DELETE_ON_CLOSE without DELETE access");
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let (parent, name, ancestors) = path.open_parent_with_ancestors(root).map_err(|error| {
        trace!(%error, ?path, "Open parent directory failed");
        status_from_parent_errno(error)
    })?;
    let ancestors = ancestors.iter().map(FileIdentity::of).collect::<Vec<_>>();
    // [MS-FSA] 2.1.5.1 Server Requests an Open of a File refuses a path through any directory marked for deletion, so nothing is created in it.
    // A local process can still add entries to such a directory, and its delete then fails when its last handle closes.
    if ancestors.iter().any(|ancestor| pending_deletes.contains_key(ancestor)) {
        return Err(NtStatus::DELETE_PENDING);
    }

    let truncates = matches!(
        req.create_disposition,
        CreateDisposition::FILE_SUPERSEDE | CreateDisposition::FILE_OVERWRITE | CreateDisposition::FILE_OVERWRITE_IF
    );
    // Statuses follow [MS-FSA] 2.1.5.1 Server Requests an Open of a File and 2.1.5.1.2 Open of an Existing File.
    let flags = match fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) if is_directory(&stat) => {
            if req.create_options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE) {
                trace!("Refused to open a directory as a file");
                return Err(NtStatus::FILE_IS_A_DIRECTORY);
            }
            // An existing directory cannot be created, overwritten or superseded.
            if !matches!(
                req.create_disposition,
                CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OPEN_IF
            ) {
                trace!(disposition = ?req.create_disposition, "Refused to replace an existing directory");
                return Err(if path.file_name().is_none() {
                    NtStatus::ACCESS_DENIED
                } else {
                    NtStatus::OBJECT_NAME_COLLISION
                });
            }
            // The root of a drive cannot be removed through the drive, as on the Windows backend.
            if delete_on_close && path.file_name().is_none() {
                trace!("Refused to delete a drive root on close");
                return Err(NtStatus::ACCESS_DENIED);
            }
            OFlag::O_RDONLY | OFlag::O_DIRECTORY
        }
        Ok(_) if is_directory_request => {
            trace!("Refused to open a file as a directory");
            return Err(if req.create_disposition == CreateDisposition::FILE_CREATE {
                NtStatus::OBJECT_NAME_COLLISION
            } else {
                NtStatus::NOT_A_DIRECTORY
            });
        }
        Err(Errno::ENOENT) if is_directory_request => {
            if req.create_disposition == CreateDisposition::FILE_OPEN {
                return Err(NtStatus::NO_SUCH_FILE);
            }
            mkdirat(parent.as_fd(), name, Mode::from_bits_truncate(0o777)).map_err(|error| {
                trace!(%error, ?path, "Create directory failed");
                status_from_errno(error)
            })?;
            OFlag::O_RDONLY | OFlag::O_DIRECTORY
        }
        Err(error) if error != Errno::ENOENT => {
            trace!(%error, ?path, "Look up file failed");
            return Err(status_from_errno(error));
        }
        _ => {
            let disposition = match req.create_disposition {
                CreateDisposition::FILE_OPEN_IF
                | CreateDisposition::FILE_SUPERSEDE
                | CreateDisposition::FILE_OVERWRITE_IF => OFlag::O_CREAT,
                CreateDisposition::FILE_CREATE => OFlag::O_CREAT | OFlag::O_EXCL,
                CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OVERWRITE => OFlag::empty(),
                // Refused before the lookup.
                _ => return Err(NtStatus::INVALID_PARAMETER),
            };
            // The access mode follows the access the server requests, with the bits [MS-SMB2] 2.2.13.1 defines, and a truncating disposition needs write access whatever it requests.
            let all = || DesiredAccess::GENERIC_ALL | DesiredAccess::MAXIMUM_ALLOWED;
            let read = req.desired_access.intersects(
                DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY
                    | DesiredAccess::FILE_EXECUTE
                    | DesiredAccess::GENERIC_READ
                    | DesiredAccess::GENERIC_EXECUTE
                    | all(),
            );
            let write = truncates
                || req.desired_access.intersects(
                    DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
                        | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY
                        | DesiredAccess::GENERIC_WRITE
                        | all(),
                );
            let access = match (read, write) {
                (_, false) => OFlag::O_RDONLY,
                (false, true) => OFlag::O_WRONLY,
                (true, true) => OFlag::O_RDWR,
            };
            disposition | access
        }
    };
    let mode = Mode::from_bits_truncate(0o666);
    let opened = match root.open_at(parent.as_fd(), name, flags, mode) {
        // MAXIMUM_ALLOWED asks for whatever access the file allows, so a refused write still leaves read access.
        Err(Errno::EACCES | Errno::EROFS)
            if req.desired_access.contains(DesiredAccess::MAXIMUM_ALLOWED)
                && flags.contains(OFlag::O_RDWR)
                && !truncates =>
        {
            root.open_at(parent.as_fd(), name, flags.difference(OFlag::O_RDWR), mode)
        }
        opened => opened,
    };
    let file = opened.map_err(|error| {
        trace!(%error, ?name, "Open file failed");
        status_from_errno(error)
    })?;
    let stat = fstat(file.as_fd()).map_err(status_from_errno)?;
    let identity = FileIdentity::of(&stat);
    // Windows refuses to open a file marked for deletion, where [MS-FSA] 2.1.5.1 Server Requests an Open of a File states this only for the directories on the path.
    if pending_deletes.contains_key(&identity) {
        return Err(NtStatus::DELETE_PENDING);
    }
    // [MS-FSA] 2.1.5.1.2.1 refuses to delete a read-only file on close.
    if delete_on_close && is_read_only(&stat) {
        return Err(cannot_delete());
    }
    // Truncating only once the open can no longer fail keeps a refused file intact, where O_TRUNC would empty it first.
    // Like O_TRUNC, this leaves anything but a regular file, such as a FIFO, unchanged.
    let is_regular_file = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFREG;
    if is_regular_file && truncates {
        ftruncate(file.as_fd(), 0).map_err(|error| {
            warn!(%error, ?name, "Truncate file error");
            status_from_errno(error)
        })?;
    }
    Ok((std::fs::File::from(file), identity, ancestors))
}

/// Returns the file `request` names, which is one opened through the device the request addresses.
///
/// A file ID used through another device names nothing, like one that was never opened, so the server cannot reach the files of another drive or learn which IDs they hold.
fn file_for_request<'a>(open_files: &'a HashMap<u32, OpenFile>, request: &DeviceIoRequest) -> Option<&'a OpenFile> {
    open_files
        .get(&request.file_id)
        .filter(|open_file| open_file.device_id == request.device_id)
}

/// Like [`file_for_request`], for a request that changes the file.
fn file_for_request_mut<'a>(
    open_files: &'a mut HashMap<u32, OpenFile>,
    request: &DeviceIoRequest,
) -> Option<&'a mut OpenFile> {
    open_files
        .get_mut(&request.file_id)
        .filter(|open_file| open_file.device_id == request.device_id)
}

pub(crate) fn process_dependent_file(
    backend: &mut NixRdpdrBackend,
    request: DeviceIoRequest,
    error_fx: impl Fn(DeviceIoRequest) -> PduResult<Vec<SvcMessage>>,
    fx: impl Fn(&mut std::fs::File, DeviceIoRequest) -> PduResult<Vec<SvcMessage>>,
) -> PduResult<Vec<SvcMessage>> {
    match file_for_request_mut(&mut backend.open_files, &request) {
        None => error_fx(request),
        Some(open_file) => fx(&mut open_file.file, request),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use rstest::rstest;

    use super::*;

    fn build_error(factory: &NixRdpdrBackendFactory) -> NixRdpdrBackendFactoryError {
        let Err(error) = factory.build_rdpdr_backend() else {
            panic!("the factory accepted an invalid configuration");
        };
        *error
            .downcast_ref::<NixRdpdrBackendFactoryError>()
            .expect("a factory configuration error")
    }

    #[test]
    fn the_factory_rejects_reserved_and_duplicate_drive_ids() {
        let factory = NixRdpdrBackendFactory::new("/tmp".to_owned());
        assert_eq!(
            build_error(&factory.clone().with_drive(0, "zero".to_owned())),
            NixRdpdrBackendFactoryError::ReservedDeviceId
        );
        assert_eq!(
            build_error(&factory.with_drive(1, "a".to_owned()).with_drive(1, "b".to_owned())),
            NixRdpdrBackendFactoryError::DuplicateDeviceId(1)
        );
    }

    #[test]
    fn the_factory_rejects_empty_and_nul_names() {
        let factory = NixRdpdrBackendFactory::new("/tmp".to_owned());
        assert_eq!(
            build_error(&factory.clone().with_drive(1, String::new())),
            NixRdpdrBackendFactoryError::InvalidDriveName(1)
        );
        assert_eq!(
            build_error(&factory.clone().with_drive(2, "home\0x".to_owned())),
            NixRdpdrBackendFactoryError::InvalidDriveName(2)
        );
        assert_eq!(
            build_error(&factory.with_printer(String::new(), PrintTarget::DefaultPrinter)),
            NixRdpdrBackendFactoryError::InvalidPrinterName
        );
    }

    fn header(file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 1,
            file_id,
            completion_id: 0,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    /// Opens the shared directory, returning the status and file ID of the Device Create Response.
    fn open_share(backend: &mut NixRdpdrBackend) -> (NtStatus, u32) {
        open_root(backend, 1)
    }

    /// Opens the root of `device_id`, returning the status and file ID of the Device Create Response.
    fn open_root(backend: &mut NixRdpdrBackend, device_id: u32) -> (NtStatus, u32) {
        open_path(backend, device_id, r"\")
    }

    /// Opens `path` through `device_id`, returning the status and file ID of the Device Create Response.
    fn open_path(backend: &mut NixRdpdrBackend, device_id: u32, path: &str) -> (NtStatus, u32) {
        let request = DeviceCreateRequest {
            device_io_request: DeviceIoRequest {
                device_id,
                ..header(0, MajorFunction::Create)
            },
            desired_access: DesiredAccess::empty(),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: path.to_owned(),
        };
        create_response(&create_drive(backend, request).expect("complete create"))
    }

    /// The status and file ID of a Device Create Response.
    fn create_response(response: &[SvcMessage]) -> (NtStatus, u32) {
        let response = response[0].encode_unframed_pdu().expect("encode create response");
        // RDPDR_HEADER (4 bytes), DeviceId and CompletionId precede IoStatus, and FileId follows it
        // ([MS-RDPEFS] 2.2.1.5.1 Device Create Response (DR_CREATE_RSP)).
        (
            NtStatus::from(u32::from_le_bytes(response[12..16].try_into().expect("status"))),
            u32::from_le_bytes(response[16..20].try_into().expect("file ID")),
        )
    }

    /// Sends a create of `path` through `device_id` as the channel does, with every access, returning the status and file ID of the Device Create Response.
    fn create_through_the_channel(
        backend: &mut NixRdpdrBackend,
        device_id: u32,
        path: &str,
        create_disposition: CreateDisposition,
        create_options: CreateOptions,
    ) -> (NtStatus, u32) {
        let request = DeviceCreateRequest {
            device_io_request: DeviceIoRequest {
                device_id,
                ..header(0, MajorFunction::Create)
            },
            desired_access: DesiredAccess::GENERIC_ALL,
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition,
            create_options,
            path: path.to_owned(),
        };
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
            .expect("complete create");
        create_response(&response)
    }

    /// Renames `from` to `to` through `device_id` as the channel does, with a handle opened for the rename and closed after it.
    fn rename_through_the_channel(backend: &mut NixRdpdrBackend, device_id: u32, from: &str, to: &str) {
        let (opened, file_id) = create_through_the_channel(
            backend,
            device_id,
            from,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        );
        assert_eq!(opened, NtStatus::SUCCESS, "open {from}");
        let header = |major_function| DeviceIoRequest {
            device_id,
            ..header(file_id, major_function)
        };
        let renamed = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
                ServerDriveSetInformationRequest {
                    device_io_request: header(MajorFunction::SetInformation),
                    set_buffer: FileInformationClass::Rename(FileRenameInformation {
                        replace_if_exists: Boolean::False,
                        file_name: to.to_owned(),
                    }),
                },
            ))
            .expect("complete rename");
        // IoStatus lies at the same offset in every Device I/O Response ([MS-RDPEFS] 2.2.1.5 Device I/O Response (DR_DEVICE_IOCOMPLETION)).
        assert_eq!(create_response(&renamed).0, NtStatus::SUCCESS, "rename {from} to {to}");
        backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
                device_io_request: header(MajorFunction::Close),
            }))
            .expect("complete close");
    }

    #[test]
    fn the_server_cannot_hold_more_files_open_than_the_cap() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        backend.max_open_files = 2;

        let (status, first) = open_share(&mut backend);
        assert_eq!(status, NtStatus::SUCCESS);
        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);
        // oracle: mutant cap check removed or off by one
        assert_eq!(open_share(&mut backend).0, NtStatus::INSUFFICIENT_RESOURCES);

        close_device(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: header(first, MajorFunction::Close),
            },
        )
        .expect("complete close");
        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);
    }

    /// The devices whose roots the tests of the cap open.
    const FIRST: u32 = 1;
    const SECOND: u32 = 2;

    /// Two directories in one sandbox, each to be the root of a drive, removed when dropped.
    struct TwoRoots {
        sandbox: PathBuf,
        first: PathBuf,
        second: PathBuf,
    }

    impl TwoRoots {
        fn new() -> Self {
            static NEXT_SANDBOX: AtomicU32 = AtomicU32::new(0);
            let unique = format!(
                "rdpdr-native-nix-cap-{}-{}",
                std::process::id(),
                NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed)
            );
            let sandbox = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&sandbox);
            let (first, second) = (sandbox.join("first"), sandbox.join("second"));
            std::fs::create_dir_all(&first).expect("create first root");
            std::fs::create_dir_all(&second).expect("create second root");
            Self { sandbox, first, second }
        }

        /// A backend without a shared directory, whose devices `FIRST` and `SECOND` serve the two roots.
        fn backend(&self) -> NixRdpdrBackend {
            NixRdpdrBackend::default()
                .with_drive_root(FIRST, utf8(&self.first))
                .with_drive_root(SECOND, utf8(&self.second))
        }

        /// A backend under a cap of eight open files, half of which are four roots, given a new directory of the sandbox as the root of each device from 1 to `root_count`.
        ///
        /// The cap is set before the roots are given, because it bounds them as they are resolved.
        fn backend_with_roots(&self, root_count: u32) -> NixRdpdrBackend {
            let mut backend = NixRdpdrBackend::default();
            backend.max_open_files = 8;
            (1..=root_count).fold(backend, |backend, device_id| {
                let root = self.sandbox.join(format!("root-{device_id}"));
                std::fs::create_dir(&root).expect("create root");
                backend.with_drive_root(device_id, utf8(&root))
            })
        }
    }

    impl Drop for TwoRoots {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.sandbox);
        }
    }

    fn utf8(path: &Path) -> String {
        path.to_str().expect("UTF-8 temporary directory").to_owned()
    }

    /// Opens the root of each device in turn under a cap of eight open files, until a create fails.
    /// Returns the number of files opened and the status of the failure.
    fn opens_until_refused(mut backend: NixRdpdrBackend, devices: &[u32]) -> (usize, NtStatus) {
        // A create refused at the cap and one that ran out of descriptors report the same status.
        // Only a cap far below the descriptor limit tells them apart, and this private field is the only way to set one.
        backend.max_open_files = 8;
        for (opened, device_id) in devices.iter().cycle().enumerate() {
            let (status, _) = open_root(&mut backend, *device_id);
            if status != NtStatus::SUCCESS {
                return (opened, status);
            }
        }
        unreachable!("a cycle over the devices does not end")
    }

    /// The open files of every drive count against one limit, so more drives do not let the server hold more files open.
    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/file_table.rs
    #[test]
    fn drives_share_the_open_file_limit() {
        let roots = TwoRoots::new();
        let through_one_drive = opens_until_refused(roots.backend(), &[FIRST]);

        let through_two_drives = opens_until_refused(roots.backend(), &[FIRST, SECOND]);

        // oracle: mutant the status that refuses a create beyond the limit changed
        assert_eq!(through_one_drive.1, NtStatus::INSUFFICIENT_RESOURCES);
        assert!(0 < through_one_drive.0);
        assert_eq!(through_two_drives, through_one_drive);
    }

    /// A root held open takes one place under the limit on open files, so the descriptors of the roots come out of the budget that limit protects.
    // oracle: mutant held root descriptors left out of the open-file limit
    #[test]
    fn a_held_root_takes_the_place_of_one_open_file() {
        let roots = TwoRoots::new();
        let with_one_root = opens_until_refused(
            NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&roots.first)),
            &[FIRST],
        );

        let with_two_roots = opens_until_refused(roots.backend(), &[FIRST]);

        assert_eq!(with_one_root.1, NtStatus::INSUFFICIENT_RESOURCES);
        assert_eq!(with_two_roots.1, NtStatus::INSUFFICIENT_RESOURCES);
        assert!(0 < with_two_roots.0);
        assert_eq!(with_two_roots.0 + 1, with_one_root.0);
    }

    /// A root refused because its path followed a link in another root keeps the directory it named open.
    /// It therefore takes a place under the limit on open files, although nothing is served from it.
    // oracle: mutant a root refused for a link left out of the places the roots take under the open-file limit
    #[test]
    fn a_root_refused_for_a_link_takes_the_place_of_one_open_file() {
        let roots = TwoRoots::new();
        let link = roots.first.join("link");
        std::os::unix::fs::symlink(&roots.second, &link).expect("link directory");
        let mut backend = NixRdpdrBackend::default()
            .with_drive_root(FIRST, utf8(&roots.first))
            .with_drive_root(SECOND, utf8(&link));
        let through_the_refused_root = open_root(&mut backend, SECOND).0;

        let opened = opens_until_refused(backend, &[FIRST]);

        // The status of a create through a drive with any other unusable root.
        assert_eq!(through_the_refused_root, NtStatus::UNSUCCESSFUL);
        // The eight places under the cap, less one for each of the two roots.
        assert_eq!(opened, (6, NtStatus::INSUFFICIENT_RESOURCES));
    }

    /// A root that was replaced is no longer held open, so it takes no place under the limit on open files.
    // oracle: mutant the directory of a replaced root kept open
    #[test]
    fn a_replaced_root_is_no_longer_held_open() {
        let roots = TwoRoots::new();
        let with_one_root = opens_until_refused(
            NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&roots.second)),
            &[FIRST],
        );

        let with_a_replaced_root = opens_until_refused(
            NixRdpdrBackend::default()
                .with_drive_root(FIRST, utf8(&roots.first))
                .with_drive_root(FIRST, utf8(&roots.second)),
            &[FIRST],
        );

        assert!(0 < with_one_root.0);
        assert_eq!(with_a_replaced_root, with_one_root);
    }

    /// Each root takes its own place under the limit on open files, also when another root names the same directory.
    /// Which descriptor the requests of a drive use is not checked here.
    // oracle: mutant roots that name one directory took one place between them under the open-file limit
    #[test]
    fn drives_of_the_same_directory_each_hold_it_open() {
        let roots = TwoRoots::new();
        let alias = roots.sandbox.join("alias");
        std::os::unix::fs::symlink(&roots.first, &alias).expect("link directory");
        let one_drive = || NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&roots.first));
        let with_one_drive = opens_until_refused(one_drive(), &[FIRST]);

        let with_two_drives = opens_until_refused(one_drive().with_drive_root(SECOND, utf8(&alias)), &[FIRST]);

        assert_eq!(with_one_drive.1, NtStatus::INSUFFICIENT_RESOURCES);
        assert_eq!(with_two_drives.1, NtStatus::INSUFFICIENT_RESOURCES);
        assert!(0 < with_two_drives.0);
        assert_eq!(with_two_drives.0 + 1, with_one_drive.0);
    }

    /// The roots held open take at most half the places under the limit on open files.
    /// A root given beyond that is served nothing, and the roots given before it are served.
    // oracle: mutant roots held open without a bound, or with a bound off by one
    #[test]
    fn a_root_beyond_half_the_open_file_limit_is_served_nothing() {
        let roots = TwoRoots::new();
        let mut backend = roots.backend_with_roots(5);

        let beyond = open_root(&mut backend, 5).0;
        let within = [1, 2, 3, 4].map(|device_id| open_root(&mut backend, device_id).0);

        // The status of a create through a drive with any other unusable root.
        assert_eq!(beyond, NtStatus::UNSUCCESSFUL);
        assert_eq!(within, [NtStatus::SUCCESS; 4]);
    }

    /// A root beyond the bound is still resolved, so a root whose path follows a link below its directory is refused, although that directory is not held open.
    /// Which root lies beyond the bound can change between backends built from one configuration: here the first root names no directory until a server creates it, and then takes the place the enclosing root had.
    /// The server also renames a link to a directory outside every root into the path of the inner root, and a create through the inner drive must not land there.
    // oracle: mutant the directory of a root beyond the bound left out of those a followed link is checked against
    #[test]
    fn a_root_reached_through_a_link_below_a_root_beyond_the_bound_is_served_nothing() {
        // The devices given the root that is missing at first, the inner root and the enclosing root.
        let (missing, inner, enclosing) = (1, 4, 5);
        let roots = TwoRoots::new();
        let (outer, outside) = (roots.sandbox.join("outer"), roots.sandbox.join("outside"));
        for directory in [&outer.join("a").join("b"), &outer.join("d"), &outside] {
            std::fs::create_dir_all(directory).expect("create directory");
        }
        std::os::unix::fs::symlink(&outside, outer.join("d").join("b")).expect("link directory");
        // Four roots are held open under a cap of eight open files, so the last of five that name a directory lies beyond the bound.
        let backend = || {
            let mut backend = NixRdpdrBackend::default();
            backend.max_open_files = 8;
            [
                outer.join("new"),
                roots.first.clone(),
                roots.second.clone(),
                outer.join("a").join("b"),
                outer.clone(),
            ]
            .iter()
            .zip(missing..=enclosing)
            .fold(backend, |backend, (root, device_id)| {
                backend.with_drive_root(device_id, utf8(root))
            })
        };
        let mut earlier = backend();
        let created = create_through_the_channel(
            &mut earlier,
            enclosing,
            r"\new",
            CreateDisposition::FILE_CREATE,
            CreateOptions::FILE_DIRECTORY_FILE,
        );
        assert_eq!(created.0, NtStatus::SUCCESS);
        rename_through_the_channel(&mut earlier, enclosing, r"\a", r"\a_old");
        rename_through_the_channel(&mut earlier, enclosing, r"\d", r"\a");
        drop(earlier);

        let mut later = backend();
        let create_file = |backend: &mut NixRdpdrBackend, device_id| {
            create_through_the_channel(
                backend,
                device_id,
                r"\new.txt",
                CreateDisposition::FILE_CREATE,
                CreateOptions::empty(),
            )
            .0
        };
        let through_the_inner_drive = create_file(&mut later, inner);
        let through_the_enclosing_drive = create_file(&mut later, enclosing);
        let through_the_drive_missing_at_first = create_file(&mut later, missing);

        // The status of a create through a drive with any other unusable root.
        assert_eq!(through_the_inner_drive, NtStatus::UNSUCCESSFUL);
        assert_eq!(std::fs::read_dir(&outside).expect("list directory").count(), 0);
        assert_eq!(through_the_enclosing_drive, NtStatus::UNSUCCESSFUL);
        // The root that took the place of the enclosing one is served.
        assert_eq!(through_the_drive_missing_at_first, NtStatus::SUCCESS);
    }

    /// The devices given a root whose walk failed, a root reached through a link, and a root whose path only descends.
    const FAILED: u32 = 1;
    const LINKED: u32 = 2;
    const DESCENDING: u32 = 3;

    /// A directory, a link to another directory that lies outside both, and a path for a root whose walk is given a failure.
    struct RootsBesideAFailedWalk {
        /// Removes the sandbox when dropped.
        _roots: TwoRoots,
        /// A path without a symbolic link, which names the first directory.
        descending: String,
        /// A path that names the second directory through a link in the sandbox.
        linked: String,
        /// The second directory, which `linked` names.
        target: PathBuf,
        /// A path below the sandbox, which the tests never resolve.
        failed: String,
    }

    impl RootsBesideAFailedWalk {
        fn new() -> Self {
            let roots = TwoRoots::new();
            // The temporary directory can lie behind a symbolic link, as `/var` is on macOS.
            // A root given through it would itself be reached through a link, so every path here starts from the canonical one.
            let sandbox = std::fs::canonicalize(&roots.sandbox).expect("canonical sandbox");
            let (alias, target) = (sandbox.join("alias"), sandbox.join("second"));
            std::os::unix::fs::symlink(&target, &alias).expect("link directory");
            Self {
                descending: utf8(&sandbox.join("first")),
                linked: utf8(&alias),
                target,
                failed: utf8(&sandbox.join("failed")),
                _roots: roots,
            }
        }

        /// A backend without a shared directory, given the roots of `order` one after the other, the walk of `FAILED` having failed with `error`.
        fn backend(&self, order: [u32; 3], error: Errno) -> NixRdpdrBackend {
            order
                .into_iter()
                .fold(NixRdpdrBackend::default(), |mut backend, device_id| match device_id {
                    FAILED => {
                        backend
                            .roots
                            .record_root(backend.max_open_files, Some(FAILED), &self.failed, Err(error));
                        backend
                    }
                    LINKED => backend.with_drive_root(LINKED, self.linked.clone()),
                    _ => backend.with_drive_root(DESCENDING, self.descending.clone()),
                })
        }
    }

    /// Creates a file in the root of `device_id` as the channel does, returning the status of the Device Create Response.
    fn create_file_in_the_root(backend: &mut NixRdpdrBackend, device_id: u32) -> NtStatus {
        create_through_the_channel(
            backend,
            device_id,
            r"\new.txt",
            CreateDisposition::FILE_CREATE,
            CreateOptions::empty(),
        )
        .0
    }

    /// A walk that fails for want of a descriptor, or of permission to search a directory, leaves the directory of its root undetermined, and that directory may hold the link another root was reached through.
    /// That root is therefore served nothing, wherever its link lies and in whichever order the roots are given.
    // oracle: mutant a root reached through a link served while the directory of another root could not be determined
    #[rstest]
    #[case::the_failed_walk_first([FAILED, LINKED, DESCENDING])]
    #[case::the_failed_walk_last([LINKED, DESCENDING, FAILED])]
    fn a_root_reached_through_a_link_is_served_nothing_while_the_directory_of_another_root_is_undetermined(
        #[case] order: [u32; 3],
        #[values(Errno::EMFILE, Errno::EACCES)] error: Errno,
    ) {
        let fixture = RootsBesideAFailedWalk::new();
        let mut backend = fixture.backend(order, error);

        let through_the_linked_root = create_file_in_the_root(&mut backend, LINKED);

        // The status of a create through a drive with any other unusable root.
        assert_eq!(through_the_linked_root, NtStatus::UNSUCCESSFUL);
        assert_eq!(std::fs::read_dir(&fixture.target).expect("list directory").count(), 0);
    }

    /// A path that only descends ends in or below every directory it passes through, an undetermined one included, so its root is served.
    // oracle: mutant every root refused while the directory of another root could not be determined
    #[rstest]
    #[case::the_failed_walk_first([FAILED, LINKED, DESCENDING])]
    #[case::the_failed_walk_last([LINKED, DESCENDING, FAILED])]
    fn a_root_whose_path_only_descends_is_served_while_the_directory_of_another_root_is_undetermined(
        #[case] order: [u32; 3],
    ) {
        let fixture = RootsBesideAFailedWalk::new();
        let mut backend = fixture.backend(order, Errno::EMFILE);

        let through_the_descending_root = create_file_in_the_root(&mut backend, DESCENDING);

        assert_eq!(through_the_descending_root, NtStatus::SUCCESS);
    }

    /// The shared directory is a root like any other once a drive has a root of its own, so a failed walk of it leaves its directory undetermined too.
    // oracle: mutant the shared directory left out of the roots whose directory has to be known
    #[test]
    fn a_root_reached_through_a_link_is_served_nothing_while_the_shared_directory_is_undetermined() {
        let fixture = RootsBesideAFailedWalk::new();
        let mut backend = NixRdpdrBackend::new(fixture.failed.clone());
        backend
            .roots
            .record_root(backend.max_open_files, None, &fixture.failed, Err(Errno::EMFILE));
        let mut backend = backend.with_drive_root(LINKED, fixture.linked.clone());

        let through_the_linked_root = create_file_in_the_root(&mut backend, LINKED);

        // The status of a create through a drive with any other unusable root.
        assert_eq!(through_the_linked_root, NtStatus::UNSUCCESSFUL);
        assert_eq!(std::fs::read_dir(&fixture.target).expect("list directory").count(), 0);
    }

    /// A root given again replaces the one whose walk failed, so the directory of every root is known and the root reached through a link is served again.
    // oracle: mutant a root reached through a link kept unusable after the root whose directory was undetermined is replaced
    #[test]
    fn a_root_reached_through_a_link_is_served_once_the_root_with_an_undetermined_directory_is_replaced() {
        let fixture = RootsBesideAFailedWalk::new();
        let mut backend = fixture
            .backend([FAILED, LINKED, DESCENDING], Errno::EMFILE)
            .with_drive_root(FAILED, fixture.descending.clone());

        let through_the_linked_root = create_file_in_the_root(&mut backend, LINKED);

        assert_eq!(through_the_linked_root, NtStatus::SUCCESS);
    }

    /// A walk that shows its path names no directory leaves no directory undetermined, so a root reached through a link outside every root is served beside it.
    // oracle: mutant a path that names no directory counted as a directory that could not be determined
    #[rstest]
    #[case::missing(Errno::ENOENT)]
    #[case::not_a_directory(Errno::ENOTDIR)]
    #[case::too_many_links(Errno::ELOOP)]
    #[case::malformed(Errno::EINVAL)]
    #[case::too_many_components(Errno::ENAMETOOLONG)]
    fn a_root_reached_through_a_link_is_served_beside_a_root_whose_path_names_no_directory(#[case] error: Errno) {
        let fixture = RootsBesideAFailedWalk::new();
        let mut backend = fixture.backend([FAILED, LINKED, DESCENDING], error);

        let through_the_linked_root = create_file_in_the_root(&mut backend, LINKED);

        assert_eq!(through_the_linked_root, NtStatus::SUCCESS);
    }

    /// A walk that reached its directory but could not open it to be read leaves the root unusable and its directory known.
    /// A root reached through a link below that directory is therefore refused, and one reached through a link outside every root is served, in whichever order the roots are given.
    // oracle: mutant the directory of a root that could not be opened to be read forgotten, or counted as undetermined
    #[rstest]
    #[case::the_unreadable_root_first([1, 2, 3])]
    #[case::the_unreadable_root_last([2, 3, 1])]
    fn a_root_reached_through_a_link_is_checked_against_the_directory_of_a_root_that_cannot_be_read(
        #[case] order: [u32; 3],
    ) {
        // The devices given the root that cannot be read, a root reached through a link below it, and one reached through a link beside it.
        let (unreadable, below, beside) = (1, 2, 3);
        let roots = TwoRoots::new();
        let (outside, other) = (&roots.second, roots.sandbox.join("other"));
        std::fs::create_dir(&other).expect("create directory");
        std::os::unix::fs::symlink(outside, roots.first.join("link")).expect("link directory");
        std::os::unix::fs::symlink(&other, roots.sandbox.join("alias")).expect("link directory");
        let mut backend = NixRdpdrBackend::default();
        for device_id in order {
            backend = if device_id == unreadable {
                let path = utf8(&roots.first);
                let walk = Root::resolve(&path).expect("resolve root");
                let walk = ResolvedRoot {
                    root: Err(Errno::EACCES),
                    ..walk
                };
                backend
                    .roots
                    .record_root(backend.max_open_files, Some(unreadable), &path, Ok(walk));
                backend
            } else if device_id == below {
                backend.with_drive_root(below, utf8(&roots.first.join("link")))
            } else {
                backend.with_drive_root(beside, utf8(&roots.sandbox.join("alias")))
            };
        }

        let through_the_unreadable_root = create_file_in_the_root(&mut backend, unreadable);
        let through_the_root_below = create_file_in_the_root(&mut backend, below);
        let through_the_root_beside = create_file_in_the_root(&mut backend, beside);

        // The status of a create through a drive with any other unusable root.
        assert_eq!(through_the_unreadable_root, NtStatus::UNSUCCESSFUL);
        assert_eq!(through_the_root_below, NtStatus::UNSUCCESSFUL);
        assert_eq!(std::fs::read_dir(outside).expect("list directory").count(), 0);
        assert_eq!(through_the_root_beside, NtStatus::SUCCESS);
    }

    /// However many roots are given, they leave half the places under the limit to open files.
    /// A root beyond the bound is therefore not held open, which would take a place.
    // oracle: mutant roots took the places the limit leaves to open files, or a root beyond the bound was kept open
    #[rstest]
    #[case::one_root_more_than_the_bound(5)]
    #[case::more_roots_than_the_limit(12)]
    fn roots_leave_half_the_open_file_limit_to_open_files(#[case] root_count: u32) {
        let roots = TwoRoots::new();
        let backend = roots.backend_with_roots(root_count);

        let opened = opens_until_refused(backend, &[1]);

        assert_eq!(opened, (4, NtStatus::INSUFFICIENT_RESOURCES));
    }

    /// A root given again takes the place of the root it replaces, so it is served although the roots already hold half the places under the limit.
    // oracle: mutant the root being replaced counted against the bound on the root that replaces it
    #[test]
    fn a_root_takes_the_place_of_the_root_it_replaces() {
        let roots = TwoRoots::new();
        std::fs::write(roots.second.join("only-in-second.txt"), b"second").expect("write file");

        let mut backend = roots.backend_with_roots(4).with_drive_root(1, utf8(&roots.second));

        assert_eq!(open_path(&mut backend, 1, r"\only-in-second.txt").0, NtStatus::SUCCESS);
    }

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/file_table.rs
    #[test]
    fn a_new_file_never_takes_the_id_of_an_open_one() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        let (status, first) = open_share(&mut backend);
        assert_eq!(status, NtStatus::SUCCESS);
        // The counter has come round to an ID that is still open.
        backend.file_id = first;

        let (status, second) = open_share(&mut backend);

        assert_eq!(status, NtStatus::SUCCESS);
        assert_ne!(second, first);
    }

    // oracle: mutant file ID counter overflows instead of wrapping
    #[test]
    fn file_ids_wrap_around_after_the_last_one() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        backend.file_id = u32::MAX;

        assert_eq!(open_share(&mut backend), (NtStatus::SUCCESS, u32::MAX));
        assert_eq!(open_share(&mut backend), (NtStatus::SUCCESS, 0));
    }

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
    #[test]
    fn reset_closes_the_files_the_server_forgot() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        backend.max_open_files = 1;
        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);

        backend.reset().expect("reset");

        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);
    }

    #[test]
    fn the_printer_takes_an_id_no_drive_uses() {
        let product = NixRdpdrBackendFactory::new("/tmp".to_owned())
            .with_drive(DEFAULT_PRINTER_DEVICE_ID, "taken".to_owned())
            .with_printer("printer".to_owned(), PrintTarget::DefaultPrinter)
            .build_rdpdr_backend()
            .expect("valid configuration");
        assert_eq!(
            product.printer().expect("printer").device_id(),
            DEFAULT_PRINTER_DEVICE_ID - 1
        );
    }
}
