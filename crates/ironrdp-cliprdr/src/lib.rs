//! Cliboard static virtual channel(SVC) implementation.
//! This library includes:
//! - Cliboard SVC PDUs parsing
//! - Clipboard SVC processing

pub mod backend;
pub mod pdu;

use ironrdp_pdu::{decode, gcc::ChannelName, PduResult};
use ironrdp_svc::{
    impl_as_any, ChannelFlags, ChunkProcessor, CompressionCondition, StaticVirtualChannel, SvcMessage, SvcRequest,
};
use pdu::{
    Capabilities, ClientTemporaryDirectory, ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileContentsResponse, FormatDataRequest, FormatDataResponse,
    FormatListResponse,
};

use crate::pdu::FormatList;
use backend::CliprdrBackend;
use thiserror::Error;
use tracing::{error, info};

pub type CliprdrSvcRequest = SvcRequest<Cliprdr>;

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

pub struct FileStreamSettings {
    /// Path to local temporary directory where files will be stored
    pub temporary_directory: String,
    /// Enable source file path information in
    pub enable_file_paths: bool,
    pub enable_data_lock: bool,
    pub enable_huge_files: bool,
}

/// CLIPRDR static virtual channel client endpoint implementation
#[derive(Debug)]
pub struct Cliprdr {
    backend: Box<dyn CliprdrBackend>,
    capabilities: Capabilities,
    state: CliprdrState,
    preprocessor: ChunkProcessor,
}

impl_as_any!(Cliprdr);

impl Cliprdr {
    const CHANNEL_NAME: ChannelName = ChannelName::from_static(b"cliprdr\0");

    pub fn new(backend: Box<dyn CliprdrBackend>) -> Self {
        // This CLIPRDR implementation supports long format names by default
        let flags = ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES | backend.client_capabilities();

        Self {
            backend,
            state: CliprdrState::Initialization,
            capabilities: Capabilities::new(ClipboardProtocolVersion::V2, flags),
            preprocessor: ChunkProcessor::new(),
        }
    }

