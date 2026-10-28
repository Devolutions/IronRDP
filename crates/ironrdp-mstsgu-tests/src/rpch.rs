use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, ensure};
use ironrdp_mstsgu::{GwConnectTarget, RpchStream};
use ironrdp_tls::CertificateValidation;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio_native_tls::TlsAcceptor;
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};

use crate::mock_rpch::{
    MockRpchConnector, MockRpchScenario, REAUTH_TUNNEL_CONTEXT, mock_rpch_proxy, mock_rpch_reauth_proxy,
};

pub(crate) async fn check_gateway() -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(20), check_receive_backpressure())
        .await
        .context("unread inbound data blocked outgoing commands")??;
    // oracle: regression Devolutions/IronRDP#2202.
    // oracle: spec MS-TSGU §3.6.4 and MS-RPCE §3.3.2.5.2.
    for scenario in [
        MockRpchScenario::Echo,
        MockRpchScenario::NoCapabilities,
        MockRpchScenario::OutBodyLength(128 * 1024),
        MockRpchScenario::OutBodyLength(2_147_483_648),
        MockRpchScenario::BasicAuthentication,
        MockRpchScenario::FragmentedReceivePipePdu,
        MockRpchScenario::RecycleOutR1,
        MockRpchScenario::RecycleOutR2,
        MockRpchScenario::RecycleOutDelayedHead,
        MockRpchScenario::Messaging,
        MockRpchScenario::MessageCallFault,
        MockRpchScenario::InterleavedMessage,
        MockRpchScenario::PartialMessageFault,
        MockRpchScenario::LargeConsent,
        MockRpchScenario::Reauth,
    ] {
        tokio::time::timeout(Duration::from_secs(20), echo(scenario))
            .await
            .with_context(|| format!("{scenario:?} timed out"))?
            .with_context(|| format!("{scenario:?}"))?;
    }
    for scenario in [
        MockRpchScenario::TamperedResponse,
        MockRpchScenario::InvalidOutFragmentLength,
        MockRpchScenario::TamperedMessageCallFault,
        MockRpchScenario::UnsignedMessageCallFault,
    ] {
        let gateway = Gateway::start(scenario).await?;
        let result = tokio::time::timeout(Duration::from_secs(20), gateway.connect())
            .await
            .with_context(|| format!("gateway connection timed out for {scenario:?}"))?;
        ensure!(result.is_err(), "{scenario:?} exposed a gateway stream");
    }
    for scenario in [
        MockRpchScenario::ReceivePipeError(0x0000_59d8),
        MockRpchScenario::TamperedReceivePipeResponse,
        MockRpchScenario::InvalidReceivePipeFinal,
        MockRpchScenario::UnexpectedResponse,
    ] {
        let gateway = Gateway::start(scenario).await?;
        let mut stream = gateway.connect().await?;
        if matches!(scenario, MockRpchScenario::TamperedReceivePipeResponse) {
            stream.write_all(b"tamper").await?;
            stream.flush().await?;
        }
        let mut data = [0; 8];
        let result = tokio::time::timeout(Duration::from_secs(20), stream.read(&mut data))
            .await
            .with_context(|| format!("terminal result timed out for {scenario:?}"))?;
        ensure!(result.is_err(), "{scenario:?} was exposed as target data");
        ensure!(
            stream.write_all(b"after-failure").await.is_err(),
            "{scenario:?} accepted a later write"
        );
        ensure!(stream.flush().await.is_err(), "{scenario:?} accepted a later flush");
        ensure!(
            stream.shutdown().await.is_err(),
            "{scenario:?} accepted a later shutdown"
        );
    }
    for scenario in [
        MockRpchScenario::RecycleOutRejectedHead,
        MockRpchScenario::InHttpError(0x6ba),
        MockRpchScenario::ReceivePipeOverflow,
    ] {
        let gateway = Gateway::start(scenario).await?;
        let mut stream = gateway.connect().await?;
        stream.write_all(b"request").await?;
        let result = tokio::time::timeout(Duration::from_secs(20), stream.flush())
            .await
            .with_context(|| format!("failed send timed out for {scenario:?}"))?;
        ensure!(result.is_err(), "{scenario:?} completed an invalid RPC send");
    }
    Ok(())
}

