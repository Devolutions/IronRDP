use std::io;
use x509_parser::parse_x509_der;

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

pub struct StreamWrapper<T: io::Write + io::Read> {
    stream: T,
}

impl<T: io::Write + io::Read> StreamWrapper<T> {
    pub fn new(stream: T) -> StreamWrapper<T> {
        StreamWrapper { stream }
    }

    pub fn into_inner(self) -> T {
        self.stream
    }

    pub fn get_ref(&self) -> &T {
        &self.stream
    }

    pub fn get_reader(&mut self) -> io::BufReader<&mut T> {
        io::BufReader::new(&mut self.stream)
    }

    pub fn get_writer(&mut self) -> io::BufWriter<&mut T> {
        io::BufWriter::new(&mut self.stream)
    }
}

pub fn socket_addr_to_string(socket_addr: std::net::SocketAddr) -> String {
    format!("{}:{}", socket_addr.ip(), socket_addr.port())
}

pub fn get_tls_peer_pubkey(cert: Vec<u8>) -> io::Result<Vec<u8>> {
    let res = parse_x509_der(&cert[..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid der certificate."))?;
    let public_key = res.1.tbs_certificate.subject_pki.subject_public_key;

    Ok(public_key.data.to_vec())
}
