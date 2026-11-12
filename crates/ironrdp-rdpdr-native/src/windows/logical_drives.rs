//! Drive-letter selection and hotplug notifications for Windows RDPDR redirection.

use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use tracing::{debug, warn};
use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SetThreadErrorMode, THREAD_ERROR_MODE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVTYP_VOLUME, DEV_BROADCAST_HDR, DEV_BROADCAST_VOLUME, DefWindowProcW,
    DestroyWindow, DispatchMessageW, GetMessageW, MSG, PostMessageW, PostThreadMessageW, RegisterClassW, SetTimer,
    WM_APP, WM_DEVICECHANGE, WM_QUIT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::factory::RedirectedDrive;

const LOGICAL_DRIVE_COUNT: u8 = 26;
/// First RDPDR device ID for drives other than drive letters, which own `1..=26`.
///
/// INVARIANT: one more than the device ID of `Z:`.
pub const FIRST_NAMED_DRIVE_DEVICE_ID: u32 = 27;
const ALL_LOGICAL_DRIVES: u32 = (1 << LOGICAL_DRIVE_COUNT) - 1;
/// Network drives mapped with `net use` do not broadcast `WM_DEVICECHANGE`.
const LOGICAL_DRIVE_POLL_INTERVAL_MS: u32 = 2_000;
/// Polls between probes of mapped letters whose volume is not ready, such as a network share that
/// is reachable again after a VPN reconnect. A probe of an unreachable share can block, so it runs
/// far less often than the letter-mask poll.
const UNREADY_RETRY_POLLS: u32 = 8;
const WM_RESCAN_LOGICAL_DRIVES: u32 = WM_APP + 1;

/// A local logical volume identified by its drive letter.
///
/// Drive letters own the RDPDR device IDs `1..=26` (`A:` to `Z:`), so a letter keeps the same ID
/// across hotplug events and reconnects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalDrive {
    /// INVARIANT: `index < LOGICAL_DRIVE_COUNT`.
    index: u8,
}

impl LogicalDrive {
    /// Returns the logical drive for an ASCII drive letter, in either case.
    pub fn from_letter(letter: char) -> Option<Self> {
        let index = u8::try_from(letter.to_ascii_uppercase()).ok()?.checked_sub(b'A')?;
        (index < LOGICAL_DRIVE_COUNT).then_some(Self { index })
    }

    /// Returns whether `name` is a drive-letter name (`C` or `C:`), which logical drives announce.
    #[must_use]
    pub fn is_drive_letter_name(name: &str) -> bool {
        matches!(name.as_bytes(), [letter] | [letter, b':'] if letter.is_ascii_alphabetic())
    }

    /// Returns the logical drive owning an RDPDR device ID.
    pub fn from_device_id(device_id: u32) -> Option<Self> {
        let index = u8::try_from(device_id.checked_sub(1)?).ok()?;
        (index < LOGICAL_DRIVE_COUNT).then_some(Self { index })
    }

    /// Returns the uppercase drive letter.
    #[must_use]
    pub fn letter(self) -> char {
        char::from(b'A' + self.index)
    }

    /// Returns the RDPDR filesystem device ID reserved for this letter.
    #[must_use]
    pub fn device_id(self) -> u32 {
        u32::from(self.index) + 1
    }

    /// Returns the drive name announced to the server (`C:`).
    #[must_use]
    pub fn display_name(self) -> String {
        format!("{}:", self.letter())
    }

    /// Returns the volume root path (`C:\`).
    #[must_use]
    pub fn root_path(self) -> PathBuf {
        PathBuf::from(format!(r"{}:\", self.letter()))
    }

    /// Returns the drive definition registered with a backend factory.
    #[must_use]
    pub fn redirected_drive(self) -> RedirectedDrive {
        RedirectedDrive::new(self.device_id(), self.display_name(), self.root_path(), false)
            .expect("logical drive IDs are nonzero and their names are nonempty")
    }

    fn bit(self) -> u32 {
        1 << self.index
    }
}

impl fmt::Display for LogicalDrive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.letter())
    }
}

