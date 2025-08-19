macro_rules! windows_skip {
    () => {
        if cfg!(target_os = "windows") {
            eprintln!("Skip (unsupported on windows)");
            return Ok(());
        }
    };
}

pub(crate) use windows_skip;

macro_rules! trace {
    ($($arg:tt)*) => {{
        if $crate::is_verbose() {
            eprintln!($($arg)*);
        }
    }};
}

pub(crate) use trace;

macro_rules! run_cmd_in {
    ($sh:expr, $prefix:expr, $args:literal) => {{
        let _guard = $sh.push_dir($prefix);
        eprintln!("In {}:", $sh.current_dir().display());
        ::xshell::cmd!($sh, $args).run()
    }};
}

pub(crate) use run_cmd_in;
