#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://webdevolutions.blob.core.windows.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg"
)]

use core::fmt;

use der::asn1::OctetString;

// Re-export der crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use der;

pub const BASE_VERSION: u64 = 3389;
pub const VERSION_1: u64 = BASE_VERSION + 1;

pub const GENERAL_ERROR_CODE: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, der::Sequence)]
#[asn1(tag_mode = "EXPLICIT")]
pub struct RDCleanPathErr {
    #[asn1(context_specific = "0")]
    pub error_code: u16,
    #[asn1(context_specific = "1", optional = "true")]
    pub http_status_code: Option<u16>,
    #[asn1(context_specific = "2", optional = "true")]
    pub wsa_last_error: Option<u16>,
    #[asn1(context_specific = "3", optional = "true")]
    pub tls_alert_code: Option<u8>,
}

impl fmt::Display for RDCleanPathErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RDCleanPath error (code {})", self.error_code)?;

        if let Some(http_status_code) = self.http_status_code {
            write!(f, " [HTTP status = {http_status_code}]")?;
        }

        if let Some(wsa_last_error) = self.wsa_last_error {
            write!(f, " [WSA last error = {wsa_last_error}]")?;
        }

        if let Some(tls_alert_code) = self.tls_alert_code {
            write!(f, " [TLS alert = {tls_alert_code}]")?;
        }

        Ok(())
    }
}

impl std::error::Error for RDCleanPathErr {}

#[derive(Clone, Debug, Eq, PartialEq, der::Sequence)]
#[asn1(tag_mode = "EXPLICIT")]
pub struct RDCleanPathPdu {
    /// RDCleanPathPdu packet version.
    #[asn1(context_specific = "0")]
    pub version: u64,
    /// The proxy error.
    ///
    /// Sent from proxy to client only.
    #[asn1(context_specific = "1", optional = "true")]
    pub error: Option<RDCleanPathErr>,
    /// The RDP server address itself.
    ///
    /// Sent from client to proxy only.
    #[asn1(context_specific = "2", optional = "true")]
    pub destination: Option<String>,
    /// Arbitrary string for authorization on proxy side.
    ///
    /// Sent from client to proxy only.
    #[asn1(context_specific = "3", optional = "true")]
    pub proxy_auth: Option<String>,
    /// Currently unused. Could be used by a custom RDP server eventually.
    #[asn1(context_specific = "4", optional = "true")]
    pub server_auth: Option<String>,
    /// The RDP PCB forwarded by the proxy to the RDP server.
    ///
    /// Sent from client to proxy only.
    #[asn1(context_specific = "5", optional = "true")]
    pub preconnection_blob: Option<String>,
    /// Either the client handshake or the server handshake response.
    ///
    /// Both client and proxy will set this field.
    #[asn1(context_specific = "6", optional = "true")]
    pub x224_connection_pdu: Option<OctetString>,
    /// The RDP server TLS chain.
    ///
    /// Sent from proxy to client only.
    #[asn1(context_specific = "7", optional = "true")]
    pub server_cert_chain: Option<Vec<OctetString>>,
    // #[asn1(context_specific = "8", optional = "true")]
    // pub ocsp_response: Option<String>,
    /// IPv4 or IPv6 address of the server found by resolving the destination field on proxy side.
    ///
    /// Sent from proxy to client only.
    #[asn1(context_specific = "9", optional = "true")]
    pub server_addr: Option<String>,
}

