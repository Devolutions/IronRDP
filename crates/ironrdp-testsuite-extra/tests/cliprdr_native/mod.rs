#[cfg(target_os = "linux")]
mod data_control;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod live;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_live;

/// Directory under the system temporary directory, removed on drop.
#[cfg(windows)]
struct TempDir(std::path::PathBuf);

#[cfg(windows)]
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ironrdp-cliprdr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(windows)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
