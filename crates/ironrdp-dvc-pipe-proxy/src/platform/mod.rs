#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
mod unix;

#[cfg(target_os = "windows")]
pub use windows::DvcNamedPipeProxy;

#[cfg(not(target_os = "windows"))]
pub use unix::DvcNamedPipeProxy;
