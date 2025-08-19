use xshell::{cmd, Shell};

use crate::macros::trace;
use crate::{local_bin, CARGO, LOCAL_CARGO_ROOT};

pub struct CargoPackage {
    pub name: &'static str,
    pub binary_name: &'static str,
    pub version: &'static str,
}

impl CargoPackage {
    pub const fn new(name: &'static str, version: &'static str) -> Self {
        Self {
            name,
            binary_name: name,
            version,
        }
    }

    pub const fn with_binary_name(self, name: &'static str) -> Self {
        Self {
            binary_name: name,
            ..self
        }
    }
}

pub fn cargo_install(sh: &Shell, package: &CargoPackage) -> anyhow::Result<()> {
    let package_name = package.name;
    let package_version = package.version;

    if is_installed(sh, package) {
        trace!("{package_name} is already installed");
        return Ok(());
    }

    if cargo_binstall_is_available(sh) {
        trace!("cargo-binstall is available");
        cmd!(
            sh,
            "{CARGO} binstall --no-confirm --root {LOCAL_CARGO_ROOT} {package_name}@{package_version}"
        )
        .run()?;
    } else {
        trace!("Install {package_name} using cargo install");
        // Install in debug because it's faster to compile and we typically don't need execution speed anyway.
        cmd!(
            sh,
            "{CARGO} install --debug --locked --root {LOCAL_CARGO_ROOT} {package_name}@{package_version}"
        )
        .run()?;
    }

    Ok(())
}

fn cargo_binstall_is_available(sh: &Shell) -> bool {
    cmd!(sh, "{CARGO} binstall -h")
        .quiet()
        .ignore_stderr()
        .ignore_stdout()
        .run()
        .is_ok()
}

/// Checks if a binary is installed in local root
pub fn is_installed(sh: &Shell, package: impl GetBinaryName) -> bool {
    let path = local_bin().join(package.binary_name());
    sh.path_exists(&path) || sh.path_exists(path.with_extension("exe"))
}

#[doc(hidden)]
pub trait GetBinaryName {
    fn binary_name(self) -> &'static str;
}

impl GetBinaryName for &CargoPackage {
    fn binary_name(self) -> &'static str {
        self.binary_name
    }
}

impl GetBinaryName for &'static str {
    fn binary_name(self) -> &'static str {
        self
    }
}