/// A set of logical drive letters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogicalDriveSet {
    /// INVARIANT: only the low `LOGICAL_DRIVE_COUNT` bits are set.
    mask: u32,
}

impl LogicalDriveSet {
    /// Every drive letter from `A:` to `Z:`.
    pub const ALL: Self = Self {
        mask: ALL_LOGICAL_DRIVES,
    };

    /// Returns the redirectable volumes currently present.
    ///
    /// A volume is present when it has a root directory and a mounted file system, so empty
    /// optical or card-reader drives are skipped until media is inserted.
    pub fn present() -> Self {
        Self::ALL.present_subset()
    }

    /// Returns the drives of `self` that are present, probing only those volumes.
    fn present_subset(self) -> Self {
        // SAFETY: `GetLogicalDrives` takes no arguments.
        let mask = unsafe { GetLogicalDrives() } & ALL_LOGICAL_DRIVES;
        if mask == 0 {
            warn!("Unable to enumerate logical drives");
        }
        Self::from_mask(mask & self.mask)
            .iter()
            .filter(|drive| volume_is_ready(*drive))
            .collect()
    }

    fn from_mask(mask: u32) -> Self {
        Self {
            mask: mask & ALL_LOGICAL_DRIVES,
        }
    }

    /// Returns whether `drive` is in the set.
    #[must_use]
    pub fn contains(self, drive: LogicalDrive) -> bool {
        self.mask & drive.bit() != 0
    }

    /// Adds `drive` to the set.
    pub fn insert(&mut self, drive: LogicalDrive) {
        self.mask |= drive.bit();
    }

    /// Removes `drive` from the set.
    pub fn remove(&mut self, drive: LogicalDrive) {
        self.mask &= !drive.bit();
    }

    /// Returns whether the set is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.mask == 0
    }

    /// Returns the drives in both sets.
    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        Self::from_mask(self.mask & other.mask)
    }

    /// Returns the drives in either set.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self::from_mask(self.mask | other.mask)
    }

    /// Returns the drives in `self` but not in `other`.
    #[must_use]
    pub fn difference(self, other: Self) -> Self {
        Self::from_mask(self.mask & !other.mask)
    }

    /// Iterates the drives in alphabetical order.
    pub fn iter(self) -> impl Iterator<Item = LogicalDrive> {
        (0..LOGICAL_DRIVE_COUNT)
            .map(|index| LogicalDrive { index })
            .filter(move |drive| self.contains(*drive))
    }
}

impl FromIterator<LogicalDrive> for LogicalDriveSet {
    fn from_iter<T: IntoIterator<Item = LogicalDrive>>(iter: T) -> Self {
        let mut set = Self::default();
        for drive in iter {
            set.insert(drive);
        }
        set
    }
}

/// Drive letters selected for redirection, shared by a backend factory and its hotplug watcher.
///
/// The factory announces the selected letters that are mapped when each connection starts.
/// A [`LogicalDriveWatcher`] adds letters that arrive during the session, so a reconnect keeps
/// announcing them.
#[derive(Clone, Debug, Default)]
pub struct LogicalDriveSelection {
    mask: Arc<AtomicU32>,
}

impl LogicalDriveSelection {
    /// Creates a selection of `drives`.
    pub fn new(drives: LogicalDriveSet) -> Self {
        Self {
            mask: Arc::new(AtomicU32::new(drives.mask)),
        }
    }

    /// Creates a selection of every letter, or of the given letters; non-letters are ignored.
    pub fn from_letters(all: bool, letters: impl IntoIterator<Item = char>) -> Self {
        Self::new(if all {
            LogicalDriveSet::ALL
        } else {
            letters.into_iter().filter_map(LogicalDrive::from_letter).collect()
        })
    }

    /// Returns the selected letters.
    #[must_use]
    pub fn get(&self) -> LogicalDriveSet {
        LogicalDriveSet::from_mask(self.mask.load(Ordering::Acquire))
    }

    /// Adds `drive` to the selection.
    pub fn insert(&self, drive: LogicalDrive) {
        self.mask.fetch_or(drive.bit(), Ordering::AcqRel);
    }

