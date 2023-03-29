use core::fmt;

use der::asn1::OctetString;

pub const BASE_VERSION: u64 = 3389;
pub const VERSION_1: u64 = BASE_VERSION + 1;

pub const GENERAL_ERROR_CODE: u16 = 1;

// Re-export der crate for convenience
pub use der;

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

pub enum DetectionResult {
    Detected(u64),
    NotEnoughBytes,
    Failed,
}

impl RDCleanPathPdu {
    /// Try to parse first few bytes in order to detect a RDCleanPath PDU
    pub fn detect(src: &[u8]) -> DetectionResult {
        use der::Decode as _;

        #[derive(der::Sequence)]
        #[asn1(tag_mode = "EXPLICIT")]
        pub struct PartialRDCleanPathPdu {
            #[asn1(context_specific = "0")]
            pub version: u64,
        }

        match PartialRDCleanPathPdu::from_der(src) {
            Ok(pdu) => match pdu.version {
                VERSION_1 => DetectionResult::Detected(pdu.version),
                _ => DetectionResult::Failed,
            },
            Err(e) => match e.kind() {
                der::ErrorKind::Incomplete { .. } => DetectionResult::NotEnoughBytes,
                _ => DetectionResult::Failed,
            },
        }
    }

    /// Attempts to decode a RDCleanPath PDU from the provided buffer of bytes.
    pub fn decode(src: &mut bytes::BytesMut) -> der::Result<Option<Self>> {
        use bytes::Buf as _;
        use der::{Decode as _, Encode as _};

        match RDCleanPathPdu::from_der(src) {
            Ok(pdu) => {
                let len = usize::try_from(pdu.encoded_len()?).expect("u32 to usize conversion");
                src.advance(len);
                Ok(Some(pdu))
            }
            Err(e) => match e.kind() {
                der::ErrorKind::Incomplete {
                    expected_len,
                    actual_len: _,
                } => {
                    let expected_len = usize::try_from(expected_len).expect("u32 to usize conversion");
                    src.reserve(expected_len - src.len());
                    Ok(None)
                }
                _ => Err(e),
            },
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

    pub fn to_der(&self) -> der::Result<Vec<u8>> {
        der::Encode::to_vec(self)
    }

    pub fn into_enum(self) -> Result<RDCleanPath, MissingRDCleanPathField> {
        RDCleanPath::try_from(self)
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

#[cfg(test)]
mod tests {
    use der::Decode as _;
    use rstest::rstest;

    use super::*;

    fn request() -> RDCleanPathPdu {
        RDCleanPathPdu::new_request(
            vec![0xDE, 0xAD, 0xBE, 0xFF],
            "destination".to_owned(),
            "proxy auth".to_owned(),
            Some("PCB".to_owned()),
        )
        .unwrap()
    }

    const REQUEST_DER: &[u8] = &[
        0x30, 0x32, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA2, 0xD, 0xC, 0xB, 0x64, 0x65, 0x73, 0x74, 0x69, 0x6E, 0x61,
        0x74, 0x69, 0x6F, 0x6E, 0xA3, 0xC, 0xC, 0xA, 0x70, 0x72, 0x6F, 0x78, 0x79, 0x20, 0x61, 0x75, 0x74, 0x68, 0xA5,
        0x5, 0xC, 0x3, 0x50, 0x43, 0x42, 0xA6, 0x6, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF,
    ];

    fn response_success() -> RDCleanPathPdu {
        RDCleanPathPdu::new_response(
            "192.168.7.95".to_owned(),
            vec![0xDE, 0xAD, 0xBE, 0xFF],
            [
                vec![0xDE, 0xAD, 0xBE, 0xFF],
                vec![0xDE, 0xAD, 0xBE, 0xFF],
                vec![0xDE, 0xAD, 0xBE, 0xFF],
            ],
        )
        .unwrap()
    }

    const RESPONSE_SUCCESS_DER: &[u8] = &[
        0x30, 0x34, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA6, 0x6, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0xA7, 0x14, 0x30,
        0x12, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF,
        0xA9, 0xE, 0xC, 0xC, 0x31, 0x39, 0x32, 0x2E, 0x31, 0x36, 0x38, 0x2E, 0x37, 0x2E, 0x39, 0x35,
    ];

    fn response_http_error() -> RDCleanPathPdu {
        RDCleanPathPdu::new_http_error(500)
    }

    const RESPONSE_HTTP_ERROR_DER: &[u8] = &[
        0x30, 0x15, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA1, 0xD, 0x30, 0xB, 0xA0, 0x3, 0x2, 0x1, 0x1, 0xA1, 0x4, 0x2,
        0x2, 0x1, 0xF4,
    ];

    fn response_tls_error() -> RDCleanPathPdu {
        RDCleanPathPdu::new_tls_error(48)
    }

    const RESPONSE_TLS_ERROR_DER: &[u8] = &[
        0x30, 0x14, 0xA0, 0x04, 0x02, 0x02, 0x0D, 0x3E, 0xA1, 0x0C, 0x30, 0x0A, 0xA0, 0x03, 0x02, 0x01, 0x01, 0xA3,
        0x03, 0x02, 0x01, 0x30,
    ];

    #[rstest]
    #[case(request())]
    #[case(response_success())]
    #[case(response_http_error())]
    #[case(response_tls_error())]
    fn smoke(#[case] message: RDCleanPathPdu) {
        let encoded = message.to_der().unwrap();
        let decoded = RDCleanPathPdu::from_der(&encoded).unwrap();
        assert_eq!(message, decoded);
    }

    macro_rules! assert_serialization {
        ($left:expr, $right:expr) => {{
            if $left != $right {
                let left = hex::encode(&$left);
                let right = hex::encode(&$right);
                let comparison = pretty_assertions::StrComparison::new(&left, &right);
                panic!(
                    "assertion failed: `({} == {})`\n\n{comparison}",
                    stringify!($left),
                    stringify!($right),
                );
            }
        }};
    }

    #[rstest]
    #[case(request(), REQUEST_DER)]
    #[case(response_success(), RESPONSE_SUCCESS_DER)]
    #[case(response_http_error(), RESPONSE_HTTP_ERROR_DER)]
    #[case(response_tls_error(), RESPONSE_TLS_ERROR_DER)]
    fn serialization(#[case] message: RDCleanPathPdu, #[case] expected_der: &[u8]) {
        let encoded = message.to_der().unwrap();
        assert_serialization!(encoded, expected_der);
    }
}
