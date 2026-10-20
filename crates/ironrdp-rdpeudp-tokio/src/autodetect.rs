//! Auto-detect messages carried in tunnel sub-headers.
//!
//! On a sideband channel in use, the server sends its auto-detect requests in the sub-headers of
//! Tunnel Data PDUs, and the client answers there ([MS-RDPBCGR] 3.2.5.14). Each sub-header is the
//! auto-detect structure itself: SubHeaderLength and SubHeaderType are its headerLength and
//! headerTypeId ([MS-RDPEMT] 2.2.1.1.1).
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/16ffa852-8aa7-481c-99a0-36c1a9a198f6
//! [MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/4f538fd7-3aca-4e7d-a213-13eb5f95c1ad

use ironrdp_core::{Decode, Encode};
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};
use tracing::debug;

use crate::transport::TunnelMessage;

impl TunnelMessage {
    /// Decodes the auto-detect requests among this message's sub-headers.
    ///
    /// Sub-headers of other types and requests that do not decode are skipped. RTT requests
    /// are returned along with the bandwidth requests MS-RDPEMT lists, because Windows sends
    /// them on the tunnel too, although MS-RDPEMT does not specify that encapsulation.
    pub fn auto_detect_requests(&self) -> Vec<AutoDetectRequest> {
        self.sub_headers
            .iter()
            .filter(|sub_header| sub_header.sub_header_type == SubHeaderType::AutoDetectRequest)
            .filter_map(|sub_header| match redecode(sub_header) {
                Ok(request) => Some(request),
                Err(error) => {
                    debug!(%error, data = ?sub_header.data, "Ignoring an undecodable auto-detect request on the tunnel");
                    None
                }
            })
            .collect()
    }

    /// Builds a message with no higher-layer data that carries `responses` as sub-headers.
    ///
    /// A response that cannot be encoded is skipped. Returns `None` when nothing is left to
    /// send.
    pub fn from_auto_detect_responses(responses: &[AutoDetectResponse]) -> Option<Self> {
        let sub_headers: Vec<TunnelSubHeader> = responses
            .iter()
            .filter_map(|response| match redecode(response) {
                Ok(sub_header) => Some(sub_header),
                Err(error) => {
                    debug!(%error, ?response, "Could not encode an auto-detect response for the tunnel");
                    None
                }
            })
            .collect();
        (!sub_headers.is_empty()).then_some(Self {
            sub_headers,
            data: Vec::new(),
        })
    }
}

/// Re-reads one wire structure as another with the same bytes.
fn redecode<T: for<'de> Decode<'de>>(value: &dyn Encode) -> Result<T, String> {
    let bytes = ironrdp_core::encode_vec(value).map_err(|error| error.to_string())?;
    ironrdp_core::decode(&bytes).map_err(|error| error.to_string())
}
