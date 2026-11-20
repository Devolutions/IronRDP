use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

use ironrdp::connector;
use ironrdp::connector::sspi::credssp::{NStatusCode, TsRequest};
use ironrdp::server::{self, DisplayUpdate, RdpServer};
use ironrdp_async::FramedWrite as _;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, DuplexStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;

use crate::e2e::{hybrid_client_config, hybrid_server, listener_source};

// oracle: mutant not forwarding the selected backend to managed connections.
#[tokio::test]
async fn windows_backend_reaches_managed_connections() {
    let (mut server, _display_tx) = native_server();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        let serve = async {
            let (stream, _) = listener.accept().await.unwrap();
            let error = server.run_connection(stream).await.unwrap_err();
            assert!(format!("{error:?}").contains("native CredSSP requires protocol version 5 or later"));
        };
        tokio::join!(serve, probe_managed_connection(addr));
    })
    .await
    .expect("backend selection must not stall negotiation");
}

// oracle: mutant not forwarding the selected backend to supplied transports.
#[tokio::test]
async fn windows_backend_reaches_supplied_transports() {
    let (mut server, _display_tx) = native_server();
    let (server_stream, client_stream) = tokio::io::duplex(4096);
    tokio::time::timeout(Duration::from_secs(10), async {
        let serve = async {
            let error = server
                .run_connection_with(server_stream, server::TransportTls::AlreadyDone)
                .await
                .unwrap_err();
            assert!(format!("{error:?}").contains("native CredSSP requires protocol version 5 or later"));
        };
        tokio::join!(serve, probe_supplied_connection(client_stream));
    })
    .await
    .expect("backend selection must not stall negotiation");
}

// oracle: mutant dropping the selected backend from the preemption snapshot or managed candidates.
#[tokio::test]
async fn windows_backend_reaches_preempting_managed_connections() {
    let (mut server, _display_tx) = native_server();
    let (mut source, addr) = listener_source(false).await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let stalled_client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (incumbent_tcp, _) = listener.accept().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        let serve = server.serve_connection(
            server::NextConnection::Incoming(server::IncomingConnection::new(
                incumbent_tcp,
                server::TransportTls::Managed,
                None,
            )),
            &mut source,
        );
        tokio::select! {
            _ = serve => panic!("a rejected candidate must not end the incumbent"),
            () = probe_managed_connection(addr) => {}
        }
    })
    .await
    .expect("candidate backend selection must not stall");
    drop(stalled_client);
}

// oracle: mutant dropping the selected backend from the preemption snapshot or supplied candidates.
#[tokio::test]
async fn windows_backend_reaches_preempting_supplied_transports() {
    let (mut server, _display_tx) = native_server();
    let (incumbent, _stalled_client) = tokio::io::duplex(4096);
    let (candidate, client) = tokio::io::duplex(4096);
    let mut source = SuppliedSource(Some(candidate));
    tokio::time::timeout(Duration::from_secs(10), async {
        let serve = server.serve_connection(
            server::NextConnection::Incoming(server::IncomingConnection::new(
                incumbent,
                server::TransportTls::AlreadyDone,
                None,
            )),
            &mut source,
        );
        tokio::select! {
            _ = serve => panic!("a rejected candidate must not end the incumbent"),
            () = probe_supplied_connection(client) => {}
        }
    })
    .await
    .expect("candidate backend selection must not stall");
}

fn native_server() -> (RdpServer, UnboundedSender<DisplayUpdate>) {
    let (mut server, display) = hybrid_server();
    server.set_credentials(None);
    server.set_credssp_backend(server::CredsspBackend::Windows);
    (server, display)
}

async fn probe_managed_connection(addr: SocketAddr) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let mut connector = connector::ClientConnector::new(hybrid_client_config(), stream.local_addr().unwrap());
    let mut framed = ironrdp_tokio::TokioFramed::new(stream);
    ironrdp_async::connect_begin(&mut framed, &mut connector).await.unwrap();
    let (stream, _) = ironrdp_tls::upgrade_with_certificate_validation(
        framed.into_inner_no_leftover(),
        "localhost",
        ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate,
    )
    .await
    .unwrap();
    assert_native_version_rejection(ironrdp_tokio::TokioFramed::new(stream)).await;
}

async fn probe_supplied_connection(stream: DuplexStream) {
    let mut connector = connector::ClientConnector::new(hybrid_client_config(), (Ipv4Addr::LOCALHOST, 0).into());
    let mut framed = ironrdp_tokio::TokioFramed::new(stream);
    ironrdp_async::connect_begin(&mut framed, &mut connector).await.unwrap();
    assert_native_version_rejection(framed).await;
}

async fn assert_native_version_rejection<S>(mut framed: ironrdp_tokio::TokioFramed<S>)
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let mut request = Vec::new();
    TsRequest {
        version: 4,
        ..TsRequest::default()
    }
    .encode_ts_request(&mut request)
    .unwrap();
    framed.write_all(&request).await.unwrap();

    let expected = TsRequest {
        version: 6,
        error_code: Some(NStatusCode(0xc000_00bb)),
        ..TsRequest::default()
    };
    let mut response = Vec::new();
    expected.encode_ts_request(&mut response).unwrap();
    framed.get_inner_mut().0.read_exact(&mut response).await.unwrap();
    let response = TsRequest::from_buffer(&response).unwrap();
    assert_eq!(response.error_code, expected.error_code);
}

struct SuppliedSource(Option<DuplexStream>);

#[async_trait::async_trait]
impl server::ConnectionSource for SuppliedSource {
    type Stream = DuplexStream;

    async fn accept(&mut self) -> std::io::Result<Option<server::IncomingConnection<DuplexStream>>> {
        Ok(self
            .0
            .take()
            .map(|stream| server::IncomingConnection::new(stream, server::TransportTls::AlreadyDone, None)))
    }
}
