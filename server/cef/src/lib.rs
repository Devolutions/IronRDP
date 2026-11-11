pub mod download;
pub mod model;
pub mod platform;

#[cfg(feature = "runtime")]
mod app;
#[cfg(feature = "runtime")]
mod input;
#[cfg(feature = "runtime")]
mod native;
#[cfg(feature = "runtime")]
mod rdp;

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
