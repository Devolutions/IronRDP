#[diplomat::bridge]
pub mod ffi {
    use core::fmt::Write as _;

    use anyhow::Context as _;
    use diplomat_runtime::DiplomatWriteable;

    use crate::error::ffi::IronRdpError;
    use crate::error::GenericError;
    use crate::utils::ffi::VecU8;

    #[diplomat::opaque]
    pub struct RDCleanPathPdu(pub ironrdp_rdcleanpath::RDCleanPathPdu);

    impl RDCleanPathPdu {
        /// Creates a new RDCleanPath request PDU
        ///
        /// # Arguments
        /// * `x224_pdu` - The X.224 Connection Request PDU bytes
        /// * `destination` - The destination RDP server address (e.g., "10.10.0.3:3389")
        /// * `proxy_auth` - The JWT authentication token
        /// * `pcb` - Optional preconnection blob (for Hyper-V VM connections, empty string if not needed)
        pub fn new_request(
            x224_pdu: &[u8],
            destination: &str,
            proxy_auth: &str,
            pcb: &str,
        ) -> Result<Box<RDCleanPathPdu>, Box<IronRdpError>> {
            let pcb_opt = if pcb.is_empty() { None } else { Some(pcb.to_owned()) };

            let pdu = ironrdp_rdcleanpath::RDCleanPathPdu::new_request(
                x224_pdu.to_vec(),
                destination.to_owned(),
                proxy_auth.to_owned(),
                pcb_opt,
            )
            .context("failed to create RDCleanPath request")
            .map_err(GenericError)?;

            Ok(Box::new(RDCleanPathPdu(pdu)))
        }

        /// Decodes a RDCleanPath PDU from DER-encoded bytes
        pub fn from_der(bytes: &[u8]) -> Result<Box<RDCleanPathPdu>, Box<IronRdpError>> {
            let pdu = ironrdp_rdcleanpath::RDCleanPathPdu::from_der(bytes)
                .context("failed to decode RDCleanPath PDU")
                .map_err(GenericError)?;

            Ok(Box::new(RDCleanPathPdu(pdu)))
        }

        /// Encodes the RDCleanPath PDU to DER-encoded bytes
        pub fn to_der(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let bytes = self
                .0
                .to_der()
                .context("failed to encode RDCleanPath PDU")
                .map_err(GenericError)?;

            Ok(Box::new(VecU8(bytes)))
        }

        /// Detects if the bytes contain a valid RDCleanPath PDU and returns detection result
        pub fn detect(bytes: &[u8]) -> Box<RDCleanPathDetectionResult> {
            let result = ironrdp_rdcleanpath::RDCleanPathPdu::detect(bytes);
            Box::new(RDCleanPathDetectionResult(result))
        }

        /// Gets the type of this RDCleanPath PDU
        pub fn get_type(&self) -> Result<RDCleanPathResultType, Box<IronRdpError>> {
            let rdcleanpath = self
                .0
                .clone()
                .into_enum()
                .context("missing RDCleanPath field")
                .map_err(GenericError)?;

            let result_type = match rdcleanpath {
                ironrdp_rdcleanpath::RDCleanPath::Request { .. } => RDCleanPathResultType::Request,
                ironrdp_rdcleanpath::RDCleanPath::Response { .. } => RDCleanPathResultType::Response,
                ironrdp_rdcleanpath::RDCleanPath::GeneralErr(_) => RDCleanPathResultType::GeneralError,
                ironrdp_rdcleanpath::RDCleanPath::NegotiationErr { .. } => RDCleanPathResultType::NegotiationError,
            };

            Ok(result_type)
        }

        /// Gets the X.224 connection response bytes (for Response or NegotiationError variants)
        pub fn get_x224_response(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let rdcleanpath = self
                .0
                .clone()
                .into_enum()
                .context("missing RDCleanPath field")
                .map_err(GenericError)?;

            match rdcleanpath {
                ironrdp_rdcleanpath::RDCleanPath::Response {
                    x224_connection_response,
                    ..
                } => Ok(Box::new(VecU8(x224_connection_response.as_bytes().to_vec()))),
                ironrdp_rdcleanpath::RDCleanPath::NegotiationErr {
                    x224_connection_response,
                } => Ok(Box::new(VecU8(x224_connection_response))),
                _ => Err(GenericError(anyhow::anyhow!("RDCleanPath variant does not contain X.224 response")).into()),
            }
        }

