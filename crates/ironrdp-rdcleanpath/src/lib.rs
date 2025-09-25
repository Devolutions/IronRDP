#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

use core::fmt;

use der::asn1::OctetString;

// Re-export der crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use der;

pub const BASE_VERSION: u64 = 3389;
pub const VERSION_1: u64 = BASE_VERSION + 1;

pub const GENERAL_ERROR_CODE: u16 = 1;
pub const NEGOTIATION_ERROR_CODE: u16 = 2;

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
        let error_description = match self.error_code {
            GENERAL_ERROR_CODE => "general error",
            NEGOTIATION_ERROR_CODE => "negotiation error",
            _ => "unknown error",
        };
        write!(f, "RDCleanPath {error_description} (code {})", self.error_code)?;

        if let Some(http_status_code) = self.http_status_code {
            let description = match http_status_code {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                405 => "Method Not Allowed",
                408 => "Request Timeout",
                409 => "Conflict",
                410 => "Gone",
                413 => "Payload Too Large",
                414 => "URI Too Long",
                422 => "Unprocessable Entity",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                501 => "Not Implemented",
                502 => "Bad Gateway",
                503 => "Service Unavailable",
                504 => "Gateway Timeout",
                505 => "HTTP Version Not Supported",
                _ => "Unknown HTTP Status",
            };
            write!(f, " [HTTP {http_status_code}: {description}]")?;
        }

        if let Some(wsa_last_error) = self.wsa_last_error {
            let description = match wsa_last_error {
                10004 => "Interrupted system call",
                10009 => "Bad file descriptor",
                10013 => "Permission denied",
                10014 => "Bad address",
                10022 => "Invalid argument",
                10024 => "Too many open files",
                10035 => "Resource temporarily unavailable",
                10036 => "Operation now in progress",
                10037 => "Operation already in progress",
                10038 => "Socket operation on nonsocket",
                10039 => "Destination address required",
                10040 => "Message too long",
                10041 => "Protocol wrong type for socket",
                10042 => "Bad protocol option",
                10043 => "Protocol not supported",
                10044 => "Socket type not supported",
                10045 => "Operation not supported",
                10046 => "Protocol family not supported",
                10047 => "Address family not supported by protocol family",
                10048 => "Address already in use",
                10049 => "Cannot assign requested address",
                10050 => "Network is down",
                10051 => "Network is unreachable",
                10052 => "Network dropped connection on reset",
                10053 => "Software caused connection abort",
                10054 => "Connection reset by peer",
                10055 => "No buffer space available",
                10056 => "Socket is already connected",
                10057 => "Socket is not connected",
                10058 => "Cannot send after socket shutdown",
                10060 => "Connection timed out",
                10061 => "Connection refused",
                10064 => "Host is down",
                10065 => "No route to host",
                10067 => "Too many processes",
                10091 => "Network subsystem is unavailable",
                10092 => "Winsock version not supported",
                10093 => "Successful WSAStartup not yet performed",
                10101 => "Graceful shutdown in progress",
                10109 => "Class type not found",
                11001 => "Host not found",
                11002 => "Nonauthoritative host not found",
                11003 => "This is a nonrecoverable error",
                11004 => "Valid name, no data record of requested type",
                _ => "Unknown WSA error",
            };
            write!(f, " [WSA {wsa_last_error}: {description}]")?;
        }

        if let Some(tls_alert_code) = self.tls_alert_code {
            let description = match tls_alert_code {
                0 => "Close notify",
                10 => "Unexpected message",
                20 => "Bad record MAC",
                21 => "Decryption failed",
                22 => "Record overflow",
                30 => "Decompression failure",
                40 => "Handshake failure",
                41 => "No certificate",
                42 => "Bad certificate",
                43 => "Unsupported certificate",
                44 => "Certificate revoked",
                45 => "Certificate expired",
                46 => "Certificate unknown",
                47 => "Illegal parameter",
                48 => "Unknown CA",
                49 => "Access denied",
                50 => "Decode error",
                51 => "Decrypt error",
                60 => "Export restriction",
                70 => "Protocol version",
                71 => "Insufficient security",
                80 => "Internal error",
                90 => "User canceled",
                100 => "No renegotiation",
                109 => "Missing extension",
                110 => "Unsupported extension",
                111 => "Certificate unobtainable",
                112 => "Unrecognized name",
                113 => "Bad certificate status response",
                114 => "Bad certificate hash value",
                115 => "Unknown PSK identity",
                116 => "Certificate required",
                120 => "No application protocol",
                _ => "Unknown TLS alert",
            };
            write!(f, " [TLS alert {tls_alert_code}: {description}]")?;
        }

        Ok(())
    }
}

impl core::error::Error for RDCleanPathErr {}

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

    /// Creates a negotiation error response that includes the server's X.224 negotiation response.
    ///
    /// This allows clients to extract specific negotiation failure details
    /// (like "CredSSP required") from the server's original response.
    ///
    /// # Example
    /// ```rust
    /// use ironrdp_rdcleanpath::RDCleanPathPdu;
    ///
    /// // Server rejected connection with "CredSSP required" - preserve this info
    /// let server_response = vec![/* X.224 Connection Confirm with failure code */];
    /// let error_pdu = RDCleanPathPdu::new_negotiation_error(server_response)?;
    /// # Ok::<(), der::Error>(())
    /// ```
    pub fn new_negotiation_error(server_x224_response: Vec<u8>) -> der::Result<Self> {
        Ok(Self {
            version: VERSION_1,
            error: Some(RDCleanPathErr {
                error_code: NEGOTIATION_ERROR_CODE,
                http_status_code: None,
                wsa_last_error: None,
                tls_alert_code: None,
            }),
            x224_connection_pdu: Some(OctetString::new(server_x224_response)?),
            ..Self::default()
        })
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
    GeneralErr(RDCleanPathErr),
    NegotiationErr {
        x224_connection_response: Vec<u8>,
    },
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

impl core::error::Error for MissingRDCleanPathField {}

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
            let error = pdu.error.ok_or(MissingRDCleanPathField("error"))?;
            match (error.error_code, pdu.x224_connection_pdu) {
                (NEGOTIATION_ERROR_CODE, Some(x224_pdu)) => Self::NegotiationErr {
                    x224_connection_response: x224_pdu.as_bytes().to_vec(),
                },
                _ => Self::GeneralErr(error),
            }
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
            RDCleanPath::GeneralErr(error) => Self {
                version: VERSION_1,
                error: Some(error),
                ..Default::default()
            },
            RDCleanPath::NegotiationErr {
                x224_connection_response,
            } => Self {
                version: VERSION_1,
                error: Some(RDCleanPathErr {
                    error_code: NEGOTIATION_ERROR_CODE,
                    http_status_code: None,
                    wsa_last_error: None,
                    tls_alert_code: None,
                }),
                x224_connection_pdu: Some(
                    OctetString::new(x224_connection_response)
                        .expect("x224_connection_response smaller than u32::MAX (256 MiB)"),
                ),
                ..Default::default()
            },
        }
    }
}
