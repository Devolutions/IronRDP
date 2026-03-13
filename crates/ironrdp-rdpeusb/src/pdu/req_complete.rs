//! PDU's specific to the [Request Completion][1] interface.
//!
//! Used by the client to send the final result for a request previously sent from the server.
//! The unique interface ID for this interface is provided by the server using the
//! [`REGISTER_REQUEST_CALLBACK`] message, during the lifecycle of a USB redirection channel.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c0a146fc-20cf-4897-af27-a3c5474151ac

use alloc::vec::Vec;
use ironrdp_core::{ensure_size, Decode, Encode};
use ironrdp_pdu::utils::strict_sum;

use super::common::{HResult, ReqIdIoctl, SharedMsgHeader};

#[doc(alias = "IOCONTROL_COMPLETION")]
pub struct IoctlCompletion {
    pub header: SharedMsgHeader,
    pub req_id: ReqIdIoctl,
    pub hresult: HResult,
    pub info: u32,
    pub out_buf_bytes: u32,
    pub out_buf: Vec<u8>,
}

impl Encode for IoctlCompletion {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.header.encode(dst)?;

        dst.write_u32(self.req_id);
        dst.write_u32(self.hresult);
        dst.write_u32(self.info);
        dst.write_u32(self.out_buf_bytes);

        dst.write_slice(&self.out_buf);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "IOCONTROL_COMPLETION"
    }

    fn size(&self) -> usize {
        let header = self.header.size();

        const REQ_ID: usize = const { size_of::<ReqIdIoctl>() };
        const HRESULT: usize = const { size_of::<HResult>() };
        const INFO: usize = const { size_of::<u32>() };
        const OUT_BUF_BYTES: usize = const { size_of::<u32>() };

        let out_buf = self.out_buf.len();

        strict_sum(&[header + REQ_ID + HRESULT + INFO + OUT_BUF_BYTES + out_buf])
    }
}

impl Decode<'_> for IoctlCompletion {
    fn decode(src: &mut ironrdp_core::ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let header = SharedMsgHeader::decode(src)?;

        let fixed_bytes = size_of::<ReqIdIoctl>() + size_of::<HResult>() + size_of::<u32>() + size_of::<u32>();
        ensure_size!(in: src, size: fixed_bytes);

        let req_id = src.read_u32();
        let hresult = src.read_u32();
        let info = src.read_u32();
        let out_buf_bytes = src.read_u32();

        #[expect(clippy::as_conversions)]
        let out_buf = {
            ensure_size!(in: src, size: out_buf_bytes as usize);
            Vec::from(src.read_slice(out_buf_bytes as usize))
        };

        Ok(Self {
            header,
            req_id,
            hresult,
            info,
            out_buf_bytes,
            out_buf,
        })
    }
}

// pub struct UrbCompletion {
//     pub header: SharedMsgHeader,
//     req_id: ReqIdTsUrb,
// }
