#![cfg(windows)]
#![expect(
    unused_crate_dependencies,
    reason = "the package's library dependencies are also linked into this integration test"
)]

use core::time::Duration;

use ironrdp_dvc::DvcProcessor as _;
use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
use tokio::net::windows::named_pipe::ClientOptions;

#[tokio::test]
async fn starts_a_connectable_windows_pipe() {
    let name = format!("ironrdp-dvc-pipe-proxy-test-{}", std::process::id());
    let mut proxy = DvcNamedPipeProxy::new("test", &name, |_, _| Ok(()));
    proxy.start(1).expect("start DVC pipe proxy");

    let pipe_path = format!(r"\\.\pipe\{name}");
    let client = (0..200)
        .find_map(|_| match ClientOptions::new().open(&pipe_path) {
            Ok(client) => Some(client),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("DVC pipe proxy must create the pipe within two seconds");

    drop(client);
}
