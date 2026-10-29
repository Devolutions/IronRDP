use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tinyjson::JsonValue;

use crate::prelude::*;

/// A crate version as `major.minor.patch`, without pre-release or build metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// Parses `major.minor.patch`, rejecting pre-release and build metadata, which release tags never carry.
    fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split('.');
        let version = Self {
            major: parts.next()?.parse().ok()?,
            minor: parts.next()?.parse().ok()?,
            patch: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// Whether a caret requirement on `self` rejects `other`, a version no older than `self`, which is when Cargo treats the change as breaking.
    ///
    /// A caret requirement on `self` accepts exactly the versions from `self` up to, excluding, `self.next_breaking()`, as described in [Caret requirements].
    ///
    /// [Caret requirements]: https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#caret-requirements
    fn is_incompatible_with(self, other: Self) -> bool {
        self.next_breaking() <= other
    }

    /// The smallest version that a caret requirement on `self` rejects, as described in [Caret requirements].
    ///
    /// [Caret requirements]: https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#caret-requirements
    fn next_breaking(self) -> Self {
        if self.major != 0 {
            Self {
                major: self.major + 1,
                minor: 0,
                patch: 0,
            }
        } else if self.minor != 0 {
            Self {
                major: 0,
                minor: self.minor + 1,
                patch: 0,
            }
        } else {
            Self {
                major: 0,
                minor: 0,
                patch: self.patch + 1,
            }
        }
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A workspace crate being released: its version differs from its last `<name>-v<version>` tag.
#[derive(Debug)]
struct Release {
    name: String,
    previous: Version,
    current: Version,
    /// Workspace dependencies marked `# public` in the crate's manifest, as described in [the public-dependency guidance].
    ///
    /// [the public-dependency guidance]: ../../.agents/skills/public-dependency/SKILL.md
    public_deps: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Bump {
    name: String,
    previous: Version,
    current: Version,
    required: Version,
    /// The public dependencies whose breaking release forces this one.
    because_of: Vec<String>,
}

/// Finds released crates whose public dependencies take a breaking release while they do not.
///
/// A crate that exposes a dependency's types breaks its users when that dependency breaks,
/// so it needs a breaking release too, and so do the crates exposing it in turn.
fn plan(releases: &[Release]) -> Vec<Bump> {
    let mut breaking: BTreeSet<&str> = releases
        .iter()
        .filter(|release| release.previous.is_incompatible_with(release.current))
        .map(|release| release.name.as_str())
        .collect();

    let mut bumps = Vec::new();

    loop {
        let mut changed = false;

        for release in releases {
            if breaking.contains(release.name.as_str()) {
                continue;
            }

            let because_of: Vec<String> = release
                .public_deps
                .iter()
                .filter(|dep| breaking.contains(dep.as_str()))
                .cloned()
                .collect();

            if because_of.is_empty() {
                continue;
            }

            breaking.insert(release.name.as_str());
            bumps.push(Bump {
                name: release.name.clone(),
                previous: release.previous,
                current: release.current,
                required: release.previous.next_breaking(),
                because_of,
            });
            changed = true;
        }

        if !changed {
            return bumps;
        }
    }
}

/// Returns the names of the dependencies marked `# public` in a manifest's non-dev dependency tables.
///
/// The marker and its meaning are described in [the public-dependency guidance].
///
/// [the public-dependency guidance]: ../../.agents/skills/public-dependency/SKILL.md
fn public_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut section = "";
    let mut deps = BTreeSet::new();

    for line in manifest.lines() {
        let line = line.trim();

        if let Some(header) = line.strip_prefix('[') {
            section = header.trim_end_matches(']');
            continue;
        }

        if !is_dependency_table(section) || section.ends_with("dev-dependencies") {
            continue;
        }

        let (value, comment) = split_comment(line);

        if !comment.is_some_and(|comment| comment.trim_start().starts_with("public")) {
            continue;
        }

        if let Some((name, _)) = value.split_once('=') {
            deps.insert(name.trim().to_owned());
        }
    }

    deps
}

/// Whether a table header such as `dependencies` or `target.'cfg(windows)'.dev-dependencies` names a dependency table.
fn is_dependency_table(section: &str) -> bool {
    matches!(
        section.rsplit('.').next(),
        Some("dependencies" | "build-dependencies" | "dev-dependencies")
    )
}

/// Splits a manifest line at its comment, ignoring `#` inside strings.
#[expect(
    clippy::string_slice,
    reason = "the index is that of an ASCII `#`, so it is a character boundary"
)]
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let mut in_string = false;

    for (idx, ch) in line.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return (&line[..idx], Some(&line[idx + 1..])),
            _ => {}
        }
    }

    (line, None)
}

