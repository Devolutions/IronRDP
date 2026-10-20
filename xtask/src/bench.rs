use crate::prelude::*;

pub fn corpus(sh: &Shell, command: &str) -> anyhow::Result<()> {
    cmd!(
        sh,
        "{CARGO} run --locked --package ironrdp-bench --bin corpus-bench -- {command}"
    )
    .run()?;
    Ok(())
}
