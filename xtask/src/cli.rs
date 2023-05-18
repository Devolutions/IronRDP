const HELP: &str = "\
cargo xtask

USAGE:
  cargo xtask [OPTIONS] [TASK]

FLAGS:
  -h, --help      Prints help information

TASKS:
  check fmt               Check formatting
  check lints             Check lints
  check tests [--no-run]  Compile tests and, unless specified otherwise, run them
  check wasm              Ensure WASM module is compatible for the web
  ci                      Run all checks required on CI
  clean                   Clean workspace
  coverage install        Install dependencies required to generate the coverage report
  coverage report         Generate code-coverage data using tests and fuzz targets
  fuzz corpus-fetch       Fetch fuzzing corpus from Azure storage
  fuzz corpus-min         Minify fuzzing corpus
  fuzz corpus-push        Push fuzzing corpus to Azure storage
  fuzz install            Install dependencies required for fuzzing
  fuzz run [--duration <SECONDS>] [--target <NAME>]
                          Fuzz a specific target if any or all targets for a limited duration (default is 5s)
  wasm check              Ensure WASM module is compatible for the web
  wasm install            Install dependencies required to build the WASM target
  web check               Ensure Web Client is building without error
  web install             Install dependencies required to build and run Web Client
  web run                 Run SvelteKit-based standalone Web Client
";

pub fn print_help() {
    println!("{HELP}");
}

pub enum Action {
    ShowHelp,
    CheckFmt,
    CheckLints,
    CheckTests {
        no_run: bool,
    },
    Ci,
    Clean,
    CovInstall,
    CovReport,
    FuzzCorpusFetch,
    FuzzCorpusMin,
    FuzzCorpusPush,
    FuzzInstall,
    FuzzRun {
        duration: Option<u32>,
        target: Option<String>,
    },
    WasmCheck,
    WasmInstall,
    WebCheck,
    WebInstall,
    WebRun,
}

pub fn parse_args() -> anyhow::Result<Action> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        match args.subcommand()?.as_deref() {
            Some("check") => match args.subcommand()?.as_deref() {
                Some("fmt") => Action::CheckFmt,
                Some("lints") => Action::CheckLints,
                Some("tests") => Action::CheckTests {
                    no_run: args.contains("--no-run"),
                },
                Some(unknown) => anyhow::bail!("unknown check action: {unknown}"),
                None => Action::ShowHelp,
            },
            Some("ci") => Action::Ci,
            Some("clean") => Action::Clean,
            Some("cov") => match args.subcommand()?.as_deref() {
                Some("install") => Action::CovInstall,
                Some("report") => Action::CovReport,
                Some(unknown) => anyhow::bail!("unknown coverage action: {unknown}"),
                None => Action::ShowHelp,
            },
            Some("fuzz") => match args.subcommand()?.as_deref() {
                Some("corpus-fetch") => Action::FuzzCorpusFetch,
                Some("corpus-min") => Action::FuzzCorpusMin,
                Some("corpus-push") => Action::FuzzCorpusPush,
                Some("install") => Action::FuzzInstall,
                Some("run") => Action::FuzzRun {
                    duration: args.opt_value_from_str("--duration")?,
                    target: args.opt_value_from_str("--target")?,
                },
                None => Action::FuzzRun {
                    duration: None,
                    target: None,
                },
                Some(unknown) => anyhow::bail!("unknown fuzz action: {unknown}"),
            },
            Some("wasm") => match args.subcommand()?.as_deref() {
                Some("check") => Action::WasmCheck,
                Some("install") => Action::WasmInstall,
                Some(unknown) => anyhow::bail!("unknown wasm action: {unknown}"),
                None => Action::ShowHelp,
            },
            Some("web") => match args.subcommand()?.as_deref() {
                Some("check") => Action::WebCheck,
                Some("install") => Action::WebInstall,
                Some("run") => Action::WebRun,
                Some(unknown) => anyhow::bail!("unknown web action: {unknown}"),
                None => Action::ShowHelp,
            },
            None | Some(_) => Action::ShowHelp,
        }
    };

    Ok(action)
}
