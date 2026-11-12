//! Calls into the process-private clipboard exported by a patched CEF runtime.
//!
//! Stock CEF does not export these functions; [`PrivateClipboard::load`] fails there.
//! Every call must run on the CEF UI thread, which is the thread pumping `do_message_loop_work`.

use std::ffi::c_int;

use anyhow::Context as _;

use crate::clipboard::{MAX_BYTES, Snapshot};
use crate::patched_runtime::Library;

/// Command-line switch that makes the patched runtime install its in-memory clipboard.
pub const SWITCH: &str = "isolated-clipboard";

const TEXT: c_int = 0;
const HTML: c_int = 1;
const PNG: c_int = 2;
const ABSENT: i64 = -1;

type Enabled = unsafe extern "C" fn() -> c_int;
type Generation = unsafe extern "C" fn() -> i64;
type Read = unsafe extern "C" fn(c_int, *mut u8, usize) -> i64;
type Write = unsafe extern "C" fn(*const u8, usize, *const u8, usize, *const u8, usize) -> c_int;
type WriteFiles = unsafe extern "C" fn(*const *const std::ffi::c_char, usize) -> c_int;

pub struct PrivateClipboard {
    generation: Generation,
    read: Read,
    write: Write,
    write_files: WriteFiles,
    /// Generation of the last snapshot read or written here.
    seen: i64,
}

impl PrivateClipboard {
    /// Resolves the patched runtime's exports and checks the private clipboard is installed.
    ///
    /// The returned functions stay valid while the CEF runtime remains loaded.
    pub fn load() -> anyhow::Result<Self> {
        let library = Library::open()?;
        // SAFETY: Each name is resolved to the signature the patched runtime declares for it.
        let (enabled, generation, read, write, write_files) = unsafe {
            (
                library.symbol::<Enabled>(c"cef_isolated_clipboard_enabled")?,
                library.symbol::<Generation>(c"cef_isolated_clipboard_generation")?,
                library.symbol::<Read>(c"cef_isolated_clipboard_read")?,
                library.symbol::<Write>(c"cef_isolated_clipboard_write")?,
                library.symbol::<WriteFiles>(c"cef_isolated_clipboard_write_files")?,
            )
        };
        // SAFETY: The caller runs on the CEF UI thread after initialization.
        anyhow::ensure!(
            unsafe { enabled() } == 1,
            "the CEF runtime did not install its private clipboard"
        );
        let mut clipboard = Self {
            generation,
            read,
            write,
            write_files,
            seen: 0,
        };
        clipboard.seen = clipboard.current_generation()?;
        Ok(clipboard)
    }

    fn current_generation(&self) -> anyhow::Result<i64> {
        // SAFETY: The function takes no arguments and runs on the CEF UI thread.
        let generation = unsafe { (self.generation)() };
        anyhow::ensure!(0 <= generation, "private clipboard generation failed with {generation}");
        Ok(generation)
    }

    /// Returns the browser's clipboard if a page changed it since the last call or write.
    pub fn take_change(&mut self) -> anyhow::Result<Option<Snapshot>> {
        let generation = self.current_generation()?;
        if generation == self.seen {
            return Ok(None);
        }
        self.seen = generation;
        let snapshot = Snapshot {
            text: self
                .read_kind(TEXT)?
                .map(String::from_utf8)
                .transpose()
                .context("private clipboard text is not valid UTF-8")?,
            html: self
                .read_kind(HTML)?
                .map(String::from_utf8)
                .transpose()
                .context("private clipboard HTML is not valid UTF-8")?,
            png: self.read_kind(PNG)?,
        };
        Ok(Some(snapshot))
    }

    fn read_kind(&self, kind: c_int) -> anyhow::Result<Option<Vec<u8>>> {
        // SAFETY: A null buffer with zero capacity only queries the length.
        let length = unsafe { (self.read)(kind, std::ptr::null_mut(), 0) };
        if length == ABSENT {
            return Ok(None);
        }
        let length =
            usize::try_from(length).map_err(|_| anyhow::anyhow!("private clipboard read failed with {length}"))?;
        if MAX_BYTES < length {
            tracing::debug!(kind, length, "Ignored oversized browser clipboard representation");
            return Ok(None);
        }
        let mut bytes = vec![0; length];
        // SAFETY: The buffer is writable for `length` bytes.
        let copied = unsafe { (self.read)(kind, bytes.as_mut_ptr(), bytes.len()) };
        // The clipboard cannot change between the two calls because both run on the UI thread.
        anyhow::ensure!(
            usize::try_from(copied) == Ok(length),
            "private clipboard changed during a read"
        );
        Ok(Some(bytes))
    }

    /// Replaces the browser's clipboard with client content.
    pub fn write(&mut self, snapshot: &Snapshot) -> anyhow::Result<()> {
        fn parts(bytes: Option<&[u8]>) -> (*const u8, usize) {
            bytes.map_or((std::ptr::null(), 0), |bytes| (bytes.as_ptr(), bytes.len()))
        }
        let (text, text_length) = parts(snapshot.text.as_deref().map(str::as_bytes));
        let (html, html_length) = parts(snapshot.html.as_deref().map(str::as_bytes));
        let (png, png_length) = parts(snapshot.png.as_deref());
        // SAFETY: Each non-null pointer is readable for its length; null marks an absent representation.
        let status = unsafe { (self.write)(text, text_length, html, html_length, png, png_length) };
        anyhow::ensure!(status == 0, "private clipboard write failed with {status}");
        self.seen = self.current_generation()?;
        Ok(())
    }

    /// Replaces the browser's clipboard with files that pages can paste.
    #[cfg_attr(
        not(feature = "device-simulator"),
        expect(dead_code, reason = "client files arrive with clipboard file transfer")
    )]
    ///
    /// Each path must be absolute and readable by the browser, such as a file staged from the client.
    pub fn write_files(&mut self, paths: &[&std::path::Path]) -> anyhow::Result<()> {
        let paths = paths
            .iter()
            .map(|path| {
                let path = path.to_str().context("clipboard file path is not UTF-8")?;
                Ok(std::ffi::CString::new(path)?)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let pointers: Vec<_> = paths.iter().map(|path| path.as_ptr()).collect();
        // SAFETY: Each pointer is a NUL-terminated string that outlives the call.
        let status = unsafe { (self.write_files)(pointers.as_ptr(), pointers.len()) };
        anyhow::ensure!(status == 0, "private clipboard file write failed with {status}");
        self.seen = self.current_generation()?;
        Ok(())
    }
}