        /// Gets the server certificate chain (for Response variant)
        /// Returns a vector iterator of certificate bytes
        pub fn get_server_cert_chain(&self) -> Result<Box<CertificateChainIterator>, Box<IronRdpError>> {
            let rdcleanpath = self
                .0
                .clone()
                .into_enum()
                .context("missing RDCleanPath field")
                .map_err(GenericError)?;

            match rdcleanpath {
                ironrdp_rdcleanpath::RDCleanPath::Response { server_cert_chain, .. } => {
                    let certs: Vec<Vec<u8>> = server_cert_chain.iter().map(|cert| cert.as_bytes().to_vec()).collect();
                    Ok(Box::new(CertificateChainIterator { certs, index: 0 }))
                }
                _ => Err(GenericError(anyhow::anyhow!(
                    "RDCleanPath variant does not contain certificate chain"
                ))
                .into()),
            }
        }

        /// Gets the server address string (for Response variant)
        pub fn get_server_addr<'a>(&'a self, writeable: &'a mut DiplomatWriteable) {
            if let Ok(ironrdp_rdcleanpath::RDCleanPath::Response { server_addr, .. }) = self.0.clone().into_enum() {
                let _ = write!(writeable, "{server_addr}");
            }
        }

        /// Gets error message (for GeneralError variant)
        pub fn get_error_message<'a>(&'a self, writeable: &'a mut DiplomatWriteable) {
            if let Ok(ironrdp_rdcleanpath::RDCleanPath::GeneralErr(err)) = self.0.clone().into_enum() {
                let _ = write!(writeable, "{err}");
            }
        }

        /// Gets the error code (for GeneralError variant)
        pub fn get_error_code(&self) -> Result<u16, Box<IronRdpError>> {
            let rdcleanpath = self
                .0
                .clone()
                .into_enum()
                .context("missing RDCleanPath field")
                .map_err(GenericError)?;

            if let ironrdp_rdcleanpath::RDCleanPath::GeneralErr(err) = rdcleanpath {
                Ok(err.error_code)
            } else {
                Err(GenericError(anyhow::anyhow!("not a GeneralError variant")).into())
            }
        }

        /// Gets the HTTP status code if present (for GeneralError variant)
        /// Returns error if not present or not a GeneralError variant
        pub fn get_http_status_code(&self) -> Result<u16, Box<IronRdpError>> {
            let rdcleanpath = self
                .0
                .clone()
                .into_enum()
                .context("missing RDCleanPath field")
                .map_err(GenericError)?;

            if let ironrdp_rdcleanpath::RDCleanPath::GeneralErr(err) = rdcleanpath {
                err.http_status_code
                    .ok_or_else(|| GenericError(anyhow::anyhow!("HTTP status code not present")).into())
            } else {
                Err(GenericError(anyhow::anyhow!("not a GeneralError variant")).into())
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RDCleanPathResultType {
        Request,
        Response,
        GeneralError,
        NegotiationError,
    }

    #[diplomat::opaque]
    pub struct RDCleanPathDetectionResult(pub ironrdp_rdcleanpath::DetectionResult);

    impl RDCleanPathDetectionResult {
        pub fn is_detected(&self) -> bool {
            matches!(self.0, ironrdp_rdcleanpath::DetectionResult::Detected { .. })
        }

        pub fn is_not_enough_bytes(&self) -> bool {
            matches!(self.0, ironrdp_rdcleanpath::DetectionResult::NotEnoughBytes)
        }

        pub fn is_failed(&self) -> bool {
            matches!(self.0, ironrdp_rdcleanpath::DetectionResult::Failed)
        }

        pub fn get_total_length(&self) -> Result<usize, Box<IronRdpError>> {
            if let ironrdp_rdcleanpath::DetectionResult::Detected { total_length, .. } = self.0 {
                Ok(total_length)
            } else {
                Err(GenericError(anyhow::anyhow!("detection result is not Detected variant")).into())
            }
        }
    }

    #[diplomat::opaque]
    pub struct CertificateChainIterator {
        certs: Vec<Vec<u8>>,
        index: usize,
    }

    impl CertificateChainIterator {
        pub fn next(&mut self) -> Option<Box<VecU8>> {
            if self.index < self.certs.len() {
                let cert = self.certs[self.index].clone();
                self.index += 1;
                Some(Box::new(VecU8(cert)))
            } else {
                None
            }
        }

        pub fn len(&self) -> usize {
            self.certs.len()
        }

        pub fn is_empty(&self) -> bool {
            self.certs.is_empty()
        }
    }
}
