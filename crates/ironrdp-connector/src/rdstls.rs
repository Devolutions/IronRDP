//! RDSTLS authentication of a redirected connection, run over TLS in place of CredSSP.
//!
//! See [\[MS-RDPBCGR\] 2.2.17 RDSTLS PDUs] and [\[MS-RDPBCGR\] 5.4.5.3 RDSTLS Security].
//!
//! [\[MS-RDPBCGR\] 2.2.17 RDSTLS PDUs]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/2aa353a2-2e46-4bb7-84ef-9354eb07a25f
//! [\[MS-RDPBCGR\] 5.4.5.3 RDSTLS Security]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/83d1186d-cab6-4ad8-8c5f-203f95e192aa

use ironrdp_core::DecodeResult;
use ironrdp_pdu::PduHint;

/// Frames one RDSTLS PDU of a known fixed size.
///
/// RDSTLS PDUs carry no length field, and the server PDUs the client waits for have a fixed size.
#[derive(Debug)]
pub(crate) struct RdstlsFixedSizeHint(pub(crate) usize);

/// Size of the RDSTLS Capabilities PDU.
pub(crate) const CAPABILITIES_HINT: RdstlsFixedSizeHint = RdstlsFixedSizeHint(
    2 /* Version */ + 2 /* PduType */ + 2 /* DataType */ + 2, /* SupportedVersions */
);

/// Size of the RDSTLS Authentication Response PDU.
pub(crate) const AUTHENTICATION_RESPONSE_HINT: RdstlsFixedSizeHint = RdstlsFixedSizeHint(
    2 /* Version */ + 2 /* PduType */ + 2 /* DataType */ + 4, /* ResultCode */
);

impl PduHint for RdstlsFixedSizeHint {
    fn find_size(&self, _bytes: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        Ok(Some((true, self.0)))
    }
}
