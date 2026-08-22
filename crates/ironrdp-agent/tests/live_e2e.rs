#![allow(unused_crate_dependencies)]
#![allow(clippy::panic)]
#![allow(clippy::std_instead_of_core)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn test_endpoint(name: &str) -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\ironrdp-agent-{name}-{}", std::process::id())
    }

    #[cfg(unix)]
    {
        let path = std::env::temp_dir().join(format!("ironrdp-agent-{name}-{}.sock", std::process::id()));
        path.display().to_string()
    }
}

fn spawn_daemon(endpoint: &str, skip_certificate_check: bool) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"));
    command.arg("--endpoint").arg(endpoint).arg("daemon-start");
    if skip_certificate_check {
        command.arg("--skip-certificate-check");
    }
    command
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

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_for_frame(endpoint: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);

    while Instant::now() < deadline {
        let output = agent(endpoint, &["status"]);
        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains("state: Connected") {
            return;
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    panic!("RDP session did not produce a frame");
}

fn send_unicode_text(endpoint: &str, text: &str) {
    for character in text.chars() {
        let character = character.to_string();
        assert_success(&agent(
            endpoint,
            &["key-unicode", "--char", &character, "--pressed", "true"],
        ));
        assert_success(&agent(
            endpoint,
            &["key-unicode", "--char", &character, "--pressed", "false"],
        ));
    }
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
    let password = env("IRONRDP_AGENT_E2E_PASSWORD").expect("IRONRDP_AGENT_E2E_PASSWORD");

    let endpoint = test_endpoint("live");
    let mut daemon = spawn_daemon(&endpoint, false);
    let screenshot = std::env::temp_dir().join(format!("ironrdp-agent-live-{}.png", std::process::id()));

    let result = std::panic::catch_unwind(|| {
        wait_for_daemon(&endpoint);

        let mut connect_args = vec![
            "connect",
            "--server",
            host.as_str(),
            "--username",
            username.as_str(),
            "--password",
            password.as_str(),
            "--prop",
            "desktopwidth:i:1280",
            "--prop",
            "desktopheight:i:720",
        ];
        if let Some(domain) = domain.as_deref() {
            connect_args.push("--domain");
            connect_args.push(domain);
        }

        let output = agent(&endpoint, &connect_args);
        assert_success(&output);
        wait_for_frame(&endpoint);

        assert_success(&agent(&endpoint, &["mouse-move", "--x", "200", "--y", "200"]));
        assert_success(&agent(
            &endpoint,
            &["mouse-button", "--button", "left", "--pressed", "true"],
        ));
        assert_success(&agent(
            &endpoint,
            &["mouse-button", "--button", "left", "--pressed", "false"],
        ));

        assert_success(&agent(
            &endpoint,
            &["key-scancode", "--scancode", "0xE05B", "--pressed", "true"],
        ));
        assert_success(&agent(
            &endpoint,
            &["key-scancode", "--scancode", "0x13", "--pressed", "true"],
        ));
        assert_success(&agent(
            &endpoint,
            &["key-scancode", "--scancode", "0x13", "--pressed", "false"],
        ));
        assert_success(&agent(
            &endpoint,
            &["key-scancode", "--scancode", "0xE05B", "--pressed", "false"],
        ));
        std::thread::sleep(Duration::from_secs(1));
        send_unicode_text(&endpoint, "msedge.exe https://example.com");
        assert_success(&agent(
            &endpoint,
            &["key-scancode", "--scancode", "0x1c", "--pressed", "true"],
        ));
        assert_success(&agent(
            &endpoint,
            &["key-scancode", "--scancode", "0x1c", "--pressed", "false"],
        ));
        std::thread::sleep(Duration::from_secs(5));

        assert_success(&agent(
            &endpoint,
            &["screenshot", screenshot.to_str().expect("screenshot path")],
        ));
        assert_png(&screenshot);
        assert_success(&agent(&endpoint, &["disconnect"]));
    });

    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = std::fs::remove_file(&screenshot);

    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
#[ignore = "requires an authorized RAIL endpoint and IRONRDP_AGENT_RAIL_E2E_* environment variables"]
fn remoteapp_records_validated_rail_evidence() {
    if env("IRONRDP_AGENT_RAIL_E2E").as_deref() != Some("1") {
        return;
    }

    let host = env("IRONRDP_AGENT_RAIL_E2E_HOST").expect("IRONRDP_AGENT_RAIL_E2E_HOST");
    let username = env("IRONRDP_AGENT_RAIL_E2E_USERNAME").expect("IRONRDP_AGENT_RAIL_E2E_USERNAME");
    let domain = env("IRONRDP_AGENT_RAIL_E2E_DOMAIN");
    let password = env("IRONRDP_AGENT_RAIL_E2E_PASSWORD").expect("IRONRDP_AGENT_RAIL_E2E_PASSWORD");

    let endpoint = test_endpoint("rail-live");
    let mut daemon = spawn_daemon(&endpoint, true);
    let result = std::panic::catch_unwind(|| {
        wait_for_daemon(&endpoint);

        let mut connect_args = vec![
            "connect",
            "--server",
            host.as_str(),
            "--username",
            username.as_str(),
            "--password",
            password.as_str(),
            "--prop",
            "remoteapplicationmode:i:1",
            "--prop",
            "remoteapplicationprogram:s:notepad.exe",
            "--prop",
            "ironrdp_autologon:i:1",
        ];
        if let Some(domain) = domain.as_deref() {
            connect_args.push("--domain");
            connect_args.push(domain);
        }
        assert_success(&agent(&endpoint, &connect_args));

        let deadline = Instant::now() + Duration::from_secs(60);
        let mut events = String::new();
        while Instant::now() < deadline {
            let status = agent(&endpoint, &["rail", "status"]);
            assert_success(&status);
            let output = agent(&endpoint, &["rail", "events"]);
            assert_success(&output);
            events = String::from_utf8_lossy(&output.stdout).into_owned();
            if String::from_utf8_lossy(&status.stdout).contains("handshake complete: true")
                && events.contains("execute result")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }

        assert!(
            events.contains("handshake"),
            "missing RAIL handshake evidence: {events}"
        );
        assert!(
            events.contains("execute result"),
            "missing RAIL Execute Result evidence: {events}"
        );
        assert_success(&agent(&endpoint, &["disconnect"]));
    });

    let _ = daemon.kill();
    let _ = daemon.wait();

    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

fn assert_png(path: &PathBuf) {
    let bytes = std::fs::read(path).expect("read screenshot");
    assert!(bytes.len() > 8);
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
}