/// Rewrites the `[package]` version of a manifest.
fn set_package_version(manifest: &str, from: Version, to: Version) -> Option<String> {
    let mut section = "";
    let mut done = false;
    let mut out = String::with_capacity(manifest.len());
    let expected = format!("version = \"{from}\"");
    let (from, to) = (from.to_string(), to.to_string());

    for line in manifest.split_inclusive('\n') {
        let trimmed = line.trim();

        if let Some(header) = trimmed.strip_prefix('[') {
            section = header.trim_end_matches(']');
        }

        if !done && section == "package" && trimmed == expected {
            out.push_str(&line.replacen(&from, &to, 1));
            done = true;
        } else {
            out.push_str(line);
        }
    }

    done.then_some(out)
}

/// Returns the index just past the opening quote of the `version` key's value in an inline dependency table.
///
/// Only a key outside strings counts, so a path or feature containing `version` is skipped.
#[expect(
    clippy::string_slice,
    reason = "every index comes from `char_indices` or follows an ASCII token, so it is a character boundary"
)]
fn inline_table_version_start(value: &str) -> Option<usize> {
    let mut in_string = false;
    let mut previous = None;

    for (idx, ch) in value.char_indices() {
        if ch == '"' {
            in_string = !in_string;
        }

        if !in_string && matches!(previous, Some('{' | ',')) {
            if let Some(rest) = value[idx..].strip_prefix("version") {
                let after_key = rest.trim_start();

                if let Some(after_equals) = after_key.strip_prefix('=') {
                    let after_equals = after_equals.trim_start();

                    if after_equals.starts_with('"') {
                        return Some(value.len() - after_equals.len() + 1);
                    }
                }
            }
        }

        if !ch.is_whitespace() {
            previous = Some(ch);
        }
    }

    None
}

/// Rewrites every requirement on `name` in a manifest's dependency tables so that it accepts `to`.
///
/// The requirement keeps its operator and its number of components, as in `"0.9"` becoming `"0.10"`.
#[expect(
    clippy::string_slice,
    reason = "every index is that of an ASCII quote or operator found in the line, so it is a character boundary"
)]
fn set_requirement(manifest: &str, name: &str, to: Version) -> String {
    let mut section = "";
    let mut out = String::with_capacity(manifest.len());

    for line in manifest.split_inclusive('\n') {
        let trimmed = line.trim();

        if let Some(header) = trimmed.strip_prefix('[') {
            section = header.trim_end_matches(']');
        }

        let is_dependency_line = is_dependency_table(section)
            && trimmed
                .strip_prefix(name)
                .is_some_and(|rest| rest.trim_start().starts_with('='));

        if !is_dependency_line {
            out.push_str(line);
            continue;
        }

        let (value, _) = split_comment(line);
        let requirement_start = if value.contains('{') {
            inline_table_version_start(value)
        } else {
            value.find('"').map(|quote| quote + 1)
        };

        let Some(start) = requirement_start else {
            out.push_str(line);
            continue;
        };
        let Some(len) = line[start..].find('"') else {
            out.push_str(line);
            continue;
        };

        let requirement = &line[start..start + len];
        let operator_len = requirement.len() - requirement.trim_start_matches(['^', '~', '=']).len();
        let components = requirement[operator_len..].split('.').count();
        let new_requirement = [to.major, to.minor, to.patch]
            .iter()
            .take(components)
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".");

        out.push_str(&line[..start + operator_len]);
        out.push_str(&new_requirement);
        out.push_str(&line[start + len..]);
    }

    out
}

