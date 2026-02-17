//! ECHO virtual channel extension PDUs [MS-RDPEECO][1] implementation.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeeco/5f4f5b76-14f2-4807-bf8c-10fcb7f7f41c

use ironrdp_core::{ensure_size, Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};
use ironrdp_dvc::DvcEncode;

/// 2.2.1 ECHO_REQUEST_PDU
///
/// [2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeeco/bf2c9ef3-2f8b-40c2-b27d-ce9df72976f2
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoRequestPdu {
    payload: Vec<u8>,
}

impl EchoRequestPdu {
    const NAME: &'static str = "ECHO_REQUEST_PDU";

    pub fn new(payload: Vec<u8>) -> Self {
        Self { payload }
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

impl Encode for EchoRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.payload.len());
        dst.write_slice(&self.payload);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.payload.len()
    }
}

impl<'de> Decode<'de> for EchoRequestPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let payload = src.read_remaining().to_vec();
        Ok(Self { payload })
    }
}

impl DvcEncode for EchoRequestPdu {}

/// 2.2.2 ECHO_RESPONSE_PDU
///
/// [2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeeco/f95db8eb-fffd-4b76-9f8f-60322ea2dd2d
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoResponsePdu {
    payload: Vec<u8>,
}

impl EchoResponsePdu {
    const NAME: &'static str = "ECHO_RESPONSE_PDU";

    pub fn new(payload: Vec<u8>) -> Self {
        Self { payload }
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

impl Encode for EchoResponsePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.payload.len());
        dst.write_slice(&self.payload);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.payload.len()
    }
}

impl<'de> Decode<'de> for EchoResponsePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let payload = src.read_remaining().to_vec();
        Ok(Self { payload })
    }
}

impl DvcEncode for EchoResponsePdu {}
