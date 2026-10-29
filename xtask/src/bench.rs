use crate::NPM;
use crate::prelude::*;
use crate::target_directory;

const RFX_DECODE_WASM_PATH: &str = "./benches/rfx-decode-wasm";
const WASM_TARGET: &str = "wasm32-unknown-unknown";
const RFX_DECODE_WASM_ARTIFACT: &str = "rfx_decode_wasm.wasm";

pub fn corpus(sh: &Shell, command: &str) -> anyhow::Result<()> {
    cmd!(
        sh,
        "{CARGO} run --locked --package ironrdp-bench --bin corpus-bench -- {command}"
    )
    .run()?;
    Ok(())
}

/// Runs the native RemoteFX decode benchmark.
pub fn rfx_decode(sh: &Shell, args: Vec<String>) -> anyhow::Result<()> {
    let _s = Section::new("BENCH-RFX-DECODE");

    cmd!(sh, "{CARGO} bench -p benches --bench rfx-decode --")
        .args(args)
        .run()?;

    Ok(())
}

/// Builds the RemoteFX decode benchmark for WASM and runs it under Node.
pub fn rfx_decode_wasm(sh: &Shell, simd: bool) -> anyhow::Result<()> {
    let _s = Section::new("BENCH-RFX-DECODE-WASM");

    run_cmd_in!(sh, RFX_DECODE_WASM_PATH, "{NPM} ci --include=dev")?;

    let mut build = cmd!(
        sh,
        "{CARGO} build -p rfx-decode-wasm --release --locked --target {WASM_TARGET}"
    );
    if simd {
        build = build.args([
            "--config",
            r#"target.wasm32-unknown-unknown.rustflags=["-C", "target-feature=+simd128"]"#,
        ]);
    }
    build.run()?;

    let module = target_directory(sh)?
        .join(WASM_TARGET)
        .join("release")
        .join(RFX_DECODE_WASM_ARTIFACT);

    let _guard = sh.push_dir(RFX_DECODE_WASM_PATH);
    cmd!(sh, "node run.mjs {module}").run()?;

    Ok(())
}
