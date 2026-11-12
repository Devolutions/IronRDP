//! Resolves functions that only the server's patched CEF runtime exports.

use std::ffi::{CStr, c_void};

use anyhow::Context as _;

pub(crate) struct Library(*mut c_void);

impl Library {
    #[cfg(target_os = "macos")]
    pub(crate) fn open() -> anyhow::Result<Self> {
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
    pub(crate) fn open() -> anyhow::Result<Self> {
        Ok(Self(libc::RTLD_DEFAULT))
    }

    #[cfg(windows)]
    pub(crate) fn open() -> anyhow::Result<Self> {
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
    pub(crate) unsafe fn symbol<T: Copy>(&self, name: &CStr) -> anyhow::Result<T> {
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
            "the CEF runtime does not export {}; it needs the server CEF patches",
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
