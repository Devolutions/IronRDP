use std::io;

const TLS_PUBLIC_KEY_HEADER: usize = 24;

#[macro_export]
macro_rules! eof_try {
    ($e:expr) => {
        match $e {
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            result => result,
        }
    };
}

#[macro_export]
macro_rules! try_ready {
    ($e:expr) => {
        match $e {
            Ok(Some(v)) => Ok(v),
            Ok(None) => return Ok(None),
            Err(e) => Err(e),
        }
    };
}

pub fn socket_addr_to_string(socket_addr: std::net::SocketAddr) -> String {
    format!("{}:{}", socket_addr.ip(), socket_addr.port())
}

pub fn get_tls_peer_pubkey<S>(stream: &native_tls::TlsStream<S>) -> io::Result<Vec<u8>>
where
    S: io::Read + io::Write,
{
    let der = get_der_cert_from_stream(&stream)?;
    let cert = openssl::x509::X509::from_der(&der)?;

    get_tls_pubkey_from_cert(cert)
}

fn get_der_cert_from_stream<S>(stream: &native_tls::TlsStream<S>) -> io::Result<Vec<u8>>
where
    S: io::Read + io::Write,
{
    stream
        .peer_certificate()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to get the peer certificate: {}", e),
            )
        })?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "A server must provide the certificate",
            )
        })?
        .to_der()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to convert the peer certificate to der: {}", e),
            )
        })
}

fn get_tls_pubkey_from_cert(cert: openssl::x509::X509) -> io::Result<Vec<u8>> {
    Ok(cert
        .public_key()?
        .public_key_to_der()?
        .split_off(TLS_PUBLIC_KEY_HEADER))
}
