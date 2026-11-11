#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    let args = cef::args::Args::new();
    #[cfg(target_os = "macos")]
    let mut sandbox = cef::sandbox::Sandbox::new();
    #[cfg(target_os = "macos")]
    sandbox.initialize(args.as_main_args());
    #[cfg(target_os = "macos")]
    let loader =
        cef::library_loader::LibraryLoader::new(&std::env::current_exe().expect("helper executable path"), true);
    #[cfg(target_os = "macos")]
    assert!(loader.load(), "failed to load CEF in helper");
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let result = cef::execute_process(Some(args.as_main_args()), None::<&mut cef::App>, std::ptr::null_mut());
    std::process::ExitCode::from(u8::try_from(result).unwrap_or(1))
}

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    eprintln!("Windows uses the packaged CEF sandbox bootstrap for subprocesses");
    std::process::ExitCode::FAILURE
}
