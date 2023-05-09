const HELP: &str = "\
cargo xtask

USAGE:
  cargo xtask [OPTIONS] [TASK]

FLAGS:
  -h, --help      Prints help information

TASKS:
  ci                Runs all checks required on CI
  check [all]       Runs all checks
  check fmt         Checks formatting
  check tests       Runs tests
  check lints       Checks lints
  check wasm        Ensures wasm module is compatible for the web
  fuzz run          Fuzz all targets for a few seconds
  fuzz corpus-min   Minify fuzzing corpus
  fuzz corpus-fetch Minify fuzzing corpus
  fuzz corpus-push  Minify fuzzing corpus
  svelte-run        Runs SvelteKit-based standalone Web Client
  coverage          Generate code-coverage data using tests and fuzz targets
  clean             Clean workspace
";

pub fn print_help() {
    println!("{HELP}");
}

pub enum Action {
    ShowHelp,
    CheckAll,
    CheckFmt,
    CheckTests,
    CheckLints,
    CheckWasm,
    FuzzRun,
    FuzzCorpusMin,
    FuzzCorpusFetch,
    FuzzCorpusPush,
    SvelteRun,
    Coverage,
    Clean,
}

pub fn parse_args() -> anyhow::Result<Action> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        match args.subcommand()?.as_deref() {
            Some("ci") => Action::CheckAll,
            Some("check") => match args.subcommand()?.as_deref() {
                Some("fmt") => Action::CheckFmt,
                Some("tests") => Action::CheckTests,
                Some("lints") => Action::CheckLints,
                Some("wasm") => Action::CheckWasm,
                Some("all") | None => Action::CheckAll,
                Some(_) => anyhow::bail!("Unknown check action"),
            },
            Some("fuzz") => match args.subcommand()?.as_deref() {
                Some("run") | None => Action::FuzzRun,
                Some("corpus-min") => Action::FuzzCorpusMin,
                Some("corpus-fetch") => Action::FuzzCorpusFetch,
                Some("corpus-push") => Action::FuzzCorpusPush,
                Some(_) => anyhow::bail!("Unknown fuzz action"),
            },
            Some("clean") => Action::Clean,
            Some("svelte-run") => Action::SvelteRun,
            Some("coverage") => Action::Coverage,
            None | Some(_) => Action::ShowHelp,
        }
    };

    Ok(action)
}