    /// Returns the selected letters that are mapped now, without probing their volumes.
    ///
    /// This only reads the logical-drive mask, so it never blocks on an unreachable network share
    /// or an empty removable drive. Like mstsc, such drives are announced and their requests fail
    /// until the volume is available.
    #[must_use]
    pub fn mapped(&self) -> LogicalDriveSet {
        // SAFETY: `GetLogicalDrives` takes no arguments.
        let mask = unsafe { GetLogicalDrives() };
        self.get().intersection(LogicalDriveSet::from_mask(mask))
    }
}

/// A change in the selected logical drives reported by [`LogicalDriveWatcher`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalDriveChange {
    /// A volume was mounted under this letter.
    Arrived(LogicalDrive),
    /// A selected volume was removed.
    Removed(LogicalDrive),
}

/// Watches for volumes mounted and removed during a session (dynamic drives).
///
/// A hidden window on a dedicated thread receives the `WM_DEVICECHANGE` volume broadcasts, and a
/// short `GetLogicalDrives` poll catches network drives, which are not broadcast. Mapped letters
/// whose volume is not ready are probed again periodically.
///
/// A volume counts as connected later when its letter is selected, was not mapped when the watcher
/// started, or is announced by a volume arrival broadcast (a USB stick or inserted media). Such
/// arrivals are added to the shared selection and reported; a removal is reported when the letter
/// is selected. A network share mapped before the session that only becomes reachable later is not
/// redirected unless selected.
///
/// The callback must not block. It returns `false` when it could not deliver a change, which is
/// then reported again on the next poll. Dropping the watcher stops the thread.
pub struct LogicalDriveWatcher {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl LogicalDriveWatcher {
    /// Starts watching, reporting changes relative to the volumes present now.
    pub fn spawn(
        selection: LogicalDriveSelection,
        on_change: impl FnMut(LogicalDriveChange) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("ironrdp-rdpdr-drive-watcher".to_owned())
            .spawn(move || run_watcher(selection, Box::new(on_change), ready_sender))?;
        match ready_receiver.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(io::Error::other("logical drive watcher thread exited during startup"))
            }
        }
    }
}

impl fmt::Debug for LogicalDriveWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogicalDriveWatcher")
            .field("thread_id", &self.thread_id)
            .finish_non_exhaustive()
    }
}

impl Drop for LogicalDriveWatcher {
    fn drop(&mut self) {
        // SAFETY: posting a message to a thread ID has no memory-safety preconditions.
        if let Err(error) = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) } {
            warn!(%error, "Unable to stop the logical drive watcher");
            return;
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            warn!("Logical drive watcher thread panicked");
        }
    }
}

type ChangeCallback = Box<dyn FnMut(LogicalDriveChange) -> bool + Send>;

