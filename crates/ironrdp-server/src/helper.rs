use std::path::Path;
use std::sync::Arc;

use tokio_rustls::rustls::pki_types::pem::PemObject as _;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::{TlsAcceptor, rustls};

use crate::error::{ServerError, ServerErrorExt as _, ServerResult};

pub struct TlsIdentityCtx {
    pub certs: Vec<CertificateDer<'static>>,
    pub priv_key: PrivateKeyDer<'static>,
    pub pub_key: Vec<u8>,
}

impl TlsIdentityCtx {
    /// A constructor to create a `TlsIdentityCtx` from the given certificate and key paths.
    ///
    /// Both files are read as PEM, whatever their extension.
    /// The key may be PKCS#8, PKCS#1 or SEC1.
    /// Errors reading or parsing either file are [`crate::ServerErrorKind::Custom`].
    pub fn init_from_paths(cert_path: &Path, key_path: &Path) -> ServerResult<Self> {
        let certs = CertificateDer::pem_file_iter(cert_path)
            .map_err(|e| ServerError::custom("reading server cert", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ServerError::custom("collecting server cert", e))?;

        let priv_key =
            PrivateKeyDer::from_pem_file(key_path).map_err(|e| ServerError::custom("reading server key", e))?;

        let pub_key = {
            use x509_cert::der::Decode as _;

            let cert = certs
                .first()
                .ok_or_else(|| ServerError::reason("server cert", "invalid cert"))?;
            let cert =
                x509_cert::Certificate::from_der(cert).map_err(|e| ServerError::custom("parsing server cert", e))?;
            cert.tbs_certificate()
                .subject_public_key_info()
                .subject_public_key
                .as_bytes()
                .ok_or_else(|| ServerError::reason("server cert", "subject public key BIT STRING is not aligned"))?
                .to_owned()
        };

        Ok(Self {
            certs,
            priv_key,
            pub_key,
        })
    }

    pub fn make_acceptor(&self) -> ServerResult<TlsAcceptor> {
        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), self.priv_key.clone_key())
            .map_err(|e| ServerError::custom("bad certificate/key", e))?;

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());

        Ok(TlsAcceptor::from(Arc::new(server_config)))
    }
}
