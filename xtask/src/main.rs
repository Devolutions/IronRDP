#[macro_use]
mod macros;

mod bin_version;
mod check;
mod clean;
mod cli;
mod cov;
mod fuzz;
mod prelude;
mod section;
mod wasm;
mod web;

use std::path::{Path, PathBuf};

use prelude::LOCAL_CARGO_ROOT;
use xshell::Shell;

use crate::cli::Action;

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
            cov::install(&sh)?;
            fuzz::install(&sh)?;
            wasm::install(&sh)?;
            web::install(&sh)?;

            if is_verbose() {
                list_files(&sh, local_cargo_root().join("bin"))?;
            }
        }
        Action::CheckFmt => check::fmt(&sh)?,
        Action::CheckLints => check::lints(&sh)?,
        Action::CheckTests { no_run } => {
            if no_run {
                check::tests_compile(&sh)?;
            } else {
                check::tests_run(&sh)?;
            }
        }
        Action::Ci => {
            check::fmt(&sh)?;
            check::tests_compile(&sh)?;
            check::tests_run(&sh)?;
            check::lints(&sh)?;
            wasm::check(&sh)?;
            fuzz::run(&sh, None, None)?;
            web::install(&sh)?;
            web::check(&sh)?;
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
        Action::WebInstall => web::install(&sh)?,
        Action::WebRun => web::run(&sh)?,
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

static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_verbose(value: bool) {
    VERBOSE.store(value, std::sync::atomic::Ordering::Release);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(std::sync::atomic::Ordering::Acquire)
}

/// Checks if a binary is installed in local root
pub fn is_installed(sh: &Shell, name: &str) -> bool {
    let path = local_bin().join(name);
    sh.path_exists(&path) || sh.path_exists(path.with_extension("exe"))
}

pub fn list_files(sh: &Shell, path: impl AsRef<Path>) -> anyhow::Result<()> {
    let path = path.as_ref();

    eprintln!("Listing folder {}:", path.display());

    for file in sh.read_dir(path)? {
        eprintln!("- {}", file.display());
    }

    Ok(())
}
