mod cli;
mod section;
mod tasks;

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
        Action::CheckAll => {
            tasks::check_formatting(&sh)?;
            tasks::run_tests(&sh)?;
            tasks::check_lints(&sh)?;
            tasks::check_wasm(&sh)?;
            tasks::fuzz_run(&sh, None, None)?;
        }
        Action::CheckFmt => tasks::check_formatting(&sh)?,
        Action::CheckLints => tasks::check_lints(&sh)?,
        Action::CheckTests => tasks::run_tests(&sh)?,
        Action::CheckWasm => tasks::check_wasm(&sh)?,
        Action::Clean => tasks::clean_workspace(&sh)?,
        Action::CoverageInstall => tasks::coverage_install(&sh)?,
        Action::CoverageReport => tasks::coverage_report(&sh)?,
        Action::FuzzCorpusFetch => tasks::fuzz_corpus_fetch(&sh)?,
        Action::FuzzCorpusMin => tasks::fuzz_corpus_minify(&sh)?,
        Action::FuzzCorpusPush => tasks::fuzz_corpus_push(&sh)?,
        Action::FuzzInstall => tasks::fuzz_install(&sh)?,
        Action::FuzzRun { duration, target } => tasks::fuzz_run(&sh, duration, target)?,
        Action::SvelteRun => tasks::svelte_run(&sh)?,
        Action::WasmInstall => tasks::wasm_install(&sh)?,
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