/// Rewrites the release header that release-plz generates from `cliff.toml`, including its compare link.
fn set_changelog_version(changelog: &str, name: &str, bump: &Bump) -> Option<String> {
    let (previous, current, required) = (bump.previous, bump.current, bump.required);
    let old = format!(
        "## [[{current}](https://github.com/Devolutions/IronRDP/compare/{name}-v{previous}...{name}-v{current})]"
    );
    let new = format!(
        "## [[{required}](https://github.com/Devolutions/IronRDP/compare/{name}-v{previous}...{name}-v{required})]"
    );

    changelog.contains(&old).then(|| changelog.replacen(&old, &new, 1))
}

struct Package {
    name: String,
    version: Version,
    manifest_path: PathBuf,
    publish: bool,
}

pub fn check_public_deps(sh: &Shell, fix: bool) -> anyhow::Result<()> {
    let _s = Section::new("RELEASE-PUBLIC-DEPS");

    let packages = workspace_packages(sh)?;
    let workspace_names: BTreeSet<&str> = packages.iter().map(|package| package.name.as_str()).collect();
    let tags = released_versions(sh, &workspace_names)?;
    let mut releases = Vec::new();

    for package in packages.iter().filter(|package| package.publish) {
        let Some(&previous) = tags.get(&package.name) else {
            continue;
        };

        if previous == package.version {
            continue;
        }

        let manifest = sh.read_file(&package.manifest_path)?;
        let public_deps = public_dependencies(&manifest)
            .into_iter()
            .filter(|dep| workspace_names.contains(dep.as_str()))
            .collect();

        releases.push(Release {
            name: package.name.clone(),
            previous,
            current: package.version,
            public_deps,
        });
    }

    let bumps = plan(&releases);

    if bumps.is_empty() {
        println!("Every released crate whose public dependencies break takes a breaking release too (good)");
        return Ok(());
    }

    for bump in &bumps {
        println!(
            "`{}` {} -> {} must be at least {}: its public dependencies {} take a breaking release",
            bump.name,
            bump.previous,
            bump.current,
            bump.required,
            bump.because_of.join(", ")
        );
    }

    if !fix {
        anyhow::bail!(
            "{} crate(s) need a breaking release, run with `--fix` to apply them",
            bumps.len()
        );
    }

    apply(sh, &packages, &bumps)?;

    println!("Applied {} version bump(s)", bumps.len());

    Ok(())
}

fn apply(sh: &Shell, packages: &[Package], bumps: &[Bump]) -> anyhow::Result<()> {
    for bump in bumps {
        let package = packages
            .iter()
            .find(|package| package.name == bump.name)
            .context("bumped crate is not a workspace package")?;

        let manifest = sh.read_file(&package.manifest_path)?;
        let manifest = set_package_version(&manifest, bump.current, bump.required)
            .with_context(|| format!("find the package version of `{}`", bump.name))?;
        sh.write_file(&package.manifest_path, manifest)?;

        let changelog_path = package
            .manifest_path
            .parent()
            .context("manifest has no parent directory")?
            .join("CHANGELOG.md");

        if sh.path_exists(&changelog_path) {
            let changelog = sh.read_file(&changelog_path)?;

            let changelog = set_changelog_version(&changelog, &bump.name, bump)
                .with_context(|| format!("find the generated release header in {}", changelog_path.display()))?;
            sh.write_file(&changelog_path, changelog)?;
        }
    }

    let manifests = packages
        .iter()
        .map(|package| package.manifest_path.as_path())
        .chain(core::iter::once(Path::new("fuzz/Cargo.toml")));

    for path in manifests {
        let manifest = bumps.iter().fold(sh.read_file(path)?, |manifest, bump| {
            set_requirement(&manifest, &bump.name, bump.required)
        });
        sh.write_file(path, manifest)?;
    }

    cmd!(sh, "{CARGO} update --workspace").run()?;
    cmd!(sh, "{CARGO} update --workspace --manifest-path fuzz/Cargo.toml").run()?;

    Ok(())
}

