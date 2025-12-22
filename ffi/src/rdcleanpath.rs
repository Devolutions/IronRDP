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
                Some(x224_pdu.to_vec()),
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
            if self.0.destination.is_some() {
                if self.0.proxy_auth.is_none() {
                    return Err(Self::missing_field("proxy_auth"));
                }

                if self.0.x224_connection_pdu.is_none() {
                    return Err(Self::missing_field("x224_connection_pdu"));
                }

                Ok(RDCleanPathResultType::Request)
            } else if self.0.server_addr.is_some() {
                if self.0.x224_connection_pdu.is_none() {
                    return Err(Self::missing_field("x224_connection_pdu"));
                }

                if self.0.server_cert_chain.is_none() {
                    return Err(Self::missing_field("server_cert_chain"));
                }

                Ok(RDCleanPathResultType::Response)
            } else if let Some(error) = &self.0.error {
                if error.error_code == ironrdp_rdcleanpath::NEGOTIATION_ERROR_CODE {
                    if self.0.x224_connection_pdu.is_none() {
                        return Err(Self::missing_field("x224_connection_pdu"));
                    }

                    Ok(RDCleanPathResultType::NegotiationError)
                } else {
                    Ok(RDCleanPathResultType::GeneralError)
                }
            } else {
                Err(Self::missing_field("error"))
            }
        }

        /// Gets the X.224 connection response bytes (for Response or NegotiationError variants)
        pub fn get_x224_response(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            if self.0.server_addr.is_some() {
                let x224 = self
                    .0
                    .x224_connection_pdu
                    .as_ref()
                    .ok_or_else(|| Self::missing_field("x224_connection_pdu"))?;
                self.0
                    .server_cert_chain
                    .as_ref()
                    .ok_or_else(|| Self::missing_field("server_cert_chain"))?;

                Ok(Box::new(VecU8(x224.as_bytes().to_vec())))
            } else if let Some(error) = &self.0.error {
                if error.error_code == ironrdp_rdcleanpath::NEGOTIATION_ERROR_CODE {
                    let x224 = self
                        .0
                        .x224_connection_pdu
                        .as_ref()
                        .ok_or_else(|| Self::missing_field("x224_connection_pdu"))?;

                    Ok(Box::new(VecU8(x224.as_bytes().to_vec())))
                } else {
                    Err(GenericError(anyhow::anyhow!("RDCleanPath variant does not contain X.224 response")).into())
                }
            } else {
                Err(GenericError(anyhow::anyhow!("RDCleanPath variant does not contain X.224 response")).into())
            }
        }

        /// Gets the server certificate chain (for Response variant)
        /// Returns a vector iterator of certificate bytes
        pub fn get_server_cert_chain(&self) -> Result<Box<CertificateChainIterator>, Box<IronRdpError>> {
            if self.0.server_addr.is_some() {
                self.0
                    .x224_connection_pdu
                    .as_ref()
                    .ok_or_else(|| Self::missing_field("x224_connection_pdu"))?;
                let certs = self
                    .0
                    .server_cert_chain
                    .as_ref()
                    .ok_or_else(|| Self::missing_field("server_cert_chain"))?;

                let certs: Vec<Vec<u8>> = certs.iter().map(|cert| cert.as_bytes().to_vec()).collect();
                Ok(Box::new(CertificateChainIterator { certs, index: 0 }))
            } else {
                Err(GenericError(anyhow::anyhow!(
                    "RDCleanPath variant does not contain certificate chain"
                ))
                .into())
            }
        }

        /// Gets the server address string (for Response variant)
        pub fn get_server_addr<'a>(&'a self, writeable: &'a mut DiplomatWriteable) {
            if self.0.server_addr.is_some()
                && self.0.server_cert_chain.is_some()
                && self.0.x224_connection_pdu.is_some()
            {
                if let Some(server_addr) = &self.0.server_addr {
                    let _ = write!(writeable, "{server_addr}");
                }
            }
        }

        /// Gets error message (for GeneralError variant)
        pub fn get_error_message<'a>(&'a self, writeable: &'a mut DiplomatWriteable) {
            if let Ok(err) = self.general_error() {
                let _ = write!(writeable, "{err}");
            }
        }

        /// Gets the error code (for GeneralError variant)
        pub fn get_error_code(&self) -> Result<u16, Box<IronRdpError>> {
            let err = self.general_error()?;
            Ok(err.error_code)
        }

        /// Gets the HTTP status code if present (for GeneralError variant)
        /// Returns error if not present or not a GeneralError variant
        pub fn get_http_status_code(&self) -> Result<u16, Box<IronRdpError>> {
            let err = self.general_error()?;

            err.http_status_code
                .ok_or_else(|| GenericError(anyhow::anyhow!("HTTP status code not present")).into())
        }

        fn missing_field(field: &'static str) -> Box<IronRdpError> {
            GenericError(anyhow::anyhow!("RDCleanPath is missing {field} field")).into()
        }

        fn general_error(&self) -> Result<&ironrdp_rdcleanpath::RDCleanPathErr, Box<IronRdpError>> {
            let error = self.0.error.as_ref().ok_or_else(|| Self::missing_field("error"))?;

            if error.error_code == ironrdp_rdcleanpath::NEGOTIATION_ERROR_CODE {
                Err(GenericError(anyhow::anyhow!("not a GeneralError variant")).into())
            } else {
                Ok(error)
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