fn run_watcher(
    selection: LogicalDriveSelection,
    mut on_change: ChangeCallback,
    ready: mpsc::SyncSender<io::Result<u32>>,
) {
    let window = match create_watcher_window() {
        Ok(window) => window,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    // SAFETY: `window` is a live window owned by this thread, and no timer callback is used.
    if unsafe { SetTimer(Some(window), 1, LOGICAL_DRIVE_POLL_INTERVAL_MS, None) } == 0 {
        warn!(error = %io::Error::last_os_error(), "Unable to start logical drive polling");
    }

    // SAFETY: `GetLogicalDrives` takes no arguments.
    let mut letters = unsafe { GetLogicalDrives() };
    // Letters mapped before the session are not "connected later" until they are unmapped or
    // announced by a volume arrival broadcast.
    let mut preexisting = LogicalDriveSet::from_mask(letters);
    // Report readiness before probing volumes, which can block on unreachable network shares.
    // SAFETY: `GetCurrentThreadId` takes no arguments. The thread has a message queue because it
    // owns `window`, so `WM_QUIT` posted to this ID reaches the loop below.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    let mut present = LogicalDriveSet::present();
    debug!(drives = ?present.iter().map(LogicalDrive::letter).collect::<String>(), "Watching logical drives");

    let mut polls = 0u32;
    let mut undelivered = false;
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is a valid writable `MSG`.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 == 0 {
            break;
        }
        if result.0 == -1 {
            warn!(error = %io::Error::last_os_error(), "Logical drive watcher message loop failed");
            break;
        }

        let rescan = match message.message {
            WM_RESCAN_LOGICAL_DRIVES => true,
            WM_TIMER => {
                polls = polls.wrapping_add(1);
                false
            }
            _ => {
                // SAFETY: `message` was filled by `GetMessageW`.
                unsafe { DispatchMessageW(&message) };
                continue;
            }
        };

        // SAFETY: `GetLogicalDrives` takes no arguments.
        let current_letters = unsafe { GetLogicalDrives() };
        let arrived = if rescan {
            LogicalDriveSet::from_mask(u32::try_from(message.wParam.0).unwrap_or(0))
        } else {
            LogicalDriveSet::default()
        };
        preexisting = preexisting
            .intersection(LogicalDriveSet::from_mask(current_letters))
            .difference(arrived);
        let ineligible = preexisting.difference(selection.get());
        // On a poll, probe every volume only when the letter mask changes or a change is
        // undelivered: the probe can block on slow network drives, and media changes are broadcast.
        let letters_changed = core::mem::replace(&mut letters, current_letters) != current_letters;
        let current = if rescan || letters_changed || undelivered {
            Some(LogicalDriveSet::present())
        } else if polls.is_multiple_of(UNREADY_RETRY_POLLS) {
            let unready = LogicalDriveSet::from_mask(current_letters)
                .difference(present)
                .difference(ineligible);
            (!unready.is_empty()).then(|| present.union(unready.present_subset()))
        } else {
            None
        };
        if let Some(current) = current {
            (present, undelivered) = report_changes(present, current, ineligible, &selection, &mut on_change);
        }
    }

    // SAFETY: `window` is owned by this thread and has not been destroyed.
    if let Err(error) = unsafe { DestroyWindow(window) } {
        debug!(%error, "Unable to destroy the logical drive watcher window");
    }
}

/// Reports the transition from `previous` to `current` and returns the volumes now known to the
/// receiver, plus whether a change was not delivered.
///
/// Arrivals in `ineligible` are recorded without being reported. An undelivered change keeps its
/// drive in the previous state, so the next scan reports it again.
fn report_changes(
    previous: LogicalDriveSet,
    current: LogicalDriveSet,
    ineligible: LogicalDriveSet,
    selection: &LogicalDriveSelection,
    on_change: &mut dyn FnMut(LogicalDriveChange) -> bool,
) -> (LogicalDriveSet, bool) {
    let mut reported = previous;
    let mut undelivered = false;
    for drive in current.difference(previous).iter() {
        if ineligible.contains(drive) {
            reported.insert(drive);
            continue;
        }
        selection.insert(drive);
        debug!(%drive, "Logical drive arrived");
        if on_change(LogicalDriveChange::Arrived(drive)) {
            reported.insert(drive);
        } else {
            undelivered = true;
        }
    }
    for drive in previous.difference(current).iter() {
        if selection.get().contains(drive) {
            debug!(%drive, "Logical drive removed");
            if !on_change(LogicalDriveChange::Removed(drive)) {
                undelivered = true;
                continue;
            }
        }
        reported.remove(drive);
    }
    (reported, undelivered)
}

