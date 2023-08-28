//! Cliboard static virtual channel(SVC) implementation.
//! This library includes:
//! - Cliboard SVC PDUs parsing
//! - Clipboard SVC processing (TODO)

pub mod pdu;

use ironrdp_pdu::{decode, gcc::ChannelName, PduResult};
use ironrdp_svc::{impl_as_any, ChannelFlags, CompressionCondition, StaticVirtualChannel, SvcMessage};
use pdu::{
    Capabilities, ClientTemporaryDirectory, ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FormatListResponse,
};

use crate::pdu::FormatList;
use thiserror::Error;
use tracing::{error, info};

#[derive(Debug, Error)]
enum ClipboardError {
    #[error("Received clipboard PDU is not implemented")]
    UnimplementedPdu { pdu: &'static str },

    #[error("Sent format list was rejected")]
    FormatListRejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliprdrState {
    Initialization,
    Ready,
    Failed,
}

/// CLIPRDR static virtual channel client endpoint implementation
#[derive(Debug)]
pub struct Cliprdr {
    capabilities: Capabilities,
    temporary_directory: String,
    state: CliprdrState,
}

impl_as_any!(Cliprdr);

impl Cliprdr {
    const CHANNEL_NAME: ChannelName = ChannelName::from_static(b"cliprdr\0");

    fn build_local_format_list(&self) -> PduResult<FormatList<'static>> {
        let formats = vec![
            ClipboardFormat::new(ClipboardFormatId::CF_TEXT),
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ];

        FormatList::new_unicode(
            &formats,
            self.capabilities
                .flags()
                .contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES),
        )
    }

    fn handle_error_transition(&mut self, err: ClipboardError) -> PduResult<Vec<SvcMessage>> {
        // Failure of clipboard is not an critical error, but we should properly report it
        // and transition channel to failed state.
        self.state = CliprdrState::Failed;
        error!("CLIPRDR(clipboard) failed: {err}");

        Ok(vec![])
    }

    fn handle_server_capabilities(&mut self, server_capabilities: Capabilities) -> PduResult<Vec<SvcMessage>> {
        self.capabilities.downgrade(&server_capabilities);

        // Do not send anything, wait for monitor ready pdu
        Ok(vec![])
    }

    fn handle_monitor_ready(&mut self) -> PduResult<Vec<SvcMessage>> {
        let response = [
            ClipboardPdu::Capabilities(self.capabilities.clone()),
            ClipboardPdu::TemporaryDirectory(ClientTemporaryDirectory::new(self.temporary_directory.clone())?),
            ClipboardPdu::FormatList(self.build_local_format_list()?),
        ]
        .into_iter()
        .map(into_cliprdr_message)
        .collect();

        Ok(response)
    }

    fn handle_format_list_response(&mut self, response: FormatListResponse) -> PduResult<Vec<SvcMessage>> {
        match response {
            FormatListResponse::Ok => {
                info!("CLIPRDR(clipboard) virtual channel has been initialized");
                self.state = CliprdrState::Ready;
            }
            FormatListResponse::Fail => {
                return self.handle_error_transition(ClipboardError::FormatListRejected);
            }
        }

        Ok(vec![])
    }
}

impl Default for Cliprdr {
    fn default() -> Self {
        Self {
            state: CliprdrState::Initialization,
            capabilities: Capabilities::new(
                ClipboardProtocolVersion::V2,
                ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
            ),
            temporary_directory: ".cliprdr".to_string(),
        }
    }
}

impl StaticVirtualChannel for Cliprdr {
    fn channel_name(&self) -> ChannelName {
        Self::CHANNEL_NAME
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode::<ClipboardPdu>(payload)?;

        if self.state == CliprdrState::Failed {
            error!("Attempted to process clipboard static virtual channel in failed state");
            return Ok(vec![]);
        }

        match pdu {
            ClipboardPdu::Capabilities(caps) => self.handle_server_capabilities(caps),
            ClipboardPdu::FormatListResponse(response) => self.handle_format_list_response(response),
            ClipboardPdu::MonitorReady => self.handle_monitor_ready(),
            _ => self.handle_error_transition(ClipboardError::UnimplementedPdu {
                pdu: pdu.message_name(),
            }),
        }
    }

    fn compression_condition(&self) -> ironrdp_svc::CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }
}

fn into_cliprdr_message(pdu: ClipboardPdu<'static>) -> SvcMessage {
    // Adding [`CHANNEL_FLAG_SHOW_PROTOCOL`] is a must for clipboard svc messages
    SvcMessage::from(pdu).with_flags(ChannelFlags::SHOW_PROTOCOL)
}
