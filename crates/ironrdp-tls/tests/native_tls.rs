#[cfg(windows)]
use ironrdp_tls::endpoint_channel_binding;
use ironrdp_tls::{CertificateValidation, upgrade, upgrade_with_certificate_validation};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::TlsAcceptor;
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use x509_cert as _;

#[tokio::test]
async fn default_accepts_self_signed_certificates_and_strict_rejects_them() {
    let identity = Identity::from_pkcs8(
        include_bytes!("certs/server-cert.pem"),
        include_bytes!("certs/server-key.pem"),
    )
    .expect("create TLS identity");
    let acceptor = TlsAcceptor::from(NativeTlsAcceptor::new(identity).expect("create TLS acceptor"));
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind TLS test listener");
    let address = listener.local_addr().expect("TLS test listener address");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept default TLS client");
        acceptor.accept(stream).await.expect("accept default TLS client");

        let (stream, _) = listener.accept().await.expect("accept strict TLS client");
        let _ = acceptor.accept(stream).await;

        let (stream, _) = listener.accept().await.expect("accept dangerous TLS client");
        acceptor.accept(stream).await.expect("accept dangerous TLS client");
    });

    let (tls_stream, _) = upgrade(
        TcpStream::connect(address).await.expect("connect default TLS client"),
        "localhost",
    )
    .await
    .expect("default validation accepts the self-signed test certificate");
    #[cfg(windows)]
    {
        let channel_binding = endpoint_channel_binding(&tls_stream)
            .expect("get TLS endpoint channel binding")
            .expect("TLS endpoint channel binding");
        assert!(channel_binding[..24].iter().all(|byte| *byte == 0));
        assert_eq!(
            &channel_binding[24..28],
            &(u32::try_from(channel_binding.len() - 32).expect("channel binding length")).to_le_bytes()
        );
        assert_eq!(&channel_binding[28..32], &32u32.to_le_bytes());
        assert!(channel_binding[32..].starts_with(b"tls-server-end-point:"));
    }
    drop(tls_stream);

    let strict_result = upgrade_with_certificate_validation(
        TcpStream::connect(address).await.expect("connect strict TLS client"),
        "localhost",
        CertificateValidation::Strict,
    )
    .await;
    assert!(
        strict_result.is_err(),
        "strict validation must reject the self-signed test certificate"
    );

    let (tls_stream, _) = upgrade_with_certificate_validation(
        TcpStream::connect(address).await.expect("connect dangerous TLS client"),
        "localhost",
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
    )
    .await
    .expect("dangerous policy accepts the self-signed test certificate");
    drop(tls_stream);

    server.await.expect("TLS test server task");
}
