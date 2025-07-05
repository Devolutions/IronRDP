#[derive(Clone, Copy, Debug)]
struct RDCleanPathHint;

const RDCLEANPATH_HINT: RDCleanPathHint = RDCleanPathHint;

impl ironrdp::pdu::PduHint for RDCleanPathHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp::core::DecodeResult<Option<(bool, usize)>> {
        match ironrdp::rdclean_path::RDCleanPathPdu::detect(bytes) {
            ironrdp::rdclean_path::DetectionResult::Detected { total_length, .. } => Ok(Some((true, total_length))),
            ironrdp::rdclean_path::DetectionResult::NotEnoughBytes => Ok(None),
            ironrdp::rdclean_path::DetectionResult::Failed => Err(ironrdp::core::other_err!(
                "RDCleanPathHint",
                "detection failed (invalid PDU)"
            )),
        }
    }
}

#[diplomat::bridge]
pub mod ffi {
    use core::fmt::Write;
    use diplomat_runtime::DiplomatWriteable;
    use ironrdp::rdclean_path::der::asn1::OctetString;

    use crate::error::ffi::{IronRdpError, IronRdpErrorKind};
    use crate::error::ValueConsumedError;
    use crate::utils::ffi::{OptionalString, VecU8, VecVecU8};

    #[diplomat::opaque]
    pub struct RdCleanPathPdu(pub Option<ironrdp::rdclean_path::RDCleanPathPdu>);

    impl RdCleanPathPdu {
        pub fn new_request(
            x224_pdu: &VecU8,
            destination: &str,
            proxy_auth: &str,
            pcb: &OptionalString,
        ) -> Result<Box<RdCleanPathPdu>, Box<IronRdpError>> {
            let x224_pdu = &x224_pdu.0;
            let destination = destination.to_owned();
            let proxy_auth = proxy_auth.to_owned();

            let cleanpath_pdu = ironrdp::rdclean_path::RDCleanPathPdu::new_request(
                x224_pdu.to_owned(),
                destination,
                proxy_auth,
                pcb.into(),
            )
            .map_err(|_| IronRdpErrorKind::EncodeError)?;

            Ok(Box::new(RdCleanPathPdu(Some(cleanpath_pdu))))
        }

        pub fn to_der(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let Some(pdu) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("RdCleanPathPdu").into());
            };

            let der = pdu.to_der().map_err(|_| IronRdpErrorKind::EncodeError)?;
            Ok(Box::new(VecU8(der)))
        }

        pub fn get_hint<'a>() -> Box<crate::connector::ffi::PduHint<'a>> {
            Box::new(crate::connector::ffi::PduHint(&super::RDCLEANPATH_HINT))
        }

        pub fn from_der(der: &[u8]) -> Result<Box<RdCleanPathPdu>, Box<IronRdpError>> {
            let pdu =
                ironrdp::rdclean_path::RDCleanPathPdu::from_der(der).map_err(|_| IronRdpErrorKind::DecodeError)?;
            Ok(Box::new(RdCleanPathPdu(Some(pdu))))
        }

        pub fn into_enum(&mut self) -> Result<Box<RdCleanPath>, Box<IronRdpError>> {
            let Some(pdu) = self.0.take() else {
                return Err(ValueConsumedError::for_item("RdCleanPathPdu").into());
            };

            let rdclean_path = pdu
                .into_enum()
                .map(|rd_clean_path| Box::new(RdCleanPath(Some(rd_clean_path))))
                .map_err(|_| IronRdpErrorKind::EncodeError)?;

            Ok(rdclean_path)
        }
    }

    #[diplomat::opaque]
    pub struct RdCleanPath(pub Option<ironrdp::rdclean_path::RDCleanPath>);

    #[diplomat::opaque]
    pub struct RdCleanPathResponse {
        x224_connection_response: OctetString,
        server_cert_chain: Vec<OctetString>,
        server_addr: String,
    }

    impl RdCleanPathResponse {
        pub fn get_x224_connection_response(&self) -> Box<VecU8> {
            VecU8::from_bytes(self.x224_connection_response.as_bytes())
        }

        pub fn get_server_cert_chain(&self) -> Box<VecVecU8> {
            let vecs = self
                .server_cert_chain
                .iter()
                .map(|cert| cert.as_bytes().to_vec())
                .collect::<Vec<_>>();

            Box::new(VecVecU8(vecs))
        }

        pub fn get_server_addr(&self, server_addr: &mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            write!(server_addr, "{}", self.server_addr).map_err(|_| IronRdpErrorKind::IO)?;

            Ok(())
        }
    }

    pub enum RdCleanPathType {
        Request,
        Response,
        Error,
    }

    impl RdCleanPath {
        pub fn get_type(&self) -> Result<RdCleanPathType, Box<IronRdpError>> {
            let value = self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("RdCleanPath"))?;

            Ok(match value {
                ironrdp::rdclean_path::RDCleanPath::Request { .. } => RdCleanPathType::Request,
                ironrdp::rdclean_path::RDCleanPath::Response { .. } => RdCleanPathType::Response,
                ironrdp::rdclean_path::RDCleanPath::Err(_) => RdCleanPathType::Error,
            })
        }

        pub fn to_response(&mut self) -> Result<Box<RdCleanPathResponse>, Box<IronRdpError>> {
            let value = self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("RdCleanPath"))?;

            match value {
                ironrdp::rdclean_path::RDCleanPath::Response {
                    x224_connection_response,
                    server_cert_chain,
                    server_addr,
                } => Ok(Box::new(RdCleanPathResponse {
                    x224_connection_response,
                    server_cert_chain,
                    server_addr,
                })),
                _ => Err(IronRdpErrorKind::IncorrectEnumType.into()),
            }
        }
    }
}