    fn are_long_format_names_enabled(&self) -> bool {
        self.capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES)
    }

    fn build_format_list(&self, formats: &[ClipboardFormat]) -> PduResult<FormatList<'static>> {
        FormatList::new_unicode(formats, self.are_long_format_names_enabled())
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
        self.backend
            .on_receive_downgraded_capabilities(self.capabilities.flags());

        // Do not send anything, wait for monitor ready pdu
        Ok(vec![])
    }

    fn handle_monitor_ready(&mut self) -> PduResult<Vec<SvcMessage>> {
        // Request client to sent list of initially available formats and wait for the backend
        // response.
        self.backend.on_request_format_list();
        Ok(vec![])
    }

    fn handle_format_list_response(&mut self, response: FormatListResponse) -> PduResult<Vec<SvcMessage>> {
        match response {
            FormatListResponse::Ok => {
                if self.state == CliprdrState::Initialization {
                    info!("CLIPRDR(clipboard) virtual channel has been initialized");
                    self.state = CliprdrState::Ready;
                } else {
                    info!("CLIPRDR(clipboard) Remote has received format list successfully");
                }
            }
            FormatListResponse::Fail => {
                return self.handle_error_transition(ClipboardError::FormatListRejected);
            }
        }

        Ok(vec![])
    }

    fn handle_format_list(&mut self, format_list: FormatList) -> PduResult<Vec<SvcMessage>> {
        let formats = format_list.get_formats(self.are_long_format_names_enabled())?;
        self.backend.on_remote_copy(&formats);

        let pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);

        Ok(vec![into_cliprdr_message(pdu)])
    }

    /// Should be called by the clipboard implementation when it receives data from the OS clipboard
    /// and is ready to sent it to the server. This should happen after `CLIPRDR` channel triggered
    /// `on_format_data_request` via `CliprdrBackend`.
    ///
    /// If data is not available anymore, an error response should be sent instead.
    pub fn sumbit_format_data(&self, response: FormatDataResponse<'static>) -> PduResult<CliprdrSvcRequest> {
        if self.state != CliprdrState::Ready {
            return Ok(vec![].into());
        }

        let pdu = ClipboardPdu::FormatDataResponse(response);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// Should be called by the clipboard implementation when file data is ready to sent it to the
    /// server. This should happen after `CLIPRDR` channel triggered `on_file_contents_request` via
    /// `CliprdrBackend`.
    ///
    /// If data is not available anymore, an error response should be sent instead.
    pub fn sumbit_file_contents(&self, response: FileContentsResponse<'static>) -> PduResult<CliprdrSvcRequest> {
        if self.state != CliprdrState::Ready {
            return Ok(vec![].into());
        }

        let pdu = ClipboardPdu::FileContentsResponse(response);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// Start processing of `CLIPRDR` copy command. Should be called by `ironrdp` client
    /// implementation when user performs OS-specific copy command (e.g. `Ctrl+C` shortcut on
    /// keyboard)
    pub fn initiate_copy(&self, available_formats: &[ClipboardFormat]) -> PduResult<CliprdrSvcRequest> {
        // During initialization state, first copy action is syntetic and should be sent along with
        // capabilities and temporary directory PDUs.
        if self.state == CliprdrState::Initialization {
            let temporary_directory = self.backend.temporary_directory();

            let response = [
                ClipboardPdu::Capabilities(self.capabilities.clone()),
                ClipboardPdu::TemporaryDirectory(ClientTemporaryDirectory::new(temporary_directory)?),
                ClipboardPdu::FormatList(self.build_format_list(available_formats).unwrap()),
            ]
            .into_iter()
            .map(into_cliprdr_message)
            .collect::<Vec<_>>();

            return Ok(response.into());
        }

        if self.state != CliprdrState::Ready {
            return Ok(vec![].into());
        }

        // When user initiates copy, we should send format list to server, and expect to
        // receive response with `FormatListResponse::Ok` status.
        let pdu = ClipboardPdu::FormatList(self.build_format_list(available_formats)?);
        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// Start processing of `CLIPRDR` paste command. Should be called by `ironrdp` client
    /// implementation when user performs OS-specific paste command (e.g. `Ctrl+V` shortcut on
    /// keyboard)
    pub fn initiate_paste(&self, requested_format: ClipboardFormatId) -> PduResult<CliprdrSvcRequest> {
        if self.state != CliprdrState::Ready {
            return Ok(vec![].into());
        }

        // When user initiates paste, we should send format data request to server, and expect to
        // receive response with contents via `FormatDataResponse` PDU.
        let pdu = ClipboardPdu::FormatDataRequest(FormatDataRequest {
            format: requested_format,
        });

        Ok(vec![into_cliprdr_message(pdu)].into())
    }
}

impl StaticVirtualChannel for Cliprdr {
    fn channel_name(&self) -> ChannelName {
        Self::CHANNEL_NAME
    }

    fn preprocessor(&self) -> &ChunkProcessor {
        &self.preprocessor
    }

    fn preprocessor_mut(&mut self) -> &mut ChunkProcessor {
        &mut self.preprocessor
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu: ClipboardPdu = decode::<ClipboardPdu>(payload)?;

        if self.state == CliprdrState::Failed {
            error!("Attempted to process clipboard static virtual channel in failed state");
            return Ok(vec![]);
        }

        match pdu {
            ClipboardPdu::Capabilities(caps) => self.handle_server_capabilities(caps),
            ClipboardPdu::FormatList(format_list) => self.handle_format_list(format_list),
            ClipboardPdu::FormatListResponse(response) => self.handle_format_list_response(response),
            ClipboardPdu::MonitorReady => self.handle_monitor_ready(),
            ClipboardPdu::LockData(id) => {
                self.backend.on_lock(id);
                Ok(vec![])
            }
            ClipboardPdu::UnlockData(id) => {
                self.backend.on_unlock(id);
                Ok(vec![])
            }
            ClipboardPdu::FormatDataRequest(request) => {
                self.backend.on_format_data_request(request);

                // NOTE: An actual data should be sent later via `submit_format_data` method,
                // therefore we do not send anything immediately.
                Ok(vec![])
            }
            ClipboardPdu::FormatDataResponse(response) => {
                self.backend.on_format_data_response(response);
                Ok(vec![])
            }
            ClipboardPdu::FileContentsRequest(request) => {
                self.backend.on_file_contents_request(request);
                Ok(vec![])
            }
            ClipboardPdu::FileContentsResponse(response) => {
                self.backend.on_file_contents_response(response);
                Ok(vec![])
            }
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
