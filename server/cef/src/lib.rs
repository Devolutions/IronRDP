#[cfg(feature = "runtime")]
mod audio;
#[cfg(feature = "runtime")]
mod camera;
#[cfg(feature = "runtime")]
pub mod clipboard;
#[cfg(feature = "runtime")]
mod cursor;
pub mod download;
pub mod model;
pub mod platform;

#[cfg(feature = "runtime")]
mod app;
#[cfg(feature = "device-simulator")]
mod device_simulator;
#[cfg(feature = "runtime")]
mod egfx;
#[cfg(feature = "runtime")]
mod input;
#[cfg(feature = "runtime")]
mod location;
#[cfg(feature = "runtime")]
mod native;
#[cfg(feature = "runtime")]
mod patched_runtime;
#[cfg(feature = "runtime")]
mod private_clipboard;
#[cfg(feature = "runtime")]
mod rdp;
#[cfg(feature = "runtime")]
mod remote_devices;
#[cfg(feature = "runtime")]
mod sound;
#[cfg(feature = "runtime")]
mod webauthn;

#[cfg(all(feature = "runtime", not(windows)))]
pub fn run() -> anyhow::Result<()> {
    app::run(cef::args::Args::new().as_main_args(), std::ptr::null_mut())
}

#[cfg(all(feature = "runtime", windows))]
#[unsafe(export_name = "RunConsoleMain")]
// UNTESTED: Windows must enter through bootstrapc.exe with its sandbox context intact.
// The bootstrap owns sandbox initialization; verify renderer sandboxing and orderly shutdown on Windows.
unsafe extern "C" fn run_console_main(
    _argc: std::ffi::c_int,
    _argv: *mut *mut std::ffi::c_char,
    sandbox_info: *mut std::ffi::c_void,
    _version_info: *mut std::ffi::c_void,
) -> i32 {
    if sandbox_info.is_null() {
        eprintln!("Windows CEF requires sandbox information from the bootstrap executable");
        return 1;
    }
    // SAFETY: `RunConsoleMain` is single-threaded before `cef::execute_process` or Tokio starts,
    // and runs in both the Windows browser process and every child subprocess.
    unsafe {
        std::env::set_var("TZ", "UTC");
        std::env::set_var("CEF_ISOLATE_TIMEZONE", "1");
    }
    let args = cef::args::Args::new();
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let code = cef::execute_process(Some(args.as_main_args()), None::<&mut cef::App>, sandbox_info.cast());
    if 0 <= code {
        return code;
    }
    match app::run(args.as_main_args(), sandbox_info.cast()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    }
}
