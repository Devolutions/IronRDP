#[cfg(windows)]
use core::time::Duration;
#[cfg(windows)]
use std::sync::mpsc;
#[cfg(windows)]
use std::{env, process::Command};

#[cfg(windows)]
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
#[cfg(windows)]
use ironrdp_dvc::{DvcMessageBatch, DvcProcessor as _};
#[cfg(windows)]
use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
#[cfg(windows)]
use tokio::io::AsyncWriteExt as _;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

#[cfg(windows)]
const CHILD_PROCESS_TEST_NAME: &str = "dvc_pipe_proxy::child_process_helper_opens_and_forwards_windows_pipe_data";
#[cfg(windows)]
const CROSS_PROCESS_DATA: &[u8] = b"cross-process test data";

#[cfg(windows)]
#[tokio::test]
async fn connects_and_forwards_windows_pipe_data() {
    let name = format!("ironrdp-dvc-pipe-proxy-test-{}", std::process::id());
    let (proxy, callback_rx) = start_proxy(&name, 1);

    let mut client = open_client_pipe(&name, "DVC pipe proxy must create the pipe within two seconds").await;

    client.write_all(b"test data").await.expect("write to DVC pipe");
    assert_forwarded_data(&callback_rx, b"test data");

    drop(proxy);
}

#[cfg(windows)]
#[tokio::test]
async fn reopens_windows_pipe_on_the_new_channel() {
    let name = format!("ironrdp-dvc-pipe-proxy-reopen-test-{}", std::process::id());
    let (mut proxy, callback_rx) = start_proxy(&name, 1);
    proxy.close(1);
    proxy.start(2).unwrap();
    let mut client = open_client_pipe(&name, "the reopened pipe client must connect").await;
    client.write_all(b"reopened channel").await.unwrap();
    let (id, messages) = callback_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2296#discussion_r4408520692
    assert_eq!(id, 2);
    DvcMessageBatch::try_new(id, messages).expect("reopened callback must encode the new channel ID");
}

#[cfg(windows)]
#[tokio::test]
async fn reconnects_after_the_local_pipe_client_disconnects() {
    let name = format!("ironrdp-dvc-pipe-proxy-reconnect-test-{}", std::process::id());
    let (proxy, callback_rx) = start_proxy(&name, 1);
    let mut client = open_client_pipe(&name, "the initial pipe client must connect").await;
    client
        .write_all(b"before disconnect")
        .await
        .expect("write before disconnect");
    assert_forwarded_data(&callback_rx, b"before disconnect");
    drop(client);

    let mut client = open_client_pipe(&name, "the local pipe must accept a replacement client").await;
    client
        .write_all(b"after reconnect")
        .await
        .expect("write after reconnect");
    // oracle: mutant blocking retries prevent local pipe reconnection and further DVC forwarding.
    assert_forwarded_data(&callback_rx, b"after reconnect");

    drop(proxy);
}

#[cfg(windows)]
#[tokio::test]
async fn dropping_the_proxy_cancels_a_pending_write_callback() {
    let name = format!("ironrdp-dvc-pipe-proxy-cancel-test-{}", std::process::id());
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let (lifetime_tx, mut lifetime_rx) = tokio::sync::mpsc::channel::<()>(1);
    let mut proxy = DvcNamedPipeProxy::new_async("test", &name, move |_, _| {
        let started = started_tx.clone();
        let lifetime = lifetime_tx.clone();
        async move {
            let _lifetime = lifetime;
            started.send(()).await.unwrap();
            core::future::pending().await
        }
    });
    proxy.start(7).unwrap();
    let mut client = open_client_pipe(&name, "the pipe client must connect").await;
    client.write_all(b"pending callback").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), started_rx.recv())
        .await
        .unwrap()
        .unwrap();
    drop(proxy);

    // oracle: mutant a callback waiting for transport capacity prevents proxy teardown.
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), lifetime_rx.recv())
            .await
            .unwrap(),
        None
    );
}

#[cfg(windows)]
#[tokio::test]
async fn forwards_pipe_data_on_the_started_channel() {
    const CHANNEL_ID: u32 = 7;

    let name = format!("ironrdp-dvc-pipe-proxy-channel-id-test-{}", std::process::id());
    let (proxy, callback_rx) = start_proxy(&name, CHANNEL_ID);

    let mut client = open_client_pipe(&name, "DVC pipe proxy must create the pipe within two seconds").await;

    client.write_all(b"test data").await.expect("write to DVC pipe");
    let (channel_id, messages) = callback_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("DVC pipe proxy must forward pipe data to its callback");

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2209
    assert_eq!(
        channel_id, CHANNEL_ID,
        "callback must receive the channel ID the proxy was started on"
    );
    // oracle: spec MS-RDPEDYC §2.2.3.2
    DvcMessageBatch::try_new(channel_id, messages).expect("callback channel ID must match the encoded DVC data");

    drop(proxy);
}

