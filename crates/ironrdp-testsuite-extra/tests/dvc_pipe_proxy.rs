#[cfg(windows)]
use core::time::Duration;
#[cfg(windows)]
use std::sync::mpsc;
#[cfg(windows)]
use std::{env, process::Command};

#[cfg(windows)]
use ironrdp_dvc::DvcProcessor as _;
#[cfg(windows)]
use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
#[cfg(windows)]
use tokio::io::AsyncWriteExt as _;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

#[cfg(windows)]
const CHILD_PROCESS_TEST_NAME: &str = "dvc_pipe_proxy::child_process_helper_opens_and_forwards_windows_pipe_data";

#[cfg(windows)]
#[tokio::test]
async fn connects_and_forwards_windows_pipe_data() {
    let name = format!("ironrdp-dvc-pipe-proxy-test-{}", std::process::id());
    let (proxy, callback_rx) = start_proxy(&name);

    let mut client = open_client_pipe(&name, "DVC pipe proxy must create the pipe within two seconds");

    client.write_all(b"test data").await.expect("write to DVC pipe");
    assert_forwarded_data(callback_rx);

    drop(proxy);
}

#[cfg(windows)]
#[tokio::test]
async fn connects_from_a_separate_process() {
    let name = format!("ironrdp-dvc-pipe-proxy-cross-process-test-{}", std::process::id());
    let (proxy, callback_rx) = start_proxy(&name);
    let current_exe = env::current_exe().expect("current test executable");
    let output = Command::new(current_exe)
        .arg("--exact")
        .arg(CHILD_PROCESS_TEST_NAME)
        .arg("--nocapture")
        .env("IRONRDP_DVC_PIPE_PROXY_TEST_NAME", &name)
        .output()
        .expect("spawn DVC pipe client process");

    assert!(
        output.status.success(),
        "DVC pipe client process must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed;"),
        "DVC pipe client process must run its helper test:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_forwarded_data(callback_rx);

    drop(proxy);
}

#[cfg(windows)]
#[tokio::test]
async fn child_process_helper_opens_and_forwards_windows_pipe_data() {
    let Ok(name) = env::var("IRONRDP_DVC_PIPE_PROXY_TEST_NAME") else {
        return;
    };

    let mut client = open_client_pipe(
        &name,
        "DVC pipe proxy must be visible to a separate process within two seconds",
    );

    client
        .write_all(b"cross-process test data")
        .await
        .expect("write to DVC pipe");
}

#[cfg(windows)]
fn open_client_pipe(name: &str, unavailable_message: &str) -> NamedPipeClient {
    let pipe_path = format!(r"\\.\pipe\{name}");

    (0..200)
        .find_map(|_| match ClientOptions::new().open(&pipe_path) {
            Ok(client) => Some(client),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect(unavailable_message)
}

#[cfg(windows)]
fn start_proxy(name: &str) -> (DvcNamedPipeProxy, mpsc::Receiver<Vec<ironrdp_svc::SvcMessage>>) {
    let (callback_tx, callback_rx) = mpsc::channel();
    let mut proxy = DvcNamedPipeProxy::new("test", name, move |_, messages| {
        callback_tx
            .send(messages)
            .expect("test callback receiver must remain alive");
        Ok(())
    });
    proxy.start(1).expect("start DVC pipe proxy");

    (proxy, callback_rx)
}

#[cfg(windows)]
fn assert_forwarded_data(callback_rx: mpsc::Receiver<Vec<ironrdp_svc::SvcMessage>>) {
    let messages = callback_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("DVC pipe proxy must forward pipe data to its callback");
    assert!(!messages.is_empty(), "DVC pipe data must produce an SVC message");
}
