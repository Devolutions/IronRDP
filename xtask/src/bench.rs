use crate::prelude::*;

pub fn corpus(sh: &Shell, arguments: &[&str]) -> anyhow::Result<()> {
    cmd!(
        sh,
        "{CARGO} run --locked --package ironrdp-bench --bin corpus-bench -- {arguments...}"
    )
    .run()?;
    Ok(())
}
