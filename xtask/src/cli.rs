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
  check dependencies      Check dependency-graph invariants between crates
  check tests [--no-run]  Compile tests and, unless specified otherwise, run them
  check typos             Check for typos in the codebase
  check features          Run every feature-matrix case sequentially
  check features --case <NAME>
                          Run a single feature-matrix case
  check features --list [--format <FMT>]
                          List feature-matrix cases (fmt: human (default) | github-matrix)
  check install           Install all requirements for check tasks
  ci                      Run all checks required on CI
  clean                   Clean workspace
  fuzz corpus-fetch       Fetch fuzzing corpus from Azure storage
  fuzz corpus-min [--target <NAME>]
                          Minify fuzzing corpus for a specific target (or all if unspecified)
  fuzz corpus-push        Push fuzzing corpus to Azure storage
  fuzz install            Install dependencies required for fuzzing
  fuzz list [--format <FMT>]
                          List fuzz targets (fmt: human (default) | github-matrix)
  fuzz run [--duration <SECONDS>] [--target <NAME>]
                          Fuzz a specific target if any or all targets for a limited duration (default is 5s)
  wasm check              Ensure WASM module is compatible for the web
  wasm install            Install dependencies required to build the WASM target
  web check               Ensure Web Client is building without error
  web install             Install dependencies required to build and run Web Client
  web build               Build the Web Client
  web run                 Run SvelteKit-based standalone Web Client
  ffi install             Install all requirements for ffi tasks
  ffi build [--release]   Build DLL for FFI (default is debug)
  ffi bindings [--skip-dotnet-build]            
                          Generate C# bindings for FFI, optionally skipping the .NET build
";

pub fn print_help() {
    println!("{HELP}");
}

pub struct Args {
    pub verbose: bool,
    pub action: Action,
}

pub enum ListFormat {
    Human,
    GithubMatrix,
}

impl ListFormat {
    pub const DEFAULT: Self = Self::Human;
}

impl core::str::FromStr for ListFormat {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "human" => Ok(Self::Human),
            "github-matrix" => Ok(Self::GithubMatrix),
            other => anyhow::bail!("unknown --format value: {other}"),
        }
    }
}

pub enum Action {
    ShowHelp,
    Bootstrap,
    CheckFmt,
    CheckLints,
    CheckLocks,
    CheckDependencies,
    CheckTests {
        no_run: bool,
    },
    CheckTypos,
    CheckFeatures {
        case: Option<String>,
        list: bool,
        format: ListFormat,
    },
    CheckInstall,
    Ci,
    Clean,
    FuzzCorpusFetch,
    FuzzCorpusMin {
        target: Option<String>,
    },
    FuzzCorpusPush,
    FuzzInstall,
    FuzzList {
        format: ListFormat,
    },
    FuzzRun {
        duration: Option<u32>,
        target: Option<String>,
    },
    WasmCheck,
    WasmInstall,
    WebCheck,
    WebInstall,
    WebBuild,
    WebRun,
    FfiInstall,
    FfiBuildDll {
        release: bool,
    },
    FfiBuildBindings {
        skip_dotnet_build: bool,
    },
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
                Some("dependencies") => Action::CheckDependencies,
                Some("tests") => Action::CheckTests {
                    no_run: args.contains("--no-run"),
                },
                Some("typos") => Action::CheckTypos,
                Some("features") => Action::CheckFeatures {
                    case: args.opt_value_from_str("--case")?,
                    list: args.contains("--list"),
                    format: args.opt_value_from_str("--format")?.unwrap_or(ListFormat::DEFAULT),
                },
                Some("install") => Action::CheckInstall,
                Some(unknown) => anyhow::bail!("unknown check action: {unknown}"),
                None => Action::ShowHelp,
            },
            Some("ci") => Action::Ci,
            Some("clean") => Action::Clean,
            Some("fuzz") => match args.subcommand()?.as_deref() {
                Some("corpus-fetch") => Action::FuzzCorpusFetch,
                Some("corpus-min") => Action::FuzzCorpusMin {
                    target: args.opt_value_from_str("--target")?,
                },
                Some("corpus-push") => Action::FuzzCorpusPush,
                Some("install") => Action::FuzzInstall,
                Some("list") => Action::FuzzList {
                    format: args.opt_value_from_str("--format")?.unwrap_or(ListFormat::DEFAULT),
                },
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
                Some("build") => Action::WebBuild,
                Some("run") => Action::WebRun,
                Some(unknown) => anyhow::bail!("unknown web action: {unknown}"),
                None => Action::ShowHelp,
            },
            Some("ffi") => match args.subcommand()?.as_deref() {
                Some("install") => Action::FfiInstall,
                Some("build") => Action::FfiBuildDll {
                    release: args.contains("--release"),
                },
                Some("bindings") => Action::FfiBuildBindings {
                    skip_dotnet_build: args.contains("--skip-dotnet-build"),
                },
                Some(unknown) => anyhow::bail!("unknown ffi action: {unknown}"),
                None => Action::ShowHelp,
            },
            None | Some(_) => Action::ShowHelp,
        }
    };

    let verbose = args.contains(["-v", "--verbose"]);

    Ok(Args { verbose, action })
}
