use anyhow::Context as _;
use xshell::{Shell, cmd};

use crate::macros::trace;
use crate::{CARGO, LOCAL_CARGO_ROOT, local_bin, local_cargo_root};

pub struct CargoPackage {
    pub name: &'static str,
    pub binary_name: &'static str,
    pub version: &'static str,
    pub source: CargoPackageSource,
}

#[derive(Clone, Copy, Debug)]
pub enum CargoPackageSource {
    Registry,
    GitTag {
        url: &'static str,
        tag: &'static str,
        toolchain: &'static str,
    },
}

impl CargoPackage {
    pub const fn new(name: &'static str, version: &'static str) -> Self {
        Self {
            name,
            binary_name: name,
            version,
            source: CargoPackageSource::Registry,
        }
    }

    pub const fn with_binary_name(self, name: &'static str) -> Self {
        Self {
            binary_name: name,
            ..self
        }
    }

    /// Builds the package from a git tag with the given Rust toolchain instead of the workspace one.
    pub const fn with_git_tag(self, url: &'static str, tag: &'static str, toolchain: &'static str) -> Self {
        Self {
            source: CargoPackageSource::GitTag { url, tag, toolchain },
            ..self
        }
    }
}

pub fn cargo_install(sh: &Shell, package: &CargoPackage) -> anyhow::Result<()> {
    let package_name = package.name;
    let package_version = package.version;

    if is_installed(sh, package) && is_pin_satisfied(sh, package) {
        trace!("{package_name} is already installed");
        return Ok(());
    }

    if let CargoPackageSource::GitTag { url, tag, toolchain } = package.source {
        trace!("Install {package_name} from {url} at tag {tag} with Rust {toolchain}");
        // cargo-binstall cannot install from a git tag.
        cmd!(
            sh,
            "rustup run --install {toolchain} cargo install --debug --locked --force --root {LOCAL_CARGO_ROOT} --git {url} --tag {tag} {package_name}"
        )
        .run()?;
        sh.write_file(marker_path(package, "source"), source_marker(url, tag))
            .with_context(|| format!("record installed {package_name} source"))?;
        sh.write_file(marker_path(package, "version"), package_version)
            .with_context(|| format!("record installed {package_name} version"))?;
    } else if cargo_binstall_is_available(sh) {
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

/// Checks that a git-pinned binary was installed from the pinned tag and version.
///
/// Every `-devo.N` tag of a Diplomat release carries the same crate version, so the recorded source is what tells them apart.
pub fn is_pin_satisfied(sh: &Shell, package: &CargoPackage) -> bool {
    let read_marker = |kind| sh.read_file(marker_path(package, kind)).ok();

    match package.source {
        CargoPackageSource::Registry => true,
        CargoPackageSource::GitTag { url, tag, .. } => {
            read_marker("source").as_deref().map(str::trim) == Some(&source_marker(url, tag))
                && read_marker("version").as_deref().map(str::trim) == Some(package.version)
        }
    }
}

fn marker_path(package: &CargoPackage, kind: &str) -> std::path::PathBuf {
    local_cargo_root().join(format!("{}.{kind}", package.binary_name))
}

fn source_marker(url: &str, tag: &str) -> String {
    format!("git+{url}?tag={tag}")
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
