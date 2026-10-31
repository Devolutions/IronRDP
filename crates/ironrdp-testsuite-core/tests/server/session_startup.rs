use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironrdp_acceptor::Acceptor;
use ironrdp_server::{
    BoundConnection, ConnectionBinder, CredentialDecision, CredentialOrigin, CredentialValidationError,
    CredentialValidator, Credentials, DesktopSize, DisplayContext, DisplayUpdate, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler, ReceivedCredentials, ServerErrorKind, ServerResult,
    StaticChannelFactory, captured_credssp_identity_for_test,
};
use ironrdp_tokio::TokioFramed;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use super::activation::{play_client_after_connection_request, start_client_connection_request};

const CLIENT_SIZE: DesktopSize = DesktopSize {
    width: 1280,
    height: 720,
};

type Lifecycle = Arc<Mutex<Vec<&'static str>>>;

#[test]
fn client_info_credentials_are_not_exposed_as_captured_identity() {
    // oracle: mutant exposing ClientInfo credentials as a captured identity.
    let received = ReceivedCredentials {
        credentials: Credentials {
            username: "alice".to_owned(),
            password: "secret".to_owned(),
            domain: Some("CORP".to_owned()),
        },
        origin: CredentialOrigin::ClientInfo,
    };

    assert!(captured_credssp_identity_for_test(Some(&received)).unwrap().is_none());
}

#[test]
fn credssp_delegated_credentials_are_captured() {
    // oracle: mutant dropping a delegated CredSSP identity.
    let received = ReceivedCredentials {
        credentials: Credentials {
            username: "alice".to_owned(),
            password: "secret".to_owned(),
            domain: Some("CORP".to_owned()),
        },
        origin: CredentialOrigin::CredSspDelegated,
    };

    let identity = captured_credssp_identity_for_test(Some(&received)).unwrap().unwrap();
    assert_eq!(
        identity,
        ironrdp_server::CredsspAuthIdentity {
            username: ironrdp_connector::sspi::Username::new("alice", Some("CORP")).unwrap(),
            password: "secret".to_owned().into(),
        }
    );
}

struct RecordingChannelFactory(Lifecycle);

impl StaticChannelFactory for RecordingChannelFactory {
    fn attach(&self, _acceptor: &mut Acceptor) {
        self.0.lock().unwrap().push("channels");
    }
}

struct RecordingValidator(Lifecycle);

#[async_trait]
impl CredentialValidator for RecordingValidator {
    async fn validate(&self, _credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        self.0.lock().unwrap().push("validate");
        Ok(CredentialDecision::Accept)
    }
}

struct RecordingBinder {
    lifecycle: Lifecycle,
    activated: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait]
impl ConnectionBinder for RecordingBinder {
    async fn bind_connection(
        &self,
        _credentials: &Credentials,
        desktop_size: DesktopSize,
    ) -> ServerResult<BoundConnection> {
        self.lifecycle.lock().unwrap().push("bind");
        Ok(BoundConnection {
            display: Box::new(RecordingDisplay {
                lifecycle: Arc::clone(&self.lifecycle),
                size: desktop_size,
                activated: self.activated.lock().unwrap().take(),
            }),
            input: Box::new(NoopInput),
        })
    }
}

struct RecordingDisplay {
    lifecycle: Lifecycle,
    size: DesktopSize,
    activated: Option<oneshot::Sender<()>>,
}

#[async_trait]
impl RdpServerDisplay for RecordingDisplay {
    async fn size(&mut self) -> DesktopSize {
        self.size
    }

    async fn updates(&mut self, _context: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        self.lifecycle.lock().unwrap().push("activate");
        if let Some(activated) = self.activated.take() {
            activated.send(()).unwrap();
        }
        Ok(Box::new(PendingUpdates))
    }
}

struct PendingUpdates;

#[async_trait]
impl RdpServerDisplayUpdates for PendingUpdates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        core::future::pending().await
    }
}

struct NoopInput;

impl RdpServerInputHandler for NoopInput {
    fn keyboard(&mut self, _event: ironrdp_server::KeyboardEvent) {}

    fn mouse(&mut self, _event: ironrdp_server::MouseEvent) {}
}

struct PeerRecorder(Arc<Mutex<Vec<Option<std::net::SocketAddr>>>>);

impl ironrdp_server::ConnectionHandler for PeerRecorder {
    fn on_connection_info(&mut self, info: &ironrdp_server::ConnectionInfo) {
        self.0.lock().unwrap().push(info.peer_addr);
    }
}

struct ClosedTcpSource;

#[async_trait]
impl ironrdp_server::ConnectionSource for ClosedTcpSource {
    type Stream = TcpStream;

    async fn accept(&mut self) -> std::io::Result<Option<ironrdp_server::IncomingConnection<TcpStream>>> {
        Ok(None)
    }
}

