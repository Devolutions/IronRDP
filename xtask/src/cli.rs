const HELP: &str = "\
cargo xtask

USAGE:
  cargo xtask [OPTIONS] [TASK]

FLAGS:
  -h, --help      Prints help information
  -v, --verbose   Prints additional execution traces

TASKS:
  bootstrap               Install all requirements for development
  check fmt               Check formatting
  check lints             Check lints
  check locks             Check for dirty or staged lock files not yet committed
  check tests [--no-run]  Compile tests and, unless specified otherwise, run them
  ci                      Run all checks required on CI
  clean                   Clean workspace
  cov grcov               Generate a nice HTML report using code-coverage data from tests and fuzz targets
  cov install             Install cargo-llvm-cov in cargo local root
  cov report-gh --repo <REPO_NAME> --pr <PR_ID>
                          Generate a coverage report, posting a comment in GitHub PR
  cov report [--html]     Generate a coverage report (optionally, a HTML report)
  cov update              Update coverage data in the cov-data branch
  fuzz corpus-fetch       Fetch fuzzing corpus from Azure storage
  fuzz corpus-min [--target <NAME>]
                          Minify fuzzing corpus for a specific target (or all if unspecified)
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

pub struct Args {
    pub verbose: bool,
    pub action: Action,
}

pub enum Action {
    ShowHelp,
    Bootstrap,
    CheckFmt,
    CheckLints,
    CheckLocks,
    CheckTests {
        no_run: bool,
    },
    Ci,
    Clean,
    CovGrcov,
    CovInstall,
    CovReportGitHub {
        repo: String,
        pr: u32,
    },
    CovReport {
        html_report: bool,
    },
    CovUpdate,
    FuzzCorpusFetch,
    FuzzCorpusMin {
        target: Option<String>,
    },
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

pub fn parse_args() -> anyhow::Result<Args> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        match args.subcommand()?.as_deref() {
            Some("bootstrap") => Action::Bootstrap,
            Some("check") => match args.subcommand()?.as_deref() {
                Some("fmt") => Action::CheckFmt,
                Some("lints") => Action::CheckLints,
                Some("locks") => Action::CheckLocks,
                Some("tests") => Action::CheckTests {
                    no_run: args.contains("--no-run"),
                },
                Some(unknown) => anyhow::bail!("unknown check action: {unknown}"),
                None => Action::ShowHelp,
            },
            Some("ci") => Action::Ci,
            Some("clean") => Action::Clean,
            Some("cov") => match args.subcommand()?.as_deref() {
                Some("grcov") => Action::CovGrcov,
                Some("install") => Action::CovInstall,
                Some("report-gh") => Action::CovReportGitHub {
                    repo: args.value_from_str("--repo")?,
                    pr: args.value_from_str("--pr")?,
                },
                Some("report") => Action::CovReport {
                    html_report: args.contains("--html"),
                },
                Some("update") => Action::CovUpdate,
                None | Some(_) => anyhow::bail!("Unknown cov action"),
            },
            Some("fuzz") => match args.subcommand()?.as_deref() {
                Some("corpus-fetch") => Action::FuzzCorpusFetch,
                Some("corpus-min") => Action::FuzzCorpusMin {
                    target: args.opt_value_from_str("--target")?,
                },
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

    let verbose = args.contains(["-v", "--verbose"]);

    Ok(Args { verbose, action })
}
