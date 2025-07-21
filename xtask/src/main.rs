#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(clippy::unwrap_used)]
#![allow(unreachable_pub)]

pub mod macros;

mod bin_install;
mod bin_version;
mod check;
mod clean;
mod cli;
mod cov;
mod ffi;
mod fuzz;
mod prelude;
mod section;
mod wasm;
mod web;

use std::path::{Path, PathBuf};

use xshell::Shell;

use crate::cli::Action;

#[cfg(target_os = "windows")]
pub const LOCAL_CARGO_ROOT: &str = ".cargo\\local_root\\";
#[cfg(not(target_os = "windows"))]
pub const LOCAL_CARGO_ROOT: &str = ".cargo/local_root/";

pub const CARGO: &str = env!("CARGO");

pub const WASM_PACKAGES: &[&str] = &["ironrdp-web"];

pub const FUZZ_TARGETS: &[&str] = &[
    "pdu_decoding",
    "rle_decompression",
    "bitmap_stream",
    "cliprdr_format",
    "channel_processing",
];

fn main() -> anyhow::Result<()> {
    let args = match cli::parse_args() {
        Ok(args) => args,
        Err(e) => {
            cli::print_help();
            return Err(e);
        }
    };

    set_verbose(args.verbose);

    let sh = new_shell()?;

    match args.action {
        Action::ShowHelp => cli::print_help(),
        Action::Bootstrap => {
            check::install(&sh)?;
            cov::install(&sh)?;
            fuzz::install(&sh)?;
            wasm::install(&sh)?;
            web::install(&sh)?;

            if is_verbose() {
                list_files(&sh, local_bin())?;
            }
        }
        Action::CheckFmt => check::fmt(&sh)?,
        Action::CheckLints => check::lints(&sh)?,
        Action::CheckLocks => check::lock_files(&sh)?,
        Action::CheckTests { no_run } => {
            if no_run {
                check::tests_compile(&sh)?;
            } else {
                check::tests_run(&sh)?;
            }
        }
        Action::CheckTypos => {
            check::typos(&sh)?;
        }
        Action::CheckInstall => {
            check::install(&sh)?;
        }
        Action::Ci => {
            check::fmt(&sh)?;
            check::typos(&sh)?;
            check::tests_compile(&sh)?;
            check::tests_run(&sh)?;
            check::lints(&sh)?;
            wasm::check(&sh)?;
            fuzz::run(&sh, None, None)?;
            web::install(&sh)?;
            web::check(&sh)?;
            check::lock_files(&sh)?;
        }
        Action::Clean => clean::workspace(&sh)?,
        Action::CovGrcov => cov::grcov(&sh)?,
        Action::CovInstall => cov::install(&sh)?,
        Action::CovReportGitHub { repo, pr } => cov::report_github(&sh, &repo, pr)?,
        Action::CovReport { html_report } => cov::report(&sh, html_report)?,
        Action::CovUpdate => cov::update(&sh)?,
        Action::FuzzCorpusFetch => fuzz::corpus_fetch(&sh)?,
        Action::FuzzCorpusMin { target } => fuzz::corpus_minify(&sh, target)?,
        Action::FuzzCorpusPush => fuzz::corpus_push(&sh)?,
        Action::FuzzInstall => fuzz::install(&sh)?,
        Action::FuzzRun { duration, target } => fuzz::run(&sh, duration, target)?,
        Action::WasmCheck => wasm::check(&sh)?,
        Action::WasmInstall => wasm::install(&sh)?,
        Action::WebCheck => web::check(&sh)?,
        Action::WebBuild => web::build(&sh, false)?,
        Action::WebInstall => web::install(&sh)?,
        Action::WebRun => web::run(&sh)?,
        Action::FfiInstall => ffi::install(&sh)?,
        Action::FfiBuildDll { release } => ffi::build_dynamic_lib(&sh, release)?,
        Action::FfiBuildBindings { skip_dotnet_build } => ffi::build_bindings(&sh, skip_dotnet_build)?,
    }

    Ok(())
}

fn new_shell() -> anyhow::Result<Shell> {
    let sh = Shell::new()?;

    sh.change_dir(project_root());
    create_folders(&sh)?;
    update_env_path(&sh)?;

    Ok(sh)
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

fn update_env_path(sh: &Shell) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let original_path = sh.var_os("PATH").context("PATH variable")?;

    let paths_to_add = vec![sh.current_dir().join(local_bin())];

    let mut new_path = std::ffi::OsString::new();

    for path in paths_to_add {
        trace!("Add {} to PATH", path.display());
        new_path.push(path.as_os_str());

        #[cfg(target_os = "windows")]
        new_path.push(";");
        #[cfg(not(target_os = "windows"))]
        new_path.push(":");
    }

    new_path.push(original_path);
    trace!("New PATH: {}", new_path.to_string_lossy());

    sh.set_var("PATH", new_path);

    Ok(())
}

fn create_folders(sh: &Shell) -> anyhow::Result<()> {
    use anyhow::Context as _;

    sh.create_dir(LOCAL_CARGO_ROOT)
        .context(format!("create directory: {LOCAL_CARGO_ROOT}"))?;

    let local_bin = local_bin();
    sh.create_dir(&local_bin)
        .context(format!("create directory: {}", local_bin.display()))?;

    Ok(())
}

pub fn local_cargo_root() -> PathBuf {
    PathBuf::from(LOCAL_CARGO_ROOT)
}

pub fn local_bin() -> PathBuf {
    let mut path = local_cargo_root();
    path.push("bin");
    path
}

static VERBOSE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn set_verbose(value: bool) {
    VERBOSE.store(value, core::sync::atomic::Ordering::Release);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(core::sync::atomic::Ordering::Acquire)
}

pub fn list_files(sh: &Shell, path: impl AsRef<Path>) -> anyhow::Result<()> {
    let path = path.as_ref();

    eprintln!("Listing folder {}:", path.display());

    for file in sh.read_dir(path)? {
        eprintln!("- {}", file.display());
    }

    Ok(())
}
