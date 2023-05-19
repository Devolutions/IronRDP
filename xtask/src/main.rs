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

use xshell::Shell;

use crate::cli::Action;

fn main() -> anyhow::Result<()> {
    let action = match cli::parse_args() {
        Ok(action) => action,
        Err(e) => {
            cli::print_help();
            return Err(e);
        }
    };

    let sh = Shell::new()?;

    sh.change_dir(project_root());

    match action {
        Action::ShowHelp => cli::print_help(),
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
        Action::FuzzCorpusMin => fuzz::corpus_minify(&sh)?,
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

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}
