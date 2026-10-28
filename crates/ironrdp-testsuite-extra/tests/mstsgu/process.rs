use core::time::Duration;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};

use anyhow::{Context as _, ensure};
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;

static BUILD_LOCK: Mutex<()> = Mutex::new(());
static BINARY: LazyLock<PathBuf> = LazyLock::new(|| build("rustls smartcard", "smartcard"));
static WITHOUT_SMARTCARD: LazyLock<PathBuf> = LazyLock::new(|| build("rustls", "without-smartcard"));

fn build(features: &str, name: &str) -> PathBuf {
    let _lock = BUILD_LOCK.lock().expect("serialize Mstsgu feature builds");
    let binary = escargot::CargoBuild::new()
        .manifest_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ironrdp-mstsgu-tests/Cargo.toml"
        ))
        .bin("ironrdp-mstsgu-tests")
        .no_default_features()
        .features(features)
        .current_release()
        .current_target()
        .arg("--locked")
        .run()
        .expect("build isolated Mstsgu checks");
    // Cargo reuses the executable's name across feature builds, so each cached configuration keeps its own copy.
    let path = binary
        .path()
        .with_file_name(format!("ironrdp-mstsgu-tests-{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(binary.path(), &path).expect("preserve the Mstsgu feature build");
    path
}

pub(super) async fn run(scenario: &str, endpoint: Option<&str>, proxy: Option<&str>) -> anyhow::Result<()> {
    let without_smartcard = scenario == "smartcard-disabled";
    let path = tokio::task::spawn_blocking(move || {
        if without_smartcard {
            &*WITHOUT_SMARTCARD
        } else {
            &*BINARY
        }
    })
    .await
    .context("build Mstsgu binary")?;
    let mut command = Command::new(path);
    command.arg(scenario);
    if let Some(endpoint) = endpoint {
        command.arg(endpoint);
    }
    for variable in ["HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"] {
        command.env_remove(variable);
    }
    if let Some(proxy) = proxy {
        command.env("HTTPS_PROXY", proxy);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawn Mstsgu check")?;
    let mut stdout = child.stdout.take().context("Mstsgu stdout pipe")?;
    let mut stderr = child.stderr.take().context("Mstsgu stderr pipe")?;
    let stdout = tokio::spawn(async move {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).await.map(|_| output)
    });
    let stderr = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });
    let timeout = if scenario == "rpch-in-recycle" { 660 } else { 240 };
    let status = tokio::time::timeout(Duration::from_secs(timeout), child.wait()).await;
    if status.is_err() {
        child.kill().await.context("kill and reap timed-out Mstsgu check")?;
    }
    let stdout = stdout.await.context("read Mstsgu stdout")??;
    let stderr = stderr.await.context("read Mstsgu stderr")??;
    let diagnostics = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    ensure!(
        status.is_ok(),
        "Mstsgu {scenario} exceeded {timeout} seconds\n{diagnostics}"
    );
    let status = status.context("Mstsgu timeout")?.context("wait for Mstsgu check")?;
    ensure!(status.success(), "Mstsgu {scenario} failed ({status})\n{diagnostics}");
    Ok(())
}
