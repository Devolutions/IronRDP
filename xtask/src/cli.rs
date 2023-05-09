const HELP: &str = "\
cargo xtask

USAGE:
  cargo xtask [OPTIONS] [TASK]

FLAGS:
  -h, --help      Prints help information

TASKS:
  check [all]       Runs all checks
  check fmt         Checks formatting
  check lints       Checks lints
  check tests       Runs tests
  check wasm        Ensures wasm module is compatible for the web
  ci                Runs all checks required on CI
  clean             Clean workspace
  coverage          Generate code-coverage data using tests and fuzz targets
  fuzz corpus-fetch Minify fuzzing corpus
  fuzz corpus-min   Minify fuzzing corpus
  fuzz corpus-push  Minify fuzzing corpus
  fuzz install      Install dependencies required for fuzzing
  fuzz run          Fuzz all targets for a few seconds
  svelte-run        Runs SvelteKit-based standalone Web Client
  wasm install      Install dependencies required to build the wasm target
";

pub fn print_help() {
    println!("{HELP}");
}

pub enum Action {
    CheckAll,
    CheckFmt,
    CheckLints,
    CheckTests,
    CheckWasm,
    Clean,
    Coverage,
    FuzzCorpusFetch,
    FuzzCorpusMin,
    FuzzCorpusPush,
    FuzzInstall,
    FuzzRun, // TODO: add option to choose the target and run duration so we can optimize CI further
    ShowHelp,
    SvelteRun,
    WasmInstall,
}

pub fn parse_args() -> anyhow::Result<Action> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        match args.subcommand()?.as_deref() {
            Some("ci") => Action::CheckAll,
            Some("check") => match args.subcommand()?.as_deref() {
                Some("all") | None => Action::CheckAll,
                Some("fmt") => Action::CheckFmt,
                Some("lints") => Action::CheckLints,
                Some("tests") => Action::CheckTests,
                Some("wasm") => Action::CheckWasm,
                Some(_) => anyhow::bail!("Unknown check action"),
            },
            Some("clean") => Action::Clean,
            Some("coverage") => Action::Coverage,
            Some("fuzz") => match args.subcommand()?.as_deref() {
                Some("corpus-fetch") => Action::FuzzCorpusFetch,
                Some("corpus-min") => Action::FuzzCorpusMin,
                Some("corpus-push") => Action::FuzzCorpusPush,
                Some("run") | None => Action::FuzzRun,
                Some("install") => Action::FuzzInstall,
                Some(_) => anyhow::bail!("Unknown fuzz action"),
            },
            Some("svelte-run") => Action::SvelteRun,
            Some("wasm") => match args.subcommand()?.as_deref() {
                Some("install") => Action::WasmInstall,
                Some(_) => anyhow::bail!("Unknown wasm action"),
                None => Action::ShowHelp,
            },
            None | Some(_) => Action::ShowHelp,
        }
    };

    Ok(action)
}
