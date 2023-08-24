//! Cliboard static virtual channel(SVC) implementation.
//! This library includes:
//! - Cliboard SVC PDUs parsing
//! - Clipboard SVC processing (TODO)

pub mod pdu;

use ironrdp_pdu::{decode, gcc::ChannelName, PduEncode, PduResult};
use ironrdp_svc::{impl_as_any, ChannelPDUFlags, CompressionCondition, StaticVirtualChannel, SvcMessage};
use pdu::{
    Capabilities, ClientTemporaryDirectory, ClipboardFormat, ClipboardGeneralCapabilityFlags, ClipboardPdu,
    ClipboardProtocolVersion, FormatListResponse,
};

use crate::pdu::FormatList;
use thiserror::Error;
use tracing::{error, info};

pub mod clipboard_format {
    pub const CF_TEXT: u32 = 1;
    pub const CF_UNICODE: u32 = 13;
}

#[derive(Debug, Error)]
enum ClipboardError {
    #[error("Expected `CLIPRDR_CAPABILITIES` or `CLIPRDR_MONITOR_READY` pdu")]
    UnexpectedInitialPdu { received_pdu: &'static str },

    #[error("Expected `CLIPRDR_MONITOR_READY` pdu")]
    UnexpectedMonitorReadyPdu { received_pdu: &'static str },

    #[error("Expected `CLIPRDR_FORMAT_LIST_RESPONSE` pdu")]
    UnexpectedFormatListResponsePdu { received_pdu: &'static str },

    #[error("Sent format list was rejected")]
    FormatListRejected,
}

#[derive(Debug)]
enum CliprdrInitializationState {
    WaitForServer,
    ServerCapabilitiesReceived,
    WaitForServerFormatListResponse,
}

#[derive(Debug)]
enum CliprdrState {
    Initialization(CliprdrInitializationState),
    Ready,
    Failed,
}

/// CLIPRDR static virtual channel client endpoint implementation
#[derive(Debug)]
pub struct Cliprdr {
    state: CliprdrState,
    capabilities: Capabilities,
    temporary_directory: String,
}

impl_as_any!(Cliprdr);

impl Cliprdr {
    const CHANNEL_NAME: ChannelName = ChannelName::from_static(b"cliprdr\0");

    fn build_local_format_list(&self) -> PduResult<FormatList<'static>> {
        let formats = vec![
            ClipboardFormat::new_standard(clipboard_format::CF_TEXT),
            ClipboardFormat::new_standard(clipboard_format::CF_UNICODE),
        ];

        FormatList::new_unicode(
            &formats,
            self.capabilities
                .flags()
                .contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES),
        )
    }

    fn handle_error_tranistion(&mut self, err: ClipboardError) -> PduResult<Vec<SvcMessage>> {
        // Failure of clipboard is not an critical error, but we should properly report it
        // and transition channel to failed state.
        self.state = CliprdrState::Failed;
        error!("CLIPRDR(clipboard) failed: {err}");

        Ok(vec![])
    }

    fn handle_server_capabilities(&mut self, server_capabilities: Capabilities) -> PduResult<Vec<SvcMessage>> {
        self.capabilities.downgrade(&server_capabilities);
        self.state = CliprdrState::Initialization(CliprdrInitializationState::ServerCapabilitiesReceived);

        // Do not send anything, wait for monitor ready pdu
        Ok(vec![])
    }

    fn handle_monitor_ready(&mut self) -> PduResult<Vec<SvcMessage>> {
        let response = [
            ClipboardPdu::Capabilites(self.capabilities.clone()),
            ClipboardPdu::TemporaryDirectory(ClientTemporaryDirectory::new(self.temporary_directory.clone())?),
            ClipboardPdu::FormatList(self.build_local_format_list()?),
        ]
        .into_iter()
        .map(into_cliprdr_message)
        .collect();

        self.state = CliprdrState::Initialization(CliprdrInitializationState::WaitForServerFormatListResponse);
        Ok(response)
    }

    fn handle_format_list_response(&mut self, response: FormatListResponse) -> PduResult<Vec<SvcMessage>> {
        match self.state {
            CliprdrState::Initialization(_) => match response {
                FormatListResponse::Ok => {
                    self.state = CliprdrState::Ready;
                    info!("CLIPRDR(clipboard) virtual channel has been initialized");
                }
                FormatListResponse::Fail => {
                    return self.handle_error_tranistion(ClipboardError::FormatListRejected);
                }
            },
            CliprdrState::Ready => {
                // TODO(@pacmancoder): Currently ignored as data transfer is not implemented,
                // but in the next feature development interation we should handle this correctly
            }
            CliprdrState::Failed => unreachable!(),
        }

        Ok(vec![])
    }
}

impl Default for Cliprdr {
    fn default() -> Self {
        Self {
            state: CliprdrState::Initialization(CliprdrInitializationState::WaitForServer),
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

        let messages = match &self.state {
            CliprdrState::Initialization(CliprdrInitializationState::WaitForServer) => {
                match pdu {
                    // Capabilities are optional (per [MS-RDPECLIP]), so we expect either
                    // Capabilities or MonitorReady PDU
                    ClipboardPdu::Capabilites(caps) => self.handle_server_capabilities(caps)?,
                    ClipboardPdu::MonitorReady => self.handle_monitor_ready()?,
                    _ => self.handle_error_tranistion(ClipboardError::UnexpectedInitialPdu {
                        received_pdu: pdu.name(),
                    })?,
                }
            }
            CliprdrState::Initialization(CliprdrInitializationState::ServerCapabilitiesReceived) => match pdu {
                ClipboardPdu::MonitorReady => self.handle_monitor_ready()?,
                _ => self.handle_error_tranistion(ClipboardError::UnexpectedMonitorReadyPdu {
                    received_pdu: pdu.name(),
                })?,
            },
            CliprdrState::Initialization(CliprdrInitializationState::WaitForServerFormatListResponse) => match pdu {
                ClipboardPdu::FormatListResponse(response) => self.handle_format_list_response(response)?,
                _ => self.handle_error_tranistion(ClipboardError::UnexpectedFormatListResponsePdu {
                    received_pdu: pdu.name(),
                })?,
            },
            CliprdrState::Ready => {
                // TODO(@pacmancoder): Implement data transfer logic after CLIPRDR is initialized
                vec![]
            }
            CliprdrState::Failed => {
                // Do nothing, channel is in error state.
                error!("Attempted to process clipboard static virtual channel in failed state");
                vec![]
            }
        };

        Ok(messages)
    }

    fn compression_condition(&self) -> ironrdp_svc::CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }
}

fn into_cliprdr_message(pdu: ClipboardPdu<'static>) -> SvcMessage {
    // Adding [`CHANNEL_FLAG_SHOW_PROTOCOL`] is a must for clipboard svc messages
    SvcMessage::from(pdu).with_flags(ChannelPDUFlags::CHANNEL_FLAG_SHOW_PROTOCOL)
}
