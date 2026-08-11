//! Full-stack integration tests: client <-> server over loopback UDP.
//!
//! Each test establishes a complete RDPEUDP2 + TLS + RDPEMT tunnel
//! between a client (`connect_udp`) and server (`accept_udp`) on
//! localhost, then exercises bidirectional data transfer.
//!
//! These tests use `multi_thread` flavor because they perform real
//! UDP I/O, which conflicts with tokio's mock clock (test-util).

use core::time::Duration;
use std::sync::Arc;

use ironrdp_rdpemt::TunnelConfig;
use ironrdp_rdpeudp::ConnectionConfig;
use ironrdp_rdpeudp_tokio::{
    MultitransportBootstrap, UdpAcceptConfig, UdpTransport, UdpTransportConfig, accept_udp, connect_udp,
};
use tokio::net::UdpSocket;

/// Generate a self-signed TLS server config for testing.
fn test_tls_server_config() -> Arc<tokio_rustls::rustls::ServerConfig> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("generate cert");

    let cert_der = tokio_rustls::rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
    // rcgen 0.14 renamed `CertifiedKey::key_pair` to `signing_key`.
    let key_der = tokio_rustls::rustls::pki_types::PrivateKeyDer::try_from(cert.signing_key.serialize_der())
        .expect("serialize key");

    let config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("server config");

    Arc::new(config)
}

fn test_tunnel_config() -> TunnelConfig {
    TunnelConfig {
        request_id: 42,
        security_cookie: [0xAB; 16],
    }
}

/// Establish a connected client/server pair over loopback.
///
/// `bind_addr` selects the address family for both ends (`connect_udp`'s own
/// bind must match the server's family, which this exercises end to end).
///
/// Returns `(client, server)`: both are `UdpTransport` handles
/// ready for bidirectional data transfer.
async fn establish_loopback_pair_on(bind_addr: &str) -> (UdpTransport, UdpTransport) {
    let server_sock = UdpSocket::bind(bind_addr).await.expect("bind server");
    let server_addr = server_sock.local_addr().expect("server addr");

    let tunnel_config = test_tunnel_config();

    let server_handle = tokio::spawn({
        let tunnel_config = tunnel_config.clone();
        async move {
            accept_udp(
                server_sock,
                UdpAcceptConfig {
                    tls_config: test_tls_server_config(),
                    tunnel_config,
                    connection_config: ConnectionConfig::default(),
                    accept_timeout: Duration::from_secs(10),
                },
            )
            .await
        }
    });

    let client_handle = tokio::spawn(async move {
        connect_udp(UdpTransportConfig::new(server_addr, "localhost".into(), tunnel_config)).await
    });

    let (server_result, client_result) = tokio::join!(server_handle, client_handle);

    let server_transport = server_result.expect("server join").expect("server accept_udp");
    let client_transport = client_result.expect("client join").expect("client connect_udp");

    (client_transport, server_transport)
}

/// Verify the full connection sequence completes on loopback.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_stack_loopback_handshake() {
    let (client, server) = establish_loopback_pair_on("127.0.0.1:0").await;
    assert!(client.is_alive());
    assert!(server.is_alive());
    client.shutdown().await.expect("client shutdown");
    server.shutdown().await.expect("server shutdown");
}

/// Verify the full connection sequence completes on IPv6 loopback.
///
/// `connect_udp` binds its own socket to match `server_addr`'s family; this
/// is the only test that exercises the IPv6 side of that match, guarding
/// against a hardcoded-IPv4 regression.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_stack_loopback_handshake_ipv6() {
    let (client, server) = establish_loopback_pair_on("[::1]:0").await;
    assert!(client.is_alive());
    assert!(server.is_alive());
    client.shutdown().await.expect("client shutdown");
    server.shutdown().await.expect("server shutdown");
}

/// Verify bidirectional data transfer through the tunnel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_stack_bidirectional_data() {
    let (mut client, mut server) = establish_loopback_pair_on("127.0.0.1:0").await;

    // Client -> Server
    client.send(vec![0x01, 0x02, 0x03]).await.expect("client send");
    let received = server.recv().await.expect("server recv");
    assert_eq!(received, vec![0x01, 0x02, 0x03]);

    // Server -> Client
    server.send(vec![0x04, 0x05, 0x06]).await.expect("server send");
    let received = client.recv().await.expect("client recv");
    assert_eq!(received, vec![0x04, 0x05, 0x06]);

    client.shutdown().await.expect("client shutdown");
    server.shutdown().await.expect("server shutdown");
}