fn create_watcher_window() -> io::Result<HWND> {
    const CLASS_NAME: PCWSTR = w!("IronRdpRdpdrLogicalDriveWatcher");

    // SAFETY: a null module name selects the current executable, which outlives the window.
    let instance = unsafe { GetModuleHandleW(None) }.map_err(io::Error::from)?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(watcher_window_proc),
        hInstance: instance.into(),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialized and its class name is a static string.
    if unsafe { RegisterClassW(&class) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(i32::try_from(ERROR_CLASS_ALREADY_EXISTS.0).expect("error code fits in i32")) {
            return Err(error);
        }
    }

    // Volume broadcasts only reach top-level windows, so this cannot be a message-only window.
    // SAFETY: the class is registered for this module and every pointer argument is valid or null.
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("IronRDP drive watcher"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(io::Error::from)
}

extern "system" fn watcher_window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_DEVICECHANGE {
        let arrived = if usize::try_from(DBT_DEVICEARRIVAL).is_ok_and(|event| wparam.0 == event) {
            // SAFETY: for `DBT_DEVICEARRIVAL`, `lparam` is null or points to a broadcast header that
            // stays valid while this message is processed.
            unsafe { arrived_volume_mask(lparam) }
        } else {
            0
        };
        // Broadcasts are sent, not posted, so queue the rescan for the message loop.
        // SAFETY: `window` is the live window receiving this message.
        if let Err(error) = unsafe {
            PostMessageW(
                Some(window),
                WM_RESCAN_LOGICAL_DRIVES,
                WPARAM(usize::try_from(arrived).unwrap_or(0)),
                LPARAM(0),
            )
        } {
            warn!(%error, "Unable to queue a logical drive rescan");
        }
        return LRESULT(1);
    }
    // SAFETY: forwards the unmodified message parameters to the default window procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Returns the drive letters announced by a `DBT_DEVICEARRIVAL` volume broadcast.
///
/// # Safety
///
/// `lparam` must be null or point to a valid `DEV_BROADCAST_HDR` followed by `dbch_size` bytes.
unsafe fn arrived_volume_mask(lparam: LPARAM) -> u32 {
    let header = core::ptr::with_exposed_provenance::<DEV_BROADCAST_HDR>(usize::from_ne_bytes(lparam.0.to_ne_bytes()));
    if header.is_null() {
        return 0;
    }
    // SAFETY: the caller guarantees that a non-null `header` points to a broadcast header.
    let header_value = unsafe { header.read_unaligned() };
    let is_volume = header_value.dbch_devicetype == DBT_DEVTYP_VOLUME
        && usize::try_from(header_value.dbch_size).is_ok_and(|size| size_of::<DEV_BROADCAST_VOLUME>() <= size);
    if !is_volume {
        return 0;
    }
    // SAFETY: a volume broadcast of at least this size is a `DEV_BROADCAST_VOLUME`.
    unsafe { header.cast::<DEV_BROADCAST_VOLUME>().read_unaligned() }.dbcv_unitmask & ALL_LOGICAL_DRIVES
}

fn volume_is_ready(drive: LogicalDrive) -> bool {
    let root = HSTRING::from(drive.root_path().as_os_str());
    // Fail instead of showing "insert a disk" dialogs for empty removable drives.
    let mut previous_mode = THREAD_ERROR_MODE::default();
    // SAFETY: `previous_mode` is a valid writable error-mode slot.
    let restore = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut previous_mode)) }.is_ok();

    // Types 0 and 1 are DRIVE_UNKNOWN and DRIVE_NO_ROOT_DIR.
    // SAFETY: `root` is a NUL-terminated path that outlives the call.
    let ready = unsafe { GetDriveTypeW(&root) } > 1
        // SAFETY: `root` outlives the call, and no output buffer is requested.
        && unsafe { GetVolumeInformationW(&root, None, None, None, None, None) }.is_ok();

    if restore {
        // SAFETY: restores the mode returned by the previous call.
        let _ = unsafe { SetThreadErrorMode(previous_mode, None) };
    }
    ready
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_drives_own_stable_device_ids() {
        let drive = LogicalDrive::from_letter('c').expect("valid drive letter");
        assert_eq!(drive.letter(), 'C');
        assert_eq!(drive.device_id(), 3);
        assert_eq!(drive.display_name(), "C:");
        assert_eq!(drive.root_path(), PathBuf::from(r"C:\"));
        assert_eq!(LogicalDrive::from_device_id(3), Some(drive));
        assert_eq!(LogicalDrive::from_device_id(1), LogicalDrive::from_letter('A'));
        assert_eq!(LogicalDrive::from_device_id(26), LogicalDrive::from_letter('Z'));

        for invalid in ['@', '[', '1', '\u{e9}'] {
            assert_eq!(LogicalDrive::from_letter(invalid), None, "{invalid}");
        }
        for invalid in [0, FIRST_NAMED_DRIVE_DEVICE_ID, u32::MAX] {
            assert_eq!(LogicalDrive::from_device_id(invalid), None, "{invalid}");
        }
        assert_eq!(
            LogicalDrive::from_letter('Z').expect("valid drive letter").device_id() + 1,
            FIRST_NAMED_DRIVE_DEVICE_ID
        );
    }

    #[test]
    fn drive_letter_names_are_recognized() {
        assert!(LogicalDrive::is_drive_letter_name("C"));
        assert!(LogicalDrive::is_drive_letter_name("z:"));
        assert!(!LogicalDrive::is_drive_letter_name("home"));
        assert!(!LogicalDrive::is_drive_letter_name(r"C:\"));
        assert!(!LogicalDrive::is_drive_letter_name("1:"));
    }

    #[test]
    fn selections_are_built_from_letters() {
        let selection = LogicalDriveSelection::from_letters(false, ['c', '1', 'E']);
        assert_eq!(
            selection.get().iter().map(LogicalDrive::letter).collect::<String>(),
            "CE"
        );
        assert_eq!(
            LogicalDriveSelection::from_letters(true, []).get(),
            LogicalDriveSet::ALL
        );
    }

    #[test]
    fn logical_drive_sets_compare_letters() {
        let letter = |letter| LogicalDrive::from_letter(letter).expect("valid drive letter");
        let before = [letter('C'), letter('E')].into_iter().collect::<LogicalDriveSet>();
        let after = [letter('C'), letter('F')].into_iter().collect::<LogicalDriveSet>();

        assert_eq!(after.difference(before).iter().collect::<Vec<_>>(), [letter('F')]);
        assert_eq!(before.difference(after).iter().collect::<Vec<_>>(), [letter('E')]);
        assert_eq!(before.intersection(after).iter().collect::<Vec<_>>(), [letter('C')]);
        assert_eq!(LogicalDriveSet::ALL.iter().count(), 26);
    }

    #[test]
    fn present_logical_drives_include_the_system_drive() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let drive = LogicalDrive::from_letter(system_drive.chars().next().expect("nonempty system drive"))
            .expect("system drive is a letter");

        assert!(LogicalDriveSet::present().contains(drive));
        assert!(
            LogicalDriveSelection::new(LogicalDriveSet::ALL)
                .mapped()
                .contains(drive)
        );
        assert!(LogicalDriveSelection::default().mapped().is_empty());
    }

    #[test]
    fn undelivered_changes_are_reported_again() {
        let letter = |letter| LogicalDrive::from_letter(letter).expect("valid drive letter");
        let selection = LogicalDriveSelection::new([letter('C'), letter('E')].into_iter().collect());
        let previous = [letter('C'), letter('E')].into_iter().collect::<LogicalDriveSet>();
        let current = [letter('C'), letter('F')].into_iter().collect::<LogicalDriveSet>();

        let mut changes = Vec::new();
        let (reported, undelivered) = report_changes(
            previous,
            current,
            LogicalDriveSet::default(),
            &selection,
            &mut |change| {
                changes.push(change);
                false
            },
        );
        assert!(undelivered);
        assert_eq!(reported, previous);
        assert_eq!(
            changes,
            [
                LogicalDriveChange::Arrived(letter('F')),
                LogicalDriveChange::Removed(letter('E'))
            ]
        );
        assert!(selection.get().contains(letter('F')));

        changes.clear();
        let (reported, undelivered) = report_changes(
            reported,
            current,
            LogicalDriveSet::default(),
            &selection,
            &mut |change| {
                changes.push(change);
                true
            },
        );
        assert!(!undelivered);
        assert_eq!(reported, current);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn unselected_removals_are_not_reported() {
        let letter = |letter| LogicalDrive::from_letter(letter).expect("valid drive letter");
        let previous = [letter('C'), letter('E')].into_iter().collect::<LogicalDriveSet>();
        let current = [letter('C')].into_iter().collect::<LogicalDriveSet>();

        let (reported, undelivered) = report_changes(
            previous,
            current,
            LogicalDriveSet::default(),
            &LogicalDriveSelection::new(current),
            &mut |change| panic!("unexpected {change:?}"),
        );
        assert!(!undelivered);
        assert_eq!(reported, current);
    }

    #[test]
    fn ineligible_arrivals_are_recorded_without_redirecting() {
        let letter = |letter| LogicalDrive::from_letter(letter).expect("valid drive letter");
        let selection = LogicalDriveSelection::new([letter('C')].into_iter().collect());
        let previous = [letter('C')].into_iter().collect::<LogicalDriveSet>();
        let current = [letter('C'), letter('Z')].into_iter().collect::<LogicalDriveSet>();

        let (reported, undelivered) = report_changes(
            previous,
            current,
            [letter('Z')].into_iter().collect(),
            &selection,
            &mut |change| panic!("unexpected {change:?}"),
        );
        assert!(!undelivered);
        assert_eq!(reported, current);
        assert!(!selection.get().contains(letter('Z')));
    }

    #[test]
    fn volume_arrival_broadcasts_report_their_letters() {
        let broadcast = DEV_BROADCAST_VOLUME {
            dbcv_size: u32::try_from(size_of::<DEV_BROADCAST_VOLUME>()).expect("broadcast size fits in u32"),
            dbcv_devicetype: DBT_DEVTYP_VOLUME.0,
            dbcv_reserved: 0,
            dbcv_unitmask: 0b10100,
            dbcv_flags: Default::default(),
        };
        let lparam = LPARAM(isize::from_ne_bytes(
            core::ptr::from_ref(&broadcast).expose_provenance().to_ne_bytes(),
        ));

        // SAFETY: `lparam` points to a live volume broadcast.
        assert_eq!(unsafe { arrived_volume_mask(lparam) }, 0b10100);
        // SAFETY: a null broadcast pointer is accepted.
        assert_eq!(unsafe { arrived_volume_mask(LPARAM(0)) }, 0);
    }

    #[test]
    fn selection_is_shared_between_clones() {
        let selection = LogicalDriveSelection::default();
        let drive = LogicalDrive::from_letter('Q').expect("valid drive letter");
        selection.clone().insert(drive);
        assert!(selection.get().contains(drive));
    }

    #[test]
    fn watcher_starts_and_stops() {
        let watcher = LogicalDriveWatcher::spawn(LogicalDriveSelection::default(), |_| true).expect("start watcher");
        drop(watcher);
    }

    /// Maps and unmaps a free drive letter with `subst`, which changes the user's drive mappings.
    #[test]
    #[ignore = "changes the user's drive letters"]
    fn watcher_reports_subst_drive_arrival_and_removal() {
        use core::time::Duration;
        use std::process::Command;

        let used = LogicalDriveSet::from_mask(
            // SAFETY: `GetLogicalDrives` takes no arguments.
            unsafe { GetLogicalDrives() },
        );
        let drive = LogicalDriveSet::ALL
            .difference(used)
            .iter()
            .last()
            .expect("a free drive letter");
        let target = std::env::temp_dir();
        let (sender, receiver) = mpsc::channel();
        let selection = LogicalDriveSelection::default();
        let watcher = LogicalDriveWatcher::spawn(selection.clone(), move |change| {
            let _ = sender.send(change);
            true
        })
        .expect("start watcher");

        let status = Command::new("subst")
            .arg(drive.to_string())
            .arg(&target)
            .status()
            .expect("run subst");
        assert!(status.success());
        let arrived = receiver.recv_timeout(Duration::from_secs(10));
        let status = Command::new("subst")
            .arg(drive.to_string())
            .arg("/D")
            .status()
            .expect("run subst /D");
        assert!(status.success());

        assert_eq!(arrived, Ok(LogicalDriveChange::Arrived(drive)));
        assert!(selection.get().contains(drive));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)),
            Ok(LogicalDriveChange::Removed(drive))
        );
        drop(watcher);
    }
}
