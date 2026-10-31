//! Client processor for the MS-RDPET dynamic virtual channel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::TelemetryPdu;

/// Client processor for `Microsoft::Windows::RDS::Telemetry`.
///
/// The channel carries a single client-to-server report per connection ([MS-RDPET 1.3]).
/// The caller measures the milestones and sends the report with [`Self::send_report`] once the channel is open.
///
/// [MS-RDPET 1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/39bb54fd-3221-477c-9955-6e603001ef63
#[derive(Debug, Default)]
pub struct TelemetryClient {
    channel_id: Option<u32>,
}

impl TelemetryClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encodes `report` for the open channel.
    ///
    /// Sending the report is optional ([MS-RDPET 3.2.5.1]).
    ///
    /// [MS-RDPET 3.2.5.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/e5270011-b8e2-40b3-92e1-c7460af45edc
    pub fn send_report(&self, report: TelemetryPdu) -> PduResult<Vec<SvcMessage>> {
        let channel_id = self
            .channel_id
            .ok_or_else(|| pdu_other_err!("TelemetryClient::send_report", "telemetry channel is not open"))?;

        encode_dvc_messages(channel_id, vec![Box::new(report)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))
    }
}

impl_as_any!(TelemetryClient);

impl DvcProcessor for TelemetryClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        debug!(channel_id, "Telemetry channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // MS-RDPET defines no server-to-client message.
        warn!(size = payload.len(), "Ignoring unexpected telemetry channel data");
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        debug!("Telemetry channel closed");
    }
}

impl DvcClientProcessor for TelemetryClient {}