/// A payload over the wire `PayloadLength` field's 65535-byte capacity must
/// be rejected synchronously by `send()`, and must not take the write pump
/// down with it: a normal-sized send right after must still succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_rejects_oversized_payload_without_killing_the_pump() {
    let (client, mut server) = establish_loopback_pair_on("127.0.0.1:0").await;

    let oversized = vec![0u8; 65536];
    let result = client.send(oversized).await;
    assert!(result.is_err(), "a 65536-byte payload must be rejected");

    // The pump must still be alive: an ordinary send right after succeeds.
    client.send(vec![0x01, 0x02, 0x03]).await.expect("send after rejection");
    let received = server.recv().await.expect("server recv");
    assert_eq!(received, vec![0x01, 0x02, 0x03]);

    client.shutdown().await.expect("client shutdown");
    server.shutdown().await.expect("server shutdown");
}

/// Verify multiple messages maintain ordering through the tunnel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_stack_message_ordering() {
    let (client, mut server) = establish_loopback_pair_on("127.0.0.1:0").await;

    // Send multiple messages rapidly
    for i in 0u8..10 {
        client.send(vec![i]).await.expect("send");
    }

    // Verify they arrive in order
    for i in 0u8..10 {
        let received = server.recv().await.expect("recv");
        assert_eq!(received, vec![i], "message {i} arrived out of order");
    }

    client.shutdown().await.expect("shutdown");
    server.shutdown().await.expect("shutdown");
}

/// Verify that a payload larger than one RDPEUDP2 packet survives the full
/// stack.
///
/// Sized well past the 1232 byte MTU on purpose, to check that a split write
/// comes back together. It does not check that the split happened: loopback
/// carries an oversized datagram perfectly well, which is why an unsegmented
/// 16 KiB write passed here for as long as it did. The datagram sizes
/// themselves are asserted in `rdpeudp::connection`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_stack_larger_payload() {
    let (client, mut server) = establish_loopback_pair_on("127.0.0.1:0").await;

    let payload: Vec<u8> = (0..8192u32)
        .map(|i| u8::try_from(i % 256).expect("modulo 256 fits in u8"))
        .collect();
    client.send(payload.clone()).await.expect("send");

    let received = server.recv().await.expect("recv");
    assert_eq!(received, payload);

    client.shutdown().await.expect("shutdown");
    server.shutdown().await.expect("shutdown");
}

/// Verify tunnel rejection when the server has a different security cookie.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tunnel_rejection_mismatched_cookie() {
    let server_sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = server_sock.local_addr().expect("addr");

    // Server expects a different cookie than the client will send
    let server_config = TunnelConfig {
        request_id: 42,
        security_cookie: [0xFF; 16],
    };
    let client_config = TunnelConfig {
        request_id: 42,
        security_cookie: [0xAA; 16],
    };

    let server_handle = tokio::spawn(async move {
        accept_udp(
            server_sock,
            UdpAcceptConfig {
                tls_config: test_tls_server_config(),
                tunnel_config: server_config,
                connection_config: ConnectionConfig::default(),
                accept_timeout: Duration::from_secs(10),
            },
        )
        .await
    });

    let client_handle = tokio::spawn(async move {
        connect_udp(UdpTransportConfig::new(server_addr, "localhost".into(), client_config)).await
    });

    let (server_result, client_result) = tokio::join!(server_handle, client_handle);

    // The mismatch is now caught during the RDPEUDP handshake rather than at
    // the tunnel: the client's SYN carries the SHA-256 of its cookie
    // ([MS-RDPEUDP] 2.2.2.9) and the server compares it against its own, so an
    // unauthenticated peer never reaches the TLS handshake.
    let server_err = server_result.expect("server join").is_err();
    let client_err = client_result.expect("client join").is_err();

    assert!(
        server_err || client_err,
        "at least one side should fail with mismatched cookies"
    );
}

