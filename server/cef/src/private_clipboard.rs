//! Calls into the process-private clipboard exported by a patched CEF runtime.
//!
//! Stock CEF does not export these functions; [`PrivateClipboard::load`] fails there.
//! Every call must run on the CEF UI thread, which is the thread pumping `do_message_loop_work`.

use std::ffi::{CStr, c_int, c_void};

use anyhow::Context as _;

use crate::clipboard::{MAX_BYTES, Snapshot};

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

pub struct PrivateClipboard {
    generation: Generation,
    read: Read,
    write: Write,
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
        let (enabled, generation, read, write) = unsafe {
            (
                library.symbol::<Enabled>(c"cef_isolated_clipboard_enabled")?,
                library.symbol::<Generation>(c"cef_isolated_clipboard_generation")?,
                library.symbol::<Read>(c"cef_isolated_clipboard_read")?,
                library.symbol::<Write>(c"cef_isolated_clipboard_write")?,
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
}

struct Library(*mut c_void);

impl Library {
    #[cfg(target_os = "macos")]
    fn open() -> anyhow::Result<Self> {
        use std::os::unix::ffi::OsStrExt as _;

        let path = std::env::current_exe()?
            .parent()
            .context("browser executable has no directory")?
            .join("../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework")
            .canonicalize()
            .context("failed to locate the CEF framework")?;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: The path is NUL-terminated; RTLD_NOLOAD only returns the framework already loaded by cef-rs.
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_NOLOAD) };
        anyhow::ensure!(!handle.is_null(), "the CEF framework is not loaded");
        Ok(Self(handle))
    }

    #[cfg(target_os = "linux")]
    fn open() -> anyhow::Result<Self> {
        Ok(Self(libc::RTLD_DEFAULT))
    }

    #[cfg(windows)]
    fn open() -> anyhow::Result<Self> {
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

        let name: Vec<u16> = "libcef.dll\0".encode_utf16().collect();
        // SAFETY: The name is NUL-terminated; the module stays loaded for the life of the process.
        let handle = unsafe { GetModuleHandleW(name.as_ptr()) };
        anyhow::ensure!(!handle.is_null(), "libcef.dll is not loaded");
        Ok(Self(handle))
    }

    /// # Safety
    ///
    /// `T` must be the function pointer type of the exported symbol.
    unsafe fn symbol<T: Copy>(&self, name: &CStr) -> anyhow::Result<T> {
        const { assert!(size_of::<T>() == size_of::<*mut c_void>()) };
        #[cfg(unix)]
        // SAFETY: The handle is a loaded image or RTLD_DEFAULT, and the name is NUL-terminated.
        let address = unsafe { libc::dlsym(self.0, name.as_ptr()) };
        #[cfg(windows)]
        // SAFETY: The handle is a loaded module, and the name is NUL-terminated.
        let address =
            unsafe { windows_sys::Win32::System::LibraryLoader::GetProcAddress(self.0, name.as_ptr().cast()) }
                .map_or(std::ptr::null_mut(), |function| function as *mut c_void);
        anyhow::ensure!(
            !address.is_null(),
            "the CEF runtime does not export {}; it needs the private clipboard patch",
            name.to_string_lossy()
        );
        // SAFETY: The caller guarantees `T` matches the symbol, and `T` is pointer-sized.
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&address) })
    }
}

#[cfg(target_os = "macos")]
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: The handle came from a successful dlopen; cef-rs keeps its own reference.
        unsafe { libc::dlclose(self.0) };
    }
}
