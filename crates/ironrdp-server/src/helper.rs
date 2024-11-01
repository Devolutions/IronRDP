use std::{fs::File, io::BufReader, path::Path, sync::Arc};

use anyhow::Context;
use rustls_pemfile::{certs, pkcs8_private_keys};
use tokio_rustls::{rustls, TlsAcceptor};

pub struct TlsIdentityCtx {
    pub cert: rustls::pki_types::CertificateDer<'static>,
    pub priv_key: rustls::pki_types::PrivateKeyDer<'static>,
    pub pub_key: Vec<u8>,
}

impl TlsIdentityCtx {
    pub fn init_from_paths(cert_path: &Path, key_path: &Path) -> anyhow::Result<Self> {
        use x509_cert::der::Decode as _;

        let cert = certs(&mut BufReader::new(File::open(cert_path)?))
            .next()
            .context("no certificate")??;

        let pub_key = {
            let cert = x509_cert::Certificate::from_der(&cert).map_err(std::io::Error::other)?;
            cert.tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_bytes()
                .ok_or_else(|| std::io::Error::other("subject public key BIT STRING is not aligned"))?
                .to_owned()
        };

        let priv_key = pkcs8_private_keys(&mut BufReader::new(File::open(key_path)?))
            .next()
            .context("no private key")?
            .map(rustls::pki_types::PrivateKeyDer::from)?;

        Ok(Self {
            cert,
            priv_key,
            pub_key,
        })
    }

    pub fn make_acceptor(&self) -> anyhow::Result<TlsAcceptor> {
        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![self.cert.clone()], self.priv_key.clone_key())
            .context("bad certificate/key")?;

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());

        Ok(TlsAcceptor::from(Arc::new(server_config)))
    }
}