#[tokio::test]
async fn handshake_defers_client_info_validation_binding_channels_and_activation_to_session_start() {
    // oracle: mutant allowing session startup to exit before activation.
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let (activated_tx, activated_rx) = oneshot::channel();
    let binder = Arc::new(RecordingBinder {
        lifecycle: Arc::clone(&lifecycle),
        activated: Mutex::new(Some(activated_tx)),
    });
    let (client_ready_tx, client_ready_rx) = oneshot::channel();
    let (close_client_tx, close_client_rx) = oneshot::channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_input_handler(NoopInput)
        .with_display_handler(RecordingDisplay {
            lifecycle: Arc::clone(&lifecycle),
            size: CLIENT_SIZE,
            activated: None,
        })
        .with_credential_validator(Some(Arc::new(RecordingValidator(Arc::clone(&lifecycle)))))
        .with_connection_binder(Some(binder))
        .with_static_channel_factory(Box::new(RecordingChannelFactory(Arc::clone(&lifecycle))))
        .build();

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (server_stream, client) = tokio::join!(listener.accept(), TcpStream::connect(listener.local_addr().unwrap()));
    let server_stream = server_stream.unwrap().0;
    let client = client.unwrap();
    let (continue_tx, continue_rx) = oneshot::channel();
    let client_task = tokio::spawn(async move {
        let mut framed = TokioFramed::new(client);
        start_client_connection_request(&mut framed).await;
        continue_rx.await.unwrap();
        play_client_after_connection_request(&mut framed, true, &[], None).await;
        activated_rx.await.unwrap();
        client_ready_tx.send(()).unwrap();
        close_client_rx.await.unwrap();
        framed.read_pdu().await.is_err()
    });

    let pending = tokio::time::timeout(Duration::from_secs(5), server.run_connection_handshake(server_stream))
        .await
        .expect("connection handshake completed")
        .expect("connection handshake succeeded");
    assert!(lifecycle.lock().unwrap().is_empty());

    continue_tx.send(()).unwrap();
    {
        let session = server.run_connection_session(pending);
        tokio::pin!(session);
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                result = &mut session => panic!("session exited before cancellation: {result:?}"),
                client_ready = client_ready_rx => client_ready.expect("client observed activation"),
            }
        })
        .await
        .expect("session activation completed");

        let lifecycle = lifecycle.lock().unwrap();
        let validation = lifecycle.iter().position(|event| *event == "validate").unwrap();
        let binding = lifecycle.iter().position(|event| *event == "bind").unwrap();
        let activation = lifecycle.iter().position(|event| *event == "activate").unwrap();
        assert!(lifecycle.contains(&"channels"));
        assert!(validation < binding);
        assert!(binding < activation);
    }

    // oracle: mutant retaining a cancelled pending connection's local address.
    assert_eq!(server.connection_local_addr_for_test(), None);
    close_client_tx.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), client_task)
            .await
            .expect("client observed session cancellation")
            .expect("client task completed")
    );
}

