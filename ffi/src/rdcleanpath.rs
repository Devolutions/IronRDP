#[derive(Clone, Copy, Debug)]
struct RDCleanPathHint;

const RDCLEANPATH_HINT: RDCleanPathHint = RDCleanPathHint;

impl ironrdp::pdu::PduHint for RDCleanPathHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp::core::DecodeResult<Option<(bool, usize)>> {
        match ironrdp_rdcleanpath::RDCleanPathPdu::detect(bytes) {
            ironrdp_rdcleanpath::DetectionResult::Detected { total_length, .. } => Ok(Some((true, total_length))),
            ironrdp_rdcleanpath::DetectionResult::NotEnoughBytes => Ok(None),
            ironrdp_rdcleanpath::DetectionResult::Failed => Err(ironrdp::core::other_err!(
                "RDCleanPathHint",
                "detection failed (invalid PDU)"
            )),
        }
    }
}

#[diplomat::bridge]
pub mod ffi {

    use crate::error::ffi::{IronRdpError, IronRdpErrorKind};
    use crate::error::ValueConsumedError;
    use crate::utils::ffi::{ServerCertChain, VecU8};
    use core::fmt::Write;

    #[diplomat::opaque]
    pub struct RdCleanPathPdu(pub ironrdp_rdcleanpath::RDCleanPathPdu);

    #[diplomat::opaque]
    pub struct RdCleanPathRequestBuilder {
        x224_pdu: Option<Vec<u8>>,
        destination: Option<String>,
        proxy_auth: Option<String>,
        pcb: Option<String>,
    }

    impl RdCleanPathRequestBuilder {
        pub fn new() -> Box<RdCleanPathRequestBuilder> {
            Box::new(RdCleanPathRequestBuilder {
                x224_pdu: None,
                destination: None,
                proxy_auth: None,
                pcb: None,
            })
        }

        pub fn with_x224_pdu(&mut self, x224_pdu: &VecU8) {
            self.x224_pdu = Some(x224_pdu.0.clone());
        }

        pub fn with_destination(&mut self, destination: &str) {
            self.destination = Some(destination.to_owned());
        }

        pub fn with_proxy_auth(&mut self, proxy_auth: &str) {
            self.proxy_auth = Some(proxy_auth.to_owned());
        }

        pub fn with_pcb(&mut self, pcb: &str) {
            self.pcb = Some(pcb.to_owned());
        }

        pub fn build(&self) -> Result<Box<RdCleanPathPdu>, Box<IronRdpError>> {
            let RdCleanPathRequestBuilder {
                x224_pdu,
                destination,
                proxy_auth,
                pcb,
            } = self;

            let request = ironrdp_rdcleanpath::RDCleanPathPdu::new_request(
                x224_pdu.to_owned().ok_or(IronRdpErrorKind::MissingRequiredField)?,
                destination.to_owned().ok_or(IronRdpErrorKind::MissingRequiredField)?,
                proxy_auth.to_owned().ok_or(IronRdpErrorKind::MissingRequiredField)?,
                pcb.to_owned(),
            )
            .map_err(|_| IronRdpErrorKind::EncodeError)?;

            Ok(Box::new(RdCleanPathPdu(request)))
        }
    }

    impl RdCleanPathPdu {
        pub fn to_der(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let der = self.0.to_der().map_err(|_| IronRdpErrorKind::EncodeError)?;
            Ok(Box::new(VecU8(der)))
        }

        pub fn get_hint<'a>() -> Box<crate::connector::ffi::PduHint<'a>> {
            Box::new(crate::connector::ffi::PduHint(&super::RDCLEANPATH_HINT))
        }

        pub fn from_der(der: &[u8]) -> Result<Box<RdCleanPathPdu>, Box<IronRdpError>> {
            let pdu = ironrdp_rdcleanpath::RDCleanPathPdu::from_der(der).map_err(|_| IronRdpErrorKind::DecodeError)?;
            Ok(Box::new(RdCleanPathPdu(pdu)))
        }

        pub fn get_x224_connection_pdu(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let Some(x224_pdu_response) = self.0.x224_connection_pdu.as_ref() else {
                return Err(ValueConsumedError::for_item("RdCleanPathPdu").into());
            };

            let result = x224_pdu_response.as_bytes().to_vec();

            Ok(Box::new(VecU8(result)))
        }

        pub fn get_server_cert_chain(&self) -> Result<Box<ServerCertChain>, Box<IronRdpError>> {
            let Some(server_cert_chain) = self.0.server_cert_chain.as_ref() else {
                return Err(ValueConsumedError::for_item("ServerCertChain").into());
            };

            let vecs = server_cert_chain
                .iter()
                .map(|cert| cert.as_bytes().to_vec())
                .collect::<Vec<_>>();

            Ok(Box::new(ServerCertChain(vecs)))
        }

        pub fn get_server_addr(
            &self,
            server_addr: &mut diplomat_runtime::DiplomatWriteable,
        ) -> Result<(), Box<IronRdpError>> {
            let Some(server_addr_str) = self.0.server_addr.as_ref() else {
                return Err(ValueConsumedError::for_item("server_addr").into());
            };

            write!(server_addr, "{server_addr_str}").map_err(|_| IronRdpErrorKind::IO)?;

            Ok(())
        }
    }
}
