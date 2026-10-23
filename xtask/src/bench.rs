use crate::prelude::*;

pub fn corpus(sh: &Shell, arguments: &[&str]) -> anyhow::Result<()> {
    cmd!(
        sh,
        "{CARGO} run --locked --package ironrdp-bench --no-default-features --bin corpus-bench -- {arguments...}"
    )
    .run()?;
    Ok(())
}

/// Time one cached capture with a benchmark kind of the `ironrdp-bench` corpus Criterion target.
pub fn time(sh: &Shell, kind: &str, capture: &str) -> anyhow::Result<()> {
    let benchmark = format!("{kind}/{capture}");

    // Criterion exits successfully without measuring anything when the filter matches no benchmark.
    let listed = cmd!(
        sh,
        "{CARGO} bench --locked --package ironrdp-bench --no-default-features --bench corpus -- {benchmark} --exact --list"
    )
    .read()?;
    anyhow::ensure!(
        listed.lines().any(|line| line == format!("{benchmark}: benchmark")),
        "capture {capture} is unknown or does not list the {kind} benchmark"
    );

    cmd!(
        sh,
        "{CARGO} bench --locked --package ironrdp-bench --no-default-features --bench corpus -- {benchmark} --exact"
    )
    .run()?;
    Ok(())
}