#[cfg(windows)]
#[tokio::test]
async fn connects_from_a_separate_process() {
    let name = format!("ironrdp-dvc-pipe-proxy-cross-process-test-{}", std::process::id());
    let (proxy, callback_rx) = start_proxy(&name, 1);
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
    assert_forwarded_data(&callback_rx, CROSS_PROCESS_DATA);

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
    )
    .await;

    client.write_all(CROSS_PROCESS_DATA).await.expect("write to DVC pipe");
}

#[cfg(windows)]
async fn open_client_pipe(name: &str, unavailable_message: &str) -> NamedPipeClient {
    let pipe_path = format!(r"\\.\pipe\{name}");

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ClientOptions::new().open(&pipe_path) {
                Ok(client) => return Ok(client),
                Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .expect(unavailable_message)
    .expect("open DVC pipe client")
}

#[cfg(windows)]
type ForwardedMessages = (u32, Vec<ironrdp_svc::SvcMessage>);

#[cfg(windows)]
fn start_proxy(name: &str, channel_id: u32) -> (DvcNamedPipeProxy, mpsc::Receiver<ForwardedMessages>) {
    let (callback_tx, callback_rx) = mpsc::channel();
    let mut proxy = DvcNamedPipeProxy::new("test", name, move |channel_id, messages| {
        callback_tx
            .send((channel_id, messages))
            .expect("test callback receiver must remain alive");
        Ok(())
    });
    proxy.start(channel_id).expect("start DVC pipe proxy");

    (proxy, callback_rx)
}

#[cfg(windows)]
fn assert_forwarded_data(callback_rx: &mpsc::Receiver<ForwardedMessages>, written: &[u8]) {
    let mut forwarded = Vec::new();

    // The proxy forwards each pipe read on its own, so one write may reach the callback in several parts.
    while forwarded.len() < written.len() {
        let (_, messages) = callback_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("DVC pipe proxy must forward pipe data to its callback");
        forwarded.extend(messages.iter().flat_map(dvc_data_payload));
    }

    // oracle: spec MS-RDPEDYC §2.2.3.1, §2.2.3.2
    assert_eq!(
        forwarded, written,
        "DVC data payloads must carry the bytes written to the pipe"
    );
}

#[cfg(windows)]
fn dvc_data_payload(message: &ironrdp_svc::SvcMessage) -> Vec<u8> {
    let encoded = message.encode_unframed_pdu().expect("encode forwarded SVC message");
    let payload = match ironrdp_core::decode(&encoded).expect("decode forwarded DVC PDU") {
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirst(pdu)) => Some(pdu.data().to_vec()),
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(pdu)) => Some(pdu.data().to_vec()),
        _ => None,
    };

    payload.expect("DVC pipe proxy must forward uncompressed DVC data")
}

#[cfg(unix)]
#[tokio::test]
async fn reopening_unix_proxy_keeps_replacement_socket_reachable() {
    use core::time::Duration;
    use std::sync::mpsc;

    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::UnixStream;

    let path = std::env::temp_dir().join(format!("ironrdp-dvc-reopen-{}.sock", std::process::id()));
    let path = path.to_str().expect("UTF-8 socket path");
    let (callback_tx, callback_rx) = mpsc::channel();
    let mut proxy = DvcNamedPipeProxy::new("test", path, move |id, _| {
        callback_tx.send(id).unwrap();
        Ok(())
    });

    for id in 1..=32 {
        proxy.start(id).unwrap();
        proxy.close(id);
    }
    proxy.start(33).unwrap();
    let mut client = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match UnixStream::connect(path).await {
                Ok(client) => break client,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("connect to replacement socket: {error}"),
            }
        }
    })
    .await
    .expect("replacement socket must remain reachable");
    client.write_all(b"replacement").await.unwrap();

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2296#discussion_r4408520692
    assert_eq!(callback_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 33);
    drop(proxy);
    std::fs::remove_file(path).expect("remove test socket");
}

#[cfg(unix)]
#[tokio::test]
async fn close_releases_the_pipe_while_a_write_is_stalled() {
    use core::time::Duration;

    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::UnixStream;

    let path = std::env::temp_dir().join(format!("ironrdp-dvc-pipe-proxy-{}.sock", std::process::id()));
    let path = path.to_str().expect("UTF-8 socket path");
    let mut proxy = DvcNamedPipeProxy::new("test", path, |_, _| Ok(()));
    proxy.start(1).expect("start DVC pipe proxy");

    let mut client = {
        let mut attempts = 0..200;
        loop {
            match UnixStream::connect(path).await {
                Ok(client) => break client,
                Err(_) if attempts.next().is_some() => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("DVC pipe proxy must create the socket within two seconds: {error}"),
            }
        }
    };

    // Far more than the socket buffers, and the client never reads it.
    for _ in 0..64 {
        proxy.process(1, &[0; 64 * 1024]).expect("queue data for the pipe");
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    proxy.close(1);

    // The worker's end is closed once the client's writes fail.
    let released = tokio::time::timeout(Duration::from_secs(2), async {
        while client.write_all(b"x").await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(released.is_ok(), "the closed channel's worker still holds the socket");

    let _ = std::fs::remove_file(path);
}