pub(crate) async fn check_malformed_replies() -> anyhow::Result<()> {
    echo(MockRpchScenario::Echo).await.context("valid response control")?;
    echo(MockRpchScenario::InvalidHttpResponse(
        b"HTTP/1.1 200 OK\r\nContent-Length: 131072\r\n\r\n",
    ))
    .await
    .context("valid custom HTTP response control")?;
    // oracle: spec MS-TSGU §2.2.9.2.1.6, §2.2.9.2.1.5, §2.2.9.2.1.9.1.1 and §3.2.6.1.4.
    // Offsets name fields in the fixed NDR32 fixture, before RPC authentication is added.
    for (name, opnum, offset, bytes) in [
        ("null tunnel pointer", 1, 12, &[0; 4][..]),
        ("unbounded certificate", 1, 20, &[0xff; 4][..]),
        ("unbounded capability count", 1, 56, &[0xff; 4][..]),
        ("unsupported tunnel version", 1, 60, &[2, 0][..]),
        ("null tunnel context", 1, 84, &[0; 20][..]),
        ("failed tunnel HRESULT", 1, 108, &[0xd8, 0x59, 0x07, 0x80][..]),
        ("unbounded authorization data", 2, 28, &[0xff; 4][..]),
        ("authorization denial", 2, 64, &[0xed, 0x59, 0x07, 0x80][..]),
        ("invalid redirection boolean", 2, 60, &[2, 0, 0, 0][..]),
        ("nonzero reserved redirection flag", 2, 52, &[1, 0, 0, 0][..]),
        ("null channel context", 4, 0, &[0; 20][..]),
        ("failed channel HRESULT", 4, 24, &[0xd8, 0x59, 0x07, 0x80][..]),
    ] {
        let scenario = MockRpchScenario::MalformedReply { opnum, offset, bytes };
        let gateway = Gateway::start(scenario).await?;
        let result = tokio::time::timeout(Duration::from_secs(20), gateway.connect())
            .await
            .with_context(|| format!("{name} timed out"))?;
        ensure!(result.is_err(), "{name} opened a target stream");
    }
    for (name, offset, bytes) in [
        ("nonzero string array offset", 124, &[1, 0, 0, 0][..]),
        ("unbounded string array count", 128, &[0xff; 4][..]),
        ("invalid UTF-16", 132, &[0, 0xd8][..]),
    ] {
        let gateway = Gateway::start(MockRpchScenario::MalformedConsent { offset, bytes }).await?;
        let result = tokio::time::timeout(Duration::from_secs(20), gateway.connect())
            .await
            .with_context(|| format!("{name} timed out"))?;
        ensure!(result.is_err(), "{name} opened a target stream");
    }
    for (opnum, length) in [(1, 16), (2, 20), (4, 23)] {
        let gateway = Gateway::start(MockRpchScenario::TruncatedReply { opnum, length }).await?;
        let result = tokio::time::timeout(Duration::from_secs(20), gateway.connect()).await?;
        ensure!(
            result.is_err(),
            "truncated operation {opnum} reply opened a target stream"
        );
    }
    for (client_name, server) in [("bad\0client", "rdp.contoso.com"), ("client", "bad\0server")] {
        let gateway = Gateway::start(MockRpchScenario::Echo).await?;
        let result = tokio::time::timeout(Duration::from_secs(20), gateway.connect_named(client_name, server)).await?;
        ensure!(result.is_err(), "embedded NUL reached a gateway resource");
    }
    // oracle: spec MS-RPCH §2.1.2.1.4 requires a bounded OUT channel body.
    for (response, diagnostic) in [
        (
            b"HTTP/2 200 OK\r\nContent-Length: 131072\r\n\r\n".as_slice(),
            "invalid rpch HTTP version",
        ),
        (
            b"HTTP/1.1 99 Invalid\r\nContent-Length: 131072\r\n\r\n",
            "invalid rpch response status",
        ),
        (
            b"HTTP/1.1 600 Invalid\r\nContent-Length: 131072\r\n\r\n",
            "invalid rpch response status",
        ),
        (
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
            "unsupported RPCH response transfer encoding",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            "invalid rpch OUT content length",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 131071\r\n\r\n",
            "invalid rpch OUT content length",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 2147483649\r\n\r\n",
            "invalid rpch OUT content length",
        ),
        (b"HTTP/1.1 200 OK\r\n\r\n", "invalid rpch OUT content length"),
    ] {
        let gateway = Gateway::start(MockRpchScenario::InvalidHttpResponse(response)).await?;
        let result = tokio::time::timeout(Duration::from_secs(20), gateway.connect()).await?;
        let error = result.err().context("invalid OUT response opened a target stream")?;
        ensure!(
            format!("{error:#}").contains(diagnostic),
            "invalid OUT response failed outside HTTP validation: {error:#}"
        );
    }
    Ok(())
}