fn workspace_packages(sh: &Shell) -> anyhow::Result<Vec<Package>> {
    let metadata = crate::cargo_metadata(sh)?;
    let packages: &Vec<JsonValue> = metadata["packages"].get().context("cargo metadata has no packages")?;

    packages
        .iter()
        .map(|package| {
            let name: &String = package["name"].get().context("package has no name")?;
            let version: &String = package["version"].get().context("package has no version")?;
            let manifest_path: &String = package["manifest_path"].get().context("package has no manifest_path")?;
            let manifest_path = Path::new(manifest_path);
            // `publish = false` is reported as an empty registry list.
            let publish = match &package["publish"] {
                JsonValue::Array(registries) => !registries.is_empty(),
                _ => true,
            };

            Ok(Package {
                name: name.clone(),
                version: Version::parse(version)
                    .with_context(|| format!("unsupported version of `{name}`: {version}"))?,
                manifest_path: manifest_path
                    .strip_prefix(sh.current_dir())
                    .unwrap_or(manifest_path)
                    .to_path_buf(),
                publish,
            })
        })
        .collect()
}

fn released_versions(sh: &Shell, workspace_names: &BTreeSet<&str>) -> anyhow::Result<BTreeMap<String, Version>> {
    let tags = cmd!(sh, "git tag --list *-v*").read()?;
    Ok(latest_versions(&tags, workspace_names))
}