/// A `ServerCertVerifier` that rejects every certificate, used to prove a
/// caller-supplied verifier is actually consulted rather than silently
/// ignored in favor of the built-in no-verification default.
#[derive(Debug)]
struct AlwaysRejectVerifier;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for AlwaysRejectVerifier {
    fn verify_server_cert(
        &self,
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error> {
        Err(tokio_rustls::rustls::Error::General(
            "test verifier rejects every certificate".into(),
        ))
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        vec![tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

/// Verify a caller-supplied `server_cert_verifier` is actually used: the
/// server's self-signed test certificate must be rejected when the caller
/// asks for real validation, where the default (no verifier supplied)
/// accepts it unconditionally.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_rejects_certificate_when_caller_supplies_a_verifier() {
    let server_sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
    let server_addr = server_sock.local_addr().expect("addr");
    let tunnel_config = test_tunnel_config();

    let server_handle = tokio::spawn({
        let tunnel_config = tunnel_config.clone();
        async move {
            accept_udp(
                server_sock,
                UdpAcceptConfig {
                    tls_config: test_tls_server_config(),
                    tunnel_config,
                    connection_config: ConnectionConfig::default(),
                    accept_timeout: Duration::from_secs(10),
                },
            )
            .await
        }
    });

    let client_handle = tokio::spawn(async move {
        let mut config = UdpTransportConfig::new(server_addr, "localhost".into(), tunnel_config);
        config.server_cert_verifier = Some(Arc::new(AlwaysRejectVerifier));
        connect_udp(config).await
    });

    let client_result = client_handle.await.expect("client join");
    assert!(
        client_result.is_err(),
        "the self-signed test certificate must be rejected once a verifier is supplied"
    );

    // The server side also errors, since the client aborts mid-handshake.
    let _ = server_handle.await;
}

/// A failed reconnect must not leave a stale transport from an earlier
/// successful `connect()` reachable through `is_connected()` /
/// `take_transport()`: the bootstrap just told the server the connection
/// is down via the abort response, so its own state must agree.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_reconnect_clears_the_transport_from_an_earlier_success() {
    let server_sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind server");
    let server_addr = server_sock.local_addr().expect("server addr");
    let tunnel_config = test_tunnel_config();

    let server_handle = tokio::spawn({
        let tunnel_config = tunnel_config.clone();
        async move {
            accept_udp(
                server_sock,
                UdpAcceptConfig {
                    tls_config: test_tls_server_config(),
                    tunnel_config,
                    connection_config: ConnectionConfig::default(),
                    accept_timeout: Duration::from_secs(10),
                },
            )
            .await
        }
    });

    // Wire bytes for a MultitransportRequestPdu (UdpFecR), built by hand to
    // avoid a new ironrdp-pdu dev-dependency for a single test. Layout per
    // MS-RDPBCGR 2.2.15.1: BasicSecurityHeader, requestId, requestedProtocol,
    // 2 reserved bytes, 16-byte securityCookie. Must match the server's
    // test_tunnel_config() (request_id 42, cookie 0xAB) or the RDPEUDP2 SYN's
    // cookie hash mismatches and the server silently ignores it, which times
    // out rather than failing fast.
    let request_wire = {
        let mut wire = vec![0x02, 0x00, 0x00, 0x00]; // BasicSecurityHeader: TRANSPORT_REQ
        wire.extend_from_slice(&42u32.to_le_bytes()); // requestId
        wire.extend_from_slice(&1u16.to_le_bytes()); // requestedProtocol = UdpFecR
        wire.extend_from_slice(&[0x00, 0x00]); // reserved
        wire.extend_from_slice(&[0xAB; 16]); // securityCookie
        wire
    };
    let mut bootstrap = MultitransportBootstrap::from_pdu(&request_wire).expect("decode request");
    bootstrap
        .connect(server_addr, "localhost".into(), ConnectionConfig::default())
        .await
        .expect("first connect succeeds");
    assert!(bootstrap.is_connected());
    server_handle.await.expect("server join").expect("server accept_udp");

    // Reconnect against a closed loopback port nothing is listening on.
    let unreachable_addr: core::net::SocketAddr = "127.0.0.1:1".parse().expect("valid loopback address");
    let result = bootstrap
        .connect(unreachable_addr, "localhost".into(), ConnectionConfig::default())
        .await;
    assert!(result.is_err(), "reconnect against an unreachable address must fail");

    assert!(
        !bootstrap.is_connected(),
        "is_connected() must not report the stale transport from the earlier success"
    );
    assert!(
        bootstrap.take_transport().is_none(),
        "take_transport() must not hand back the stale transport"
    );
}
