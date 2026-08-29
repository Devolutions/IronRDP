use tinyjson::JsonValue;

use crate::prelude::*;

const ALLOWED_TEST_TARGETS: &[(&str, &str)] = &[
    ("ironrdp-testsuite-core", "integration_tests_core"),
    ("ironrdp-testsuite-extra", "integration_tests_extra"),
];

pub fn check(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TEST-TARGETS");
    let metadata = cmd!(sh, "{CARGO} metadata --format-version=1 --no-deps --locked")
        .read()
        .context("read Cargo metadata")?;

    validate(&metadata)?;

    println!("All good!");
    Ok(())
}

fn validate(metadata: &str) -> anyhow::Result<()> {
    let metadata: JsonValue = metadata.parse().context("parse Cargo metadata")?;
    let metadata = json_object(&metadata, "Cargo metadata")?;
    let packages = json_array(
        metadata
            .get("packages")
            .context("Cargo metadata is missing `packages`")?,
        "`packages`",
    )?;
    let mut unauthorized = Vec::new();

    for package in packages {
        let package = json_object(package, "Cargo metadata package")?;
        let package_name = json_string(
            package
                .get("name")
                .context("Cargo metadata package is missing `name`")?,
            "Cargo metadata package name",
        )?;
        let targets = json_array(
            package
                .get("targets")
                .context("Cargo metadata package is missing `targets`")?,
            "Cargo metadata package targets",
        )?;

        for target in targets {
            let target = json_object(target, "Cargo metadata target")?;
            let kinds = json_array(
                target.get("kind").context("Cargo metadata target is missing `kind`")?,
                "Cargo metadata target kind",
            )?;
            let mut is_test = false;
            for kind in kinds {
                if json_string(kind, "Cargo metadata target kind")? == "test" {
                    is_test = true;
                    break;
                }
            }

            if !is_test {
                continue;
            }

            let target_name = json_string(
                target.get("name").context("Cargo metadata target is missing `name`")?,
                "Cargo metadata target name",
            )?;
            if ALLOWED_TEST_TARGETS.contains(&(package_name, target_name)) {
                continue;
            }

            let source_path = json_string(
                target
                    .get("src_path")
                    .context("Cargo metadata target is missing `src_path`")?,
                "Cargo metadata target source path",
            )?;
            unauthorized.push((package_name, target_name, source_path));
        }
    }

    if !unauthorized.is_empty() {
        let targets = unauthorized
            .into_iter()
            .map(|(package, target, source)| format!("- package: `{package}`, target: `{target}`, source: `{source}`"))
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("unauthorized Cargo test target(s):\n{targets}");
    }

    Ok(())
}

fn json_object<'a>(
    value: &'a JsonValue,
    name: &str,
) -> anyhow::Result<&'a std::collections::HashMap<String, JsonValue>> {
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => anyhow::bail!("{name} must be an object"),
    }
}

fn json_array<'a>(value: &'a JsonValue, name: &str) -> anyhow::Result<&'a [JsonValue]> {
    match value {
        JsonValue::Array(array) => Ok(array),
        _ => anyhow::bail!("{name} must be an array"),
    }
}

fn json_string<'a>(value: &'a JsonValue, name: &str) -> anyhow::Result<&'a str> {
    match value {
        JsonValue::String(string) => Ok(string),
        _ => anyhow::bail!("{name} must be a string"),
    }
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn accepts_centralized_test_targets() {
        let metadata = r#"{
            "packages": [
                {
                    "name": "ironrdp-testsuite-core",
                    "targets": [
                        {
                            "kind": ["test"],
                            "name": "integration_tests_core",
                            "src_path": "crates/ironrdp-testsuite-core/tests/main.rs"
                        }
                    ]
                },
                {
                    "name": "ironrdp-testsuite-extra",
                    "targets": [
                        {
                            "kind": ["test"],
                            "name": "integration_tests_extra",
                            "src_path": "crates/ironrdp-testsuite-extra/tests/main.rs"
                        }
                    ]
                }
            ]
        }"#;

        validate(metadata).unwrap();
    }

    #[test]
    fn rejects_unauthorized_explicit_test_target() {
        let metadata = r#"{
            "packages": [
                {
                    "name": "ironrdp-example",
                    "targets": [
                        {
                            "kind": ["test"],
                            "name": "explicit_test",
                            "src_path": "crates/ironrdp-example/tests/explicit.rs"
                        }
                    ]
                }
            ]
        }"#;

        let error = validate(metadata).unwrap_err().to_string();
        assert!(error.contains("package: `ironrdp-example`"));
        assert!(error.contains("target: `explicit_test`"));
        assert!(error.contains("source: `crates/ironrdp-example/tests/explicit.rs`"));
    }

    #[test]
    fn rejects_unauthorized_auto_discovered_test_target() {
        let metadata = r#"{
            "packages": [
                {
                    "name": "ironrdp-example",
                    "targets": [
                        {
                            "kind": ["test"],
                            "name": "auto_discovered",
                            "src_path": "crates/ironrdp-example/tests/auto_discovered.rs"
                        }
                    ]
                }
            ]
        }"#;

        let error = validate(metadata).unwrap_err().to_string();
        assert!(error.contains("package: `ironrdp-example`"));
        assert!(error.contains("target: `auto_discovered`"));
        assert!(error.contains("source: `crates/ironrdp-example/tests/auto_discovered.rs`"));
    }
}
