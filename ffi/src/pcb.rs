#[diplomat::bridge]
pub mod ffi {
    use crate::error::ffi::IronRdpError;
    use crate::utils::ffi::VecU8;

    /// Preconnection PDU (MS-RDPEPS), sent on the transport before the X.224 Connection Request
    /// so a listener such as a gateway or a Hyper-V host can select the RDP source.
    #[diplomat::opaque]
    pub struct PreconnectionBlob(pub ironrdp::pdu::pcb::PreconnectionBlob);

    impl PreconnectionBlob {
        /// Creates a V1 PDU, which carries only the id.
        pub fn new_v1(id: u32) -> Box<PreconnectionBlob> {
            Box::new(PreconnectionBlob(ironrdp::pdu::pcb::PreconnectionBlob {
                version: ironrdp::pdu::pcb::PcbVersion::V1,
                id,
                v2_payload: None,
            }))
        }

        /// Creates a V2 PDU, which carries the id and a string opaque to the protocol,
        /// such as a gateway association token or a Hyper-V VM id.
        pub fn new_v2(id: u32, payload: &str) -> Box<PreconnectionBlob> {
            Box::new(PreconnectionBlob(ironrdp::pdu::pcb::PreconnectionBlob {
                version: ironrdp::pdu::pcb::PcbVersion::V2,
                id,
                v2_payload: Some(payload.to_owned()),
            }))
        }

        /// Encodes the PDU as the bytes to write on the transport.
        pub fn encode(&self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let bytes = ironrdp_core::encode_vec(&self.0)?;
            Ok(Box::new(VecU8(bytes)))
        }
    }
}