async fn check_receive_backpressure() -> anyhow::Result<()> {
    // oracle: regression Devolutions/IronRDP#2202: unread responses must not block outgoing commands.
    let gateway = Gateway::start(MockRpchScenario::Echo).await?;
    let mut stream = gateway.connect().await?;
    let mut expected = Vec::new();
    for index in 0..20u8 {
        let chunk = [b'a' + index; 4];
        stream.write_all(&chunk).await?;
        expected.extend_from_slice(&chunk);
    }
    let mut response = vec![0; expected.len()];
    stream.read_exact(&mut response).await?;
    ensure!(
        response == expected,
        "receive backpressure lost or reordered target data"
    );
    stream.shutdown().await.context("shutdown after receive backpressure")?;
    Ok(())
}

pub(crate) async fn check_in_recycle() -> anyhow::Result<()> {
    // oracle: spec MS-RPCH §2.1.2.1.1 and §3.2.2.5.5.
    let gateway = Gateway::start(MockRpchScenario::OutBodyLength(2_147_483_648)).await?;
    let stream = gateway.connect().await?;
    let (mut reader, mut writer) = tokio::io::split(stream);
    let chunks = (1024 * 1024 * 1024 / 4096) + 2;
    let writing = async {
        let mut chunk = [0x5a; 4096];
        for index in 0..chunks {
            chunk[..8].copy_from_slice(&u64::try_from(index)?.to_le_bytes());
            writer.write_all(&chunk).await?;
        }
        writer.flush().await?;
        Ok::<_, anyhow::Error>(())
    };
    let reading = async {
        let mut chunk = [0; 4096];
        for index in 0..chunks {
            reader.read_exact(&mut chunk).await?;
            let sequence = u64::from_le_bytes(chunk[..8].try_into().expect("chunk sequence"));
            ensure!(
                sequence == u64::try_from(index)?,
                "target data was lost, reordered, or repeated during IN rollover"
            );
            ensure!(
                chunk[8..].iter().all(|byte| *byte == 0x5a),
                "target data changed during IN rollover"
            );
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::try_join!(writing, reading)?;
    writer.shutdown().await?;
    let completed = gateway
        .state
        .lock()
        .map_err(|_| anyhow::anyhow!("gateway fixture lock poisoned"))?
        .channels
        .values()
        .map(|channel| channel.connector.completed_in_recycles())
        .sum::<usize>();
    ensure!(0 < completed, "gateway did not observe a completed IN channel rollover");
    Ok(())
}

async fn echo(scenario: MockRpchScenario) -> anyhow::Result<()> {
    let mut gateway = Gateway::start(scenario).await?;
    let mut stream = gateway.connect().await?;
    if matches!(scenario, MockRpchScenario::Reauth) {
        let context = gateway
            .reauthenticated
            .recv()
            .await
            .context("reauthentication report")?;
        ensure!(
            context == REAUTH_TUNNEL_CONTEXT,
            "reauthentication changed the tunnel context"
        );
    }
    let mut absent = [0; 1];
    ensure!(
        tokio::time::timeout(Duration::from_millis(1), stream.read_exact(&mut absent))
            .await
            .is_err(),
        "gateway exposed target data before a request"
    );
    for payload in [b"target-data".as_slice(), &[0x5a; 4096]] {
        stream.write_all(payload).await.context("write target data")?;
        stream.flush().await.context("complete RPC send")?;
        let mut received = vec![0; payload.len()];
        stream.read_exact(&mut received).await.context("read target data")?;
        ensure!(received == payload, "target data changed across the gateway");
    }
    stream.write_all(b"unread-on-shutdown").await?;
    stream.shutdown().await.context("close channel and tunnel")?;
    stream.shutdown().await.context("repeat completed shutdown")?;
    Ok(())
}

struct Channels {
    out: Option<DuplexStream>,
    input: Option<DuplexStream>,
    connector: MockRpchConnector,
}

struct State {
    scenario: MockRpchScenario,
    channels: HashMap<String, Channels>,
    proxies: Vec<JoinHandle<()>>,
    reauthenticated: mpsc::UnboundedSender<u64>,
}

impl State {
    fn channel(&mut self, session: &str, out: bool) -> anyhow::Result<DuplexStream> {
        if !self.channels.contains_key(session) {
            let (out, input, connector, task) = if self.channels.is_empty() {
                mock_rpch_proxy(self.scenario)
            } else {
                mock_rpch_reauth_proxy(self.reauthenticated.clone())
            };
            self.proxies.push(task);
            self.channels.insert(
                session.to_owned(),
                Channels {
                    out: Some(out),
                    input: Some(input),
                    connector,
                },
            );
        }
        let channels = self.channels.get_mut(session).context("gateway session")?;
        if out {
            channels.out.take().or_else(|| channels.connector.connect_out())
        } else {
            channels.input.take().or_else(|| channels.connector.connect_in())
        }
        .context("mock gateway stopped")
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for proxy in &self.proxies {
            proxy.abort();
        }
    }
}

struct Gateway {
    endpoint: String,
    listener: JoinHandle<()>,
    state: Arc<Mutex<State>>,
    reauthenticated: mpsc::UnboundedReceiver<u64>,
}

impl Gateway {
    async fn start(scenario: MockRpchScenario) -> anyhow::Result<Self> {
        let identity = Identity::from_pkcs8(
            include_bytes!("../../ironrdp-tls/tests/certs/server-cert.pem"),
            include_bytes!("../../ironrdp-tls/tests/certs/server-key.pem"),
        )?;
        let acceptor = TlsAcceptor::from(NativeTlsAcceptor::new(identity)?);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let endpoint = format!("localhost:{}", listener.local_addr()?.port());
        let (report, reauthenticated) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(State {
            scenario,
            channels: HashMap::new(),
            proxies: Vec::new(),
            reauthenticated: report,
        }));
        let clients = Arc::clone(&state);
        let listener = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let state = Arc::clone(&clients);
                connections.spawn(async move {
                    stream.set_nodelay(true)?;
                    let mut stream = acceptor.accept(stream).await?;
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        ensure!(head.len() < 16 * 1024, "gateway request head exceeded its bound");
                        head.push(stream.read_u8().await?);
                    }
                    let text = core::str::from_utf8(&head)?;
                    let session = text
                        .lines()
                        .find_map(|line| line.strip_prefix("Pragma: SessionId="))
                        .context("gateway session ID")?;
                    let out = text.starts_with("RPC_OUT_DATA ");
                    ensure!(out || text.starts_with("RPC_IN_DATA "), "unexpected gateway method");
                    let mut channel = state
                        .lock()
                        .map_err(|_| anyhow::anyhow!("gateway fixture lock poisoned"))?
                        .channel(session, out)?;
                    channel.write_all(&head).await?;
                    tokio::io::copy_bidirectional(&mut stream, &mut channel).await?;
                    Ok::<_, anyhow::Error>(())
                });
            }
        });
        Ok(Self {
            endpoint,
            listener,
            state,
            reauthenticated,
        })
    }

    async fn connect(&self) -> anyhow::Result<RpchStream> {
        self.connect_named("test-client", "rdp.contoso.com").await
    }

    async fn connect_named(&self, client_name: &str, server: &str) -> anyhow::Result<RpchStream> {
        let target = GwConnectTarget {
            gw_endpoint: self.endpoint.clone(),
            gw_user: "CONTOSO\\alice".to_owned(),
            gw_pass: "secret".to_owned(),
            smart_card: None,
            server: server.to_owned(),
        };
        RpchStream::connect_with_port_and_certificate_validation(
            &target,
            client_name,
            3389,
            CertificateValidation::DangerouslyAcceptInvalidCertificate,
            None,
        )
        .await
        .map(|(stream, _)| stream)
        .context("open public RPCH stream")
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.listener.abort();
        if let Ok(mut state) = self.state.lock() {
            state.proxies.drain(..).for_each(|proxy| proxy.abort());
        }
    }
}
