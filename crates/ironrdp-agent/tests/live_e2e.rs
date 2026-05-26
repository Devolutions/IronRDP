#![allow(unused_crate_dependencies)]
#![allow(clippy::panic)]
#![allow(clippy::std_instead_of_core)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn test_endpoint(name: &str) -> String {
    #[cfg(windows)]
    {
        format!("pipe:ironrdp-agent-{name}-{}", std::process::id())
    }

    #[cfg(unix)]
    {
        let path = std::env::temp_dir().join(format!("ironrdp-agent-{name}-{}.sock", std::process::id()));
        format!("unix:{}", path.display())
    }
}

fn spawn_daemon(endpoint: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
        .arg("--endpoint")
        .arg(endpoint)
        .arg("--no-spawn-daemon")
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon")
}

fn agent(endpoint: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
        .arg("--endpoint")
        .arg(endpoint)
        .arg("--no-spawn-daemon")
        .args(args)
        .output()
        .expect("run agent")
}

fn wait_for_daemon(endpoint: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);

    while Instant::now() < deadline {
        let output = agent(endpoint, &["status"]);
        if output.status.success() {
            return;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    panic!("daemon did not become ready");
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn session_id(stdout: &[u8]) -> String {
    let value: serde_json::Value = serde_json::from_slice(stdout).expect("connect response");
    value["session_id"].as_str().expect("session_id").to_owned()
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires a reachable RDP host and IRONRDP_AGENT_E2E_* environment variables"]
fn connect_launch_browser_and_capture_screenshot() {
    if env("IRONRDP_AGENT_E2E").as_deref() != Some("1") {
        return;
    }

    let host = env("IRONRDP_AGENT_E2E_HOST").expect("IRONRDP_AGENT_E2E_HOST");
    let username = env("IRONRDP_AGENT_E2E_USERNAME").expect("IRONRDP_AGENT_E2E_USERNAME");
    let domain = env("IRONRDP_AGENT_E2E_DOMAIN");
    env("IRONRDP_AGENT_E2E_PASSWORD").expect("IRONRDP_AGENT_E2E_PASSWORD");

    let endpoint = test_endpoint("live");
    let mut daemon = spawn_daemon(&endpoint);
    let screenshot = std::env::temp_dir().join(format!("ironrdp-agent-live-{}.png", std::process::id()));

    let result = std::panic::catch_unwind(|| {
        wait_for_daemon(&endpoint);

        let mut connect_args = vec![
            "connect",
            host.as_str(),
            "--username",
            username.as_str(),
            "--password-env",
            "IRONRDP_AGENT_E2E_PASSWORD",
            "--desktop-size",
            "1280x720",
        ];
        if let Some(domain) = domain.as_deref() {
            connect_args.push("--domain");
            connect_args.push(domain);
        }

        let output = agent(&endpoint, &connect_args);
        assert_success(&output);
        let id = session_id(&output.stdout);

        assert_success(&agent(
            &endpoint,
            &["wait-frame", "--session", &id, "--timeout-ms", "60000"],
        ));
        assert_success(&agent(
            &endpoint,
            &["mouse", "--session", &id, "move", "--x", "200", "--y", "200"],
        ));
        assert_success(&agent(
            &endpoint,
            &["mouse", "--session", &id, "click", "--button", "left"],
        ));

        assert_success(&agent(
            &endpoint,
            &["keyboard", "--session", &id, "shortcut", "--scancodes", "0xE05B,0x13"],
        ));
        std::thread::sleep(Duration::from_secs(1));
        assert_success(&agent(
            &endpoint,
            &[
                "keyboard",
                "--session",
                &id,
                "text",
                "--text",
                "msedge.exe https://example.com",
            ],
        ));
        assert_success(&agent(
            &endpoint,
            &["keyboard", "--session", &id, "key", "--scancode", "0x1c"],
        ));
        assert_success(&agent(
            &endpoint,
            &["keyboard", "--session", &id, "key", "--scancode", "0x1c", "--release"],
        ));
        std::thread::sleep(Duration::from_secs(5));

        assert_success(&agent(
            &endpoint,
            &[
                "screenshot",
                "--session",
                &id,
                "--output",
                screenshot.to_str().expect("screenshot path"),
            ],
        ));
        assert_png(&screenshot);
        assert_success(&agent(&endpoint, &["disconnect", "--session", &id]));
    });

    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = std::fs::remove_file(&screenshot);

    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

fn assert_png(path: &PathBuf) {
    let bytes = std::fs::read(path).expect("read screenshot");
    assert!(bytes.len() > 8);
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
}