impl Default for RDCleanPathPdu {
    fn default() -> Self {
        Self {
            version: VERSION_1,
            error: None,
            destination: None,
            proxy_auth: None,
            server_auth: None,
            preconnection_blob: None,
            x224_connection_pdu: None,
            server_cert_chain: None,
            server_addr: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DetectionResult {
    Detected { version: u64, total_length: usize },
    NotEnoughBytes,
    Failed,
}

impl RDCleanPathPdu {
    /// Attempts to decode a RDCleanPath PDU from the provided buffer of bytes.
    pub fn from_der(src: &[u8]) -> der::Result<Self> {
        der::Decode::from_der(src)
    }

    /// Try to parse first few bytes in order to detect a RDCleanPath PDU
    pub fn detect(src: &[u8]) -> DetectionResult {
        use der::{Decode as _, Encode as _};

        let Ok(mut slice_reader) = der::SliceReader::new(src) else {
            return DetectionResult::Failed;
        };

        let header = match der::Header::decode(&mut slice_reader) {
            Ok(header) => header,
            Err(e) => match e.kind() {
                der::ErrorKind::Incomplete { .. } => return DetectionResult::NotEnoughBytes,
                _ => return DetectionResult::Failed,
            },
        };

        let (Ok(header_encoded_len), Ok(body_length)) = (
            header.encoded_len().and_then(usize::try_from),
            usize::try_from(header.length),
        ) else {
            return DetectionResult::Failed;
        };

        let Some(total_length) = header_encoded_len.checked_add(body_length) else {
            return DetectionResult::Failed;
        };

        match der::asn1::ContextSpecific::<u64>::decode_explicit(&mut slice_reader, der::TagNumber::N0) {
            Ok(Some(version)) if version.value == VERSION_1 => DetectionResult::Detected {
                version: VERSION_1,
                total_length,
            },
            Ok(Some(_)) => DetectionResult::Failed,
            Ok(None) => DetectionResult::NotEnoughBytes,
            Err(e) => match e.kind() {
                der::ErrorKind::Incomplete { .. } => DetectionResult::NotEnoughBytes,
                _ => DetectionResult::Failed,
            },
        }
    }

    pub fn into_enum(self) -> Result<RDCleanPath, MissingRDCleanPathField> {
        RDCleanPath::try_from(self)
    }

    pub fn new_general_error() -> Self {
        Self {
            version: VERSION_1,
            error: Some(RDCleanPathErr {
                error_code: GENERAL_ERROR_CODE,
                http_status_code: None,
                wsa_last_error: None,
                tls_alert_code: None,
            }),
            ..Self::default()
        }
    }

    pub fn new_http_error(status_code: u16) -> Self {
        Self {
            version: VERSION_1,
            error: Some(RDCleanPathErr {
                error_code: GENERAL_ERROR_CODE,
                http_status_code: Some(status_code),
                wsa_last_error: None,
                tls_alert_code: None,
            }),
            ..Self::default()
        }
    }

    pub fn new_request(
        x224_pdu: Vec<u8>,
        destination: String,
        proxy_auth: String,
        pcb: Option<String>,
    ) -> der::Result<Self> {
        Ok(Self {
            version: VERSION_1,
            destination: Some(destination),
            proxy_auth: Some(proxy_auth),
            preconnection_blob: pcb,
            x224_connection_pdu: Some(OctetString::new(x224_pdu)?),
            ..Self::default()
        })
    }

    pub fn new_response(
        server_addr: String,
        x224_pdu: Vec<u8>,
        x509_chain: impl IntoIterator<Item = Vec<u8>>,
    ) -> der::Result<Self> {
        Ok(Self {
            version: VERSION_1,
            x224_connection_pdu: Some(OctetString::new(x224_pdu)?),
            server_cert_chain: Some(
                x509_chain
                    .into_iter()
                    .map(OctetString::new)
                    .collect::<der::Result<_>>()?,
            ),
            server_addr: Some(server_addr),
            ..Self::default()
        })
    }

    pub fn new_tls_error(alert_code: u8) -> Self {
        Self {
            version: VERSION_1,
            error: Some(RDCleanPathErr {
                error_code: GENERAL_ERROR_CODE,
                http_status_code: None,
                wsa_last_error: None,
                tls_alert_code: Some(alert_code),
            }),
            ..Self::default()
        }
    }

    pub fn new_wsa_error(wsa_error_code: u16) -> Self {
        Self {
            version: VERSION_1,
            error: Some(RDCleanPathErr {
                error_code: GENERAL_ERROR_CODE,
                http_status_code: None,
                wsa_last_error: Some(wsa_error_code),
                tls_alert_code: None,
            }),
            ..Self::default()
        }
    }

    pub fn to_der(&self) -> der::Result<Vec<u8>> {
        der::Encode::to_der(self)
    }
}

/// Helper enum to leverage Rust pattern matching feature.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RDCleanPath {
    Request {
        destination: String,
        proxy_auth: String,
        server_auth: Option<String>,
        preconnection_blob: Option<String>,
        x224_connection_request: OctetString,
    },
    Response {
        x224_connection_response: OctetString,
        server_cert_chain: Vec<OctetString>,
        server_addr: String,
    },
    Err(RDCleanPathErr),
}

impl RDCleanPath {
    pub fn into_pdu(self) -> RDCleanPathPdu {
        RDCleanPathPdu::from(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MissingRDCleanPathField(&'static str);

impl fmt::Display for MissingRDCleanPathField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RDCleanPath is missing {} field", self.0)
    }
}

impl std::error::Error for MissingRDCleanPathField {}

impl TryFrom<RDCleanPathPdu> for RDCleanPath {
    type Error = MissingRDCleanPathField;

    fn try_from(pdu: RDCleanPathPdu) -> Result<Self, Self::Error> {
        let rdcleanpath = if let Some(destination) = pdu.destination {
            Self::Request {
                destination,
                proxy_auth: pdu.proxy_auth.ok_or(MissingRDCleanPathField("proxy_auth"))?,
                server_auth: pdu.server_auth,
                preconnection_blob: pdu.preconnection_blob,
                x224_connection_request: pdu
                    .x224_connection_pdu
                    .ok_or(MissingRDCleanPathField("x224_connection_pdu"))?,
            }
        } else if let Some(server_addr) = pdu.server_addr {
            Self::Response {
                x224_connection_response: pdu
                    .x224_connection_pdu
                    .ok_or(MissingRDCleanPathField("x224_connection_pdu"))?,
                server_cert_chain: pdu
                    .server_cert_chain
                    .ok_or(MissingRDCleanPathField("server_cert_chain"))?,
                server_addr,
            }
        } else {
            Self::Err(pdu.error.ok_or(MissingRDCleanPathField("error"))?)
        };

        Ok(rdcleanpath)
    }
}

impl From<RDCleanPath> for RDCleanPathPdu {
    fn from(value: RDCleanPath) -> Self {
        match value {
            RDCleanPath::Request {
                destination,
                proxy_auth,
                server_auth,
                preconnection_blob,
                x224_connection_request,
            } => Self {
                version: VERSION_1,
                destination: Some(destination),
                proxy_auth: Some(proxy_auth),
                server_auth,
                preconnection_blob,
                x224_connection_pdu: Some(x224_connection_request),
                ..Default::default()
            },
            RDCleanPath::Response {
                x224_connection_response,
                server_cert_chain,
                server_addr,
            } => Self {
                version: VERSION_1,
                x224_connection_pdu: Some(x224_connection_response),
                server_cert_chain: Some(server_cert_chain),
                server_addr: Some(server_addr),
                ..Default::default()
            },
            RDCleanPath::Err(error) => Self {
                version: VERSION_1,
                error: Some(error),
                ..Default::default()
            },
        }
    }
}