/// Returns the latest released version of each workspace crate, from its `<name>-v<version>` tags, one per line.
fn latest_versions(tags: &str, workspace_names: &BTreeSet<&str>) -> BTreeMap<String, Version> {
    let mut versions = BTreeMap::new();

    for tag in tags.lines() {
        let Some((name, version)) = tag.rsplit_once("-v") else {
            continue;
        };

        if !workspace_names.contains(name) {
            continue;
        }

        let Some(version) = Version::parse(version) else {
            println!("Ignoring tag `{tag}`: its version is not `major.minor.patch`");
            continue;
        };

        versions
            .entry(name.to_owned())
            .and_modify(|latest: &mut Version| *latest = (*latest).max(version))
            .or_insert(version);
    }

    versions
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn v(value: &str) -> Version {
        Version::parse(value).expect("valid version")
    }

    fn release(name: &str, previous: &str, current: &str, public_deps: &[&str]) -> Release {
        Release {
            name: name.to_owned(),
            previous: v(previous),
            current: v(current),
            public_deps: public_deps.iter().map(|dep| (*dep).to_owned()).collect(),
        }
    }

    // oracle: spec Cargo Book, Specifying Dependencies §Caret requirements
    #[rstest]
    #[case::minor_after_1_0("1.2.3", "1.9.0", false)]
    #[case::major_after_1_0("1.2.3", "2.0.0", true)]
    #[case::patch_before_1_0("0.2.3", "0.2.9", false)]
    #[case::minor_before_1_0("0.2.3", "0.3.0", true)]
    #[case::patch_before_0_1("0.0.3", "0.0.4", true)]
    fn caret_compatibility(#[case] from: &str, #[case] to: &str, #[case] incompatible: bool) {
        assert_eq!(v(from).is_incompatible_with(v(to)), incompatible);
    }

    // oracle: spec Cargo Book, Specifying Dependencies §Caret requirements
    #[rstest]
    #[case::after_1_0("1.2.3", "2.0.0")]
    #[case::before_1_0("0.10.1", "0.11.0")]
    #[case::before_0_1("0.0.2", "0.0.3")]
    fn next_breaking_version(#[case] from: &str, #[case] expected: &str) {
        assert_eq!(v(from).next_breaking(), v(expected));
    }

    // oracle: mutant accepting a pre-release, build metadata, or a version with too few or too many components
    #[rstest]
    #[case::pre_release("0.10.0-rc.1")]
    #[case::build_metadata("0.10.0+build")]
    #[case::two_components("0.10")]
    #[case::four_components("0.10.0.1")]
    fn versions_other_than_major_minor_patch_are_rejected(#[case] value: &str) {
        assert_eq!(Version::parse(value), None);
    }

    // oracle: regression Devolutions/IronRDP#2240
    #[test]
    fn patch_release_exposing_a_broken_dependency_needs_a_breaking_release() {
        let releases = [
            release("ironrdp-pdu", "0.10.0", "0.11.0", &[]),
            release("ironrdp-svc", "0.9.0", "0.9.1", &["ironrdp-pdu"]),
        ];

        assert_eq!(
            plan(&releases),
            [Bump {
                name: "ironrdp-svc".to_owned(),
                previous: v("0.9.0"),
                current: v("0.9.1"),
                required: v("0.10.0"),
                because_of: vec!["ironrdp-pdu".to_owned()],
            }]
        );
    }

    // oracle: regression Devolutions/IronRDP#2240
    #[test]
    fn breaking_releases_propagate_through_chains_of_public_dependencies() {
        let releases = [
            release("ironrdp-echo", "0.5.0", "0.5.1", &["ironrdp-dvc"]),
            release("ironrdp-dvc", "0.9.0", "0.9.1", &["ironrdp-pdu"]),
            release("ironrdp-pdu", "0.10.0", "0.11.0", &[]),
        ];

        let bumped: Vec<String> = plan(&releases).into_iter().map(|bump| bump.name).collect();

        assert_eq!(bumped, ["ironrdp-dvc", "ironrdp-echo"]);
    }

    // oracle: spec .agents/skills/public-dependency/SKILL.md §Public dependency
    #[test]
    fn private_dependencies_do_not_force_a_breaking_release() {
        let releases = [
            release("ironrdp-pdu", "0.10.0", "0.11.0", &[]),
            release("ironrdp-tokio", "0.11.0", "0.11.1", &[]),
        ];

        assert_eq!(plan(&releases), []);
    }

    // oracle: spec .agents/skills/public-dependency/SKILL.md §Public dependency
    #[test]
    fn public_dependencies_come_from_marked_non_dev_entries() {
        let manifest = r#"
[package]
name = "example"

[dependencies]
ironrdp-pdu = { path = "../ironrdp-pdu", version = "0.10" } # public (PduResult type)
ironrdp-core = { path = "../ironrdp-core", version = "0.3" }
tracing = "0.1" # public

[target.'cfg(windows)'.dependencies]
ironrdp-dvc = { version = "0.9", path = "../ironrdp-dvc", features = ["x#y"] } # public

[dev-dependencies]
ironrdp-svc = { path = "../ironrdp-svc", version = "0.9" } # public
"#;

        assert_eq!(
            public_dependencies(manifest),
            ["ironrdp-dvc", "ironrdp-pdu", "tracing"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
    }

    // oracle: mutant dropping the trailing comment or the requirement's precision
    #[test]
    fn requirement_keeps_its_precision_and_comment() {
        let manifest = "[dependencies]\nironrdp-pdu = { path = \"../ironrdp-pdu\", version = \"0.10\" } # public\n";

        assert_eq!(
            set_requirement(manifest, "ironrdp-pdu", v("0.11.0")),
            "[dependencies]\nironrdp-pdu = { path = \"../ironrdp-pdu\", version = \"0.11\" } # public\n"
        );
    }

    // oracle: regression review of Devolutions/IronRDP#2241, a `version` substring inside an earlier quoted value
    #[test]
    fn requirement_after_a_value_containing_version_is_found() {
        let manifest = "[dependencies]\nironrdp-pdu = { path = \"../a-version-b\", version = \"0.10\" }\n";

        assert_eq!(
            set_requirement(manifest, "ironrdp-pdu", v("0.11.0")),
            "[dependencies]\nironrdp-pdu = { path = \"../a-version-b\", version = \"0.11\" }\n"
        );
    }

    // oracle: mutant comparing tag versions as strings, which ranks 0.9.0 above 0.10.0
    #[test]
    fn latest_tag_is_chosen_by_version() {
        let names = BTreeSet::from(["ironrdp-pdu"]);
        let tags = "ironrdp-pdu-v0.9.0\nironrdp-pdu-v0.10.0\nironrdp-pdu-v0.2.0\n";

        assert_eq!(
            latest_versions(tags, &names),
            BTreeMap::from([("ironrdp-pdu".to_owned(), v("0.10.0"))])
        );
    }

    // oracle: mutant splitting at the first `-v`, which would cut a crate named like `ironrdp-vmconnect`
    #[test]
    fn tag_name_is_split_at_the_last_version_marker() {
        let names = BTreeSet::from(["ironrdp-vmconnect"]);

        assert_eq!(
            latest_versions("ironrdp-vmconnect-v0.1.0\n", &names),
            BTreeMap::from([("ironrdp-vmconnect".to_owned(), v("0.1.0"))])
        );
    }

    // oracle: mutant keeping tags of crates outside the workspace or with versions that do not parse
    #[test]
    fn foreign_and_unparsable_tags_are_ignored() {
        let names = BTreeSet::from(["ironrdp-pdu"]);
        let tags = "other-crate-v1.0.0\nironrdp-pdu-v0.10.0-rc.1\n";

        assert_eq!(latest_versions(tags, &names), BTreeMap::new());
    }

    // oracle: mutant matching a crate whose name only starts with the bumped one
    #[test]
    fn requirement_on_a_crate_sharing_the_name_prefix_is_kept() {
        let manifest = "[dependencies]\nironrdp-pdu-extra = { version = \"0.1\" }\n";

        assert_eq!(set_requirement(manifest, "ironrdp-pdu", v("0.11.0")), manifest);
    }

    // oracle: mutant skipping a dependency table, whose requirement would then no longer resolve to the workspace crate
    #[rstest]
    #[case::dev_dependencies("dev-dependencies")]
    #[case::build_dependencies("build-dependencies")]
    #[case::target_dependencies("target.'cfg(windows)'.dependencies")]
    fn requirement_in_every_dependency_table_is_updated(#[case] table: &str) {
        let manifest = format!("[{table}]\nironrdp-pdu = \"0.10.1\"\n");

        assert_eq!(
            set_requirement(&manifest, "ironrdp-pdu", v("0.11.0")),
            format!("[{table}]\nironrdp-pdu = \"0.11.0\"\n")
        );
    }

    // oracle: spec Cargo Book, Specifying Dependencies, whose caret, tilde and exact operators prefix the version
    #[rstest]
    #[case::caret("^0.10", "^0.11")]
    #[case::tilde("~0.10.1", "~0.11.0")]
    #[case::exact("=0.10.1", "=0.11.0")]
    fn requirement_keeps_its_operator(#[case] from: &str, #[case] expected: &str) {
        let manifest = format!("[dependencies]\nironrdp-pdu = {{ version = \"{from}\" }}\n");

        assert_eq!(
            set_requirement(&manifest, "ironrdp-pdu", v("0.11.0")),
            format!("[dependencies]\nironrdp-pdu = {{ version = \"{expected}\" }}\n")
        );
    }

    // oracle: mutant whose `Display` and `parse` disagree on the component order or separator
    #[test]
    fn version_display_round_trips_through_parse() {
        let components = [0, 1, 9, 10, u64::MAX];

        for major in components {
            for minor in components {
                for patch in components {
                    let version = Version { major, minor, patch };
                    assert_eq!(Version::parse(&version.to_string()), Some(version));
                }
            }
        }
    }

    fn svc_bump() -> Bump {
        Bump {
            name: "ironrdp-svc".to_owned(),
            previous: v("0.9.0"),
            current: v("0.9.1"),
            required: v("0.10.0"),
            because_of: vec!["ironrdp-pdu".to_owned()],
        }
    }

    // oracle: reference crates/ironrdp-svc/CHANGELOG.md, whose release headers release-plz renders from cliff.toml
    #[test]
    fn changelog_header_and_compare_link_take_the_required_version() {
        let changelog = "## [[0.9.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.9.0...ironrdp-svc-v0.9.1)] - 2026-10-26\n";

        assert_eq!(
            set_changelog_version(changelog, "ironrdp-svc", &svc_bump()).as_deref(),
            Some(
                "## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-svc-v0.9.0...ironrdp-svc-v0.10.0)] - 2026-10-26\n"
            )
        );
    }

    // oracle: mutant reporting success when the generated header is missing, which would leave a stale version in the changelog
    #[test]
    fn changelog_without_the_generated_header_is_reported() {
        assert_eq!(
            set_changelog_version("## [Unreleased]\n", "ironrdp-svc", &svc_bump()),
            None
        );
    }

    // oracle: mutant rewriting a dependency requirement that happens to equal the package version
    #[test]
    fn only_the_package_version_changes() {
        let manifest = "[package]\nname = \"x\"\nversion = \"0.9.1\"\n\n[dependencies]\ny = { version = \"0.9.1\" }\n";

        assert_eq!(
            set_package_version(manifest, v("0.9.1"), v("0.10.0")).as_deref(),
            Some("[package]\nname = \"x\"\nversion = \"0.10.0\"\n\n[dependencies]\ny = { version = \"0.9.1\" }\n")
        );
    }
}