#[tokio::test]
async fn dropping_pending_session_closes_transport_without_attaching_channels() {
    // oracle: mutant leaking the pending transport on drop.
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_static_channel_factory(Box::new(RecordingChannelFactory(Arc::clone(&lifecycle))))
        .build();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (server_stream, client) = tokio::join!(listener.accept(), TcpStream::connect(listener.local_addr().unwrap()));
    let server_stream = server_stream.unwrap().0;
    let client = client.unwrap();
    let (check_disconnect_tx, check_disconnect_rx) = oneshot::channel();
    let client_task = tokio::spawn(async move {
        let mut framed = TokioFramed::new(client);
        start_client_connection_request(&mut framed).await;
        check_disconnect_rx.await.unwrap();
        framed.read_pdu().await.is_err()
    });

    let pending = tokio::time::timeout(Duration::from_secs(5), server.run_connection_handshake(server_stream))
        .await
        .expect("connection handshake completed")
        .expect("connection handshake succeeded");
    drop(pending);
    check_disconnect_tx.send(()).unwrap();

    assert!(
        tokio::time::timeout(Duration::from_secs(5), client_task)
            .await
            .expect("peer observed pending transport drop")
            .expect("client task completed")
    );
    assert!(lifecycle.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pending_session_cannot_be_consumed_by_another_server() {
    // oracle: mutant permitting cross-server pending-session consumption.
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let mut origin_server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build();
    let mut other_server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_static_channel_factory(Box::new(RecordingChannelFactory(Arc::clone(&lifecycle))))
        .build();
    let other_local_addr = ([127, 0, 0, 2], 3389).into();
    other_server.set_connection_local_addr(Some(other_local_addr));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (server_stream, client) = tokio::join!(listener.accept(), TcpStream::connect(listener.local_addr().unwrap()));
    let server_stream = server_stream.unwrap().0;
    let client = client.unwrap();
    let (close_client_tx, close_client_rx) = oneshot::channel();
    let client_task = tokio::spawn(async move {
        let mut framed = TokioFramed::new(client);
        start_client_connection_request(&mut framed).await;
        close_client_rx.await.unwrap();
        framed.read_pdu().await.is_err()
    });

    let pending = tokio::time::timeout(
        Duration::from_secs(5),
        origin_server.run_connection_handshake(server_stream),
    )
    .await
    .expect("connection handshake completed")
    .expect("connection handshake succeeded");
    let result = tokio::time::timeout(Duration::from_secs(2), other_server.run_connection_session(pending)).await;
    assert!(lifecycle.lock().unwrap().is_empty());
    let error = result.expect("different server rejected pending session").unwrap_err();
    assert!(matches!(
        error.kind(),
        ServerErrorKind::Reason(reason) if reason == "belongs to a different server instance"
    ));
    // oracle: mutant clearing another server's caller-supplied address on rejection.
    assert_eq!(other_server.connection_local_addr_for_test(), Some(other_local_addr));
    close_client_tx.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), client_task)
            .await
            .expect("peer observed rejected pending session")
            .expect("client task completed")
    );
    assert!(lifecycle.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pending_session_discards_events_left_by_an_intervening_session() {
    // oracle: mutant delivering an intervening session's disconnect to a resumed pending session.
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let peers = Arc::new(Mutex::new(Vec::new()));
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_input_handler(NoopInput)
        .with_display_handler(RecordingDisplay {
            lifecycle: Arc::clone(&lifecycle),
            size: CLIENT_SIZE,
            activated: None,
        })
        .with_connection_handler(Some(Box::new(PeerRecorder(Arc::clone(&peers)))))
        .build();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (original_stream, original_client) =
        tokio::join!(listener.accept(), TcpStream::connect(listener.local_addr().unwrap()));
    let (resume_tx, resume_rx) = oneshot::channel();
    let (original_ready_tx, original_ready_rx) = oneshot::channel();
    let (original_close_tx, original_close_rx) = oneshot::channel();
    let original_task = tokio::spawn(async move {
        let mut framed = TokioFramed::new(original_client.unwrap());
        start_client_connection_request(&mut framed).await;
        resume_rx.await.unwrap();
        play_client_after_connection_request(&mut framed, true, &[], None).await;
        original_ready_tx.send(()).unwrap();
        original_close_rx.await.unwrap();
        framed.read_pdu().await.is_err()
    });
    let pending = tokio::time::timeout(
        Duration::from_secs(5),
        server.run_connection_handshake(original_stream.unwrap().0),
    )
    .await
    .unwrap()
    .unwrap();

    let (intervening_stream, intervening_client) =
        tokio::join!(listener.accept(), TcpStream::connect(listener.local_addr().unwrap()));
    let (intervening_ready_tx, intervening_ready_rx) = oneshot::channel();
    let (intervening_close_tx, intervening_close_rx) = oneshot::channel();
    let intervening_task = tokio::spawn(async move {
        let mut framed = TokioFramed::new(intervening_client.unwrap());
        super::activation::play_client_with_capabilities(&mut framed, true, &[], None).await;
        intervening_ready_tx.send(()).unwrap();
        intervening_close_rx.await.unwrap();
        framed.read_pdu().await.is_err()
    });
    let intervening_stream = intervening_stream.unwrap().0;
    let intervening_peer = intervening_stream.peer_addr().unwrap();
    {
        let mut source = ClosedTcpSource;
        let intervening = server.serve_connection(
            ironrdp_server::NextConnection::Incoming(ironrdp_server::IncomingConnection::new(
                intervening_stream,
                ironrdp_server::TransportTls::Managed,
                Some(intervening_peer),
            )),
            &mut source,
        );
        tokio::pin!(intervening);
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                result = &mut intervening => panic!("intervening session exited: {result:?}"),
                ready = intervening_ready_rx => ready.unwrap(),
            }
        })
        .await
        .unwrap();
    }
    intervening_close_tx.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), intervening_task)
            .await
            .unwrap()
            .unwrap()
    );
    server
        .event_sender()
        .send(ironrdp_server::ServerEvent::Disconnect(
            ironrdp_pdu::rdp::server_error_info::ErrorInfo::ProtocolIndependentCode(
                ironrdp_pdu::rdp::server_error_info::ProtocolIndependentCode::RpcInitiatedDisconnect,
            ),
        ))
        .unwrap();

    resume_tx.send(()).unwrap();
    {
        let resumed = server.run_connection_session(pending);
        tokio::pin!(resumed);
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                result = &mut resumed => panic!("stale disconnect reached pending session: {result:?}"),
                ready = original_ready_rx => ready.unwrap(),
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut resumed)
                    .await
                    .is_err(),
                "the resumed session must remain live"
            );
        })
        .await
        .unwrap();
    }
    // oracle: mutant attributing the resumed pending connection to the intervening session's peer.
    assert_eq!(*peers.lock().unwrap(), [Some(intervening_peer), None]);
    original_close_tx.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), original_task)
            .await
            .unwrap()
            .unwrap()
    );
}
