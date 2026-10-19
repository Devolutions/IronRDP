use crate::prelude::*;

pub fn corpus(sh: &Shell, arguments: &[&str]) -> anyhow::Result<()> {
    cmd!(
        sh,
        "{CARGO} run --locked --package ironrdp-bench --bin corpus-bench -- {arguments...}"
    )
    .run()?;
    Ok(())
}

/// Time capture replay of one cached capture with the `ironrdp-bench` Criterion target.
pub fn replay(sh: &Shell, capture: &str) -> anyhow::Result<()> {
    let benchmark = format!("replay/{capture}");

    // Criterion exits successfully without measuring anything when the filter matches no benchmark.
    let listed = cmd!(
        sh,
        "{CARGO} bench --locked --package ironrdp-bench --bench corpus -- {benchmark} --exact --list"
    )
    .read()?;
    anyhow::ensure!(
        listed.lines().any(|line| line == format!("{benchmark}: benchmark")),
        "capture {capture} is unknown or does not list the replay benchmark"
    );

    cmd!(
        sh,
        "{CARGO} bench --locked --package ironrdp-bench --bench corpus -- {benchmark} --exact"
    )
    .run()?;
    Ok(())
}
