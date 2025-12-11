#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::arithmetic_side_effects)] // FIXME: remove

pub mod backend;
pub mod pdu;

use backend::CliprdrBackend;
use ironrdp_core::{decode, AsAny, EncodeResult};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{decode_err, encode_err, PduResult};
use ironrdp_svc::{
    ChannelFlags, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor, SvcProcessorMessages,
    SvcServerProcessor,
};
use pdu::{
    Capabilities, ClientTemporaryDirectory, ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileContentsResponse, FormatDataRequest, FormatListResponse,
    OwnedFormatDataResponse,
};
use tracing::{error, info};

#[rustfmt::skip] // do not reorder
use crate::pdu::FormatList;

/// PDUs for sending to the server on the CLIPRDR channel.
pub type CliprdrSvcMessages<R> = SvcProcessorMessages<Cliprdr<R>>;

#[derive(Debug)]
enum ClipboardError {
    FormatListRejected,
}

impl core::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClipboardError::FormatListRejected => write!(f, "sent format list was rejected"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum CliprdrState {
    Initialization,
    Ready,
    Failed,
}

pub trait Role: core::fmt::Debug + Send + 'static {
    fn is_server() -> bool;
}

/// CLIPRDR static virtual channel endpoint implementation
#[derive(Debug)]
pub struct Cliprdr<R: Role> {
    backend: Box<dyn CliprdrBackend>,
    capabilities: Capabilities,
    state: CliprdrState,
    _marker: core::marker::PhantomData<R>,
}

pub type CliprdrClient = Cliprdr<Client>;
pub type CliprdrServer = Cliprdr<Server>;

impl SvcClientProcessor for CliprdrClient {}
impl SvcServerProcessor for CliprdrServer {}

impl<R: Role> AsAny for Cliprdr<R> {
    #[inline]
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    #[inline]
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

macro_rules! ready_guard {
        ($self:ident, $function:ident) => {{
            let _ = Self::$function; // ensure the function actually exists

            if $self.state != CliprdrState::Ready {
                error!(?$self.state, concat!("Attempted to initiate ", stringify!($function), " in incorrect state"));
                return Ok(Vec::new().into());
            }
        }};
    }

impl<R: Role> Cliprdr<R> {
    const CHANNEL_NAME: ChannelName = ChannelName::from_static(b"cliprdr\0");

    pub fn new(backend: Box<dyn CliprdrBackend>) -> Self {
        // This CLIPRDR implementation supports long format names by default
        let flags = ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES | backend.client_capabilities();

        Self {
            backend,
            state: CliprdrState::Initialization,
            capabilities: Capabilities::new(ClipboardProtocolVersion::V2, flags),
            _marker: core::marker::PhantomData,
        }
    }

    pub fn downcast_backend<T: CliprdrBackend>(&self) -> Option<&T> {
        self.backend.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: CliprdrBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_any_mut().downcast_mut::<T>()
    }

    fn are_long_format_names_enabled(&self) -> bool {
        self.capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES)
    }

    fn build_format_list(&self, formats: &[ClipboardFormat]) -> EncodeResult<FormatList<'static>> {
        FormatList::new_unicode(formats, self.are_long_format_names_enabled())
    }

    fn handle_error_transition(&mut self, err: ClipboardError) -> PduResult<Vec<SvcMessage>> {
        // Failure of clipboard is not an critical error, but we should properly report it
        // and transition channel to failed state.
        self.state = CliprdrState::Failed;
        error!("CLIPRDR(clipboard) failed: {err}");

        Ok(Vec::new())
    }

    fn handle_server_capabilities(&mut self, server_capabilities: Capabilities) -> PduResult<Vec<SvcMessage>> {
        self.capabilities.downgrade(&server_capabilities);
        self.backend
            .on_process_negotiated_capabilities(self.capabilities.flags());

        // Do not send anything, wait for monitor ready pdu
        Ok(Vec::new())
    }

    fn handle_monitor_ready(&mut self) -> PduResult<Vec<SvcMessage>> {
        // Request client to sent list of initially available formats and wait for the backend
        // response.
        self.backend.on_request_format_list();
        Ok(Vec::new())
    }

    fn handle_format_list_response(&mut self, response: FormatListResponse) -> PduResult<Vec<SvcMessage>> {
        match response {
            FormatListResponse::Ok => {
                if !R::is_server() {
                    if self.state == CliprdrState::Initialization {
                        info!("CLIPRDR(clipboard) virtual channel has been initialized");
                        self.state = CliprdrState::Ready;
                        self.backend.on_ready();
                    } else {
                        info!("CLIPRDR(clipboard) Remote has received format list successfully");
                    }
                }
            }
            FormatListResponse::Fail => {
                return self.handle_error_transition(ClipboardError::FormatListRejected);
            }
        }

        Ok(Vec::new())
    }

    fn handle_format_list(&mut self, format_list: FormatList<'_>) -> PduResult<Vec<SvcMessage>> {
        if R::is_server() && self.state == CliprdrState::Initialization {
            info!("CLIPRDR(clipboard) virtual channel has been initialized");
            self.state = CliprdrState::Ready;
            self.backend.on_ready();
        }

        let formats = format_list.get_formats(self.are_long_format_names_enabled())?;
        self.backend.on_remote_copy(&formats);

        let pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);

        Ok(vec![into_cliprdr_message(pdu)])
    }

    /// Submits the format data response, returning a [`CliprdrSvcMessages`] to send on the channel.
    ///
    /// Should be called by the clipboard implementation when it receives data from the OS clipboard
    /// and is ready to sent it to the server. This should happen after
    /// [`CliprdrBackend::on_format_data_request`] is called by [`Cliprdr`].
    ///
    /// If data is not available anymore, an error response should be sent instead.
    pub fn submit_format_data(&self, response: OwnedFormatDataResponse) -> PduResult<CliprdrSvcMessages<R>> {
        ready_guard!(self, submit_format_data);

        let pdu = ClipboardPdu::FormatDataResponse(response);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// Submits the file contents response, returning a [`CliprdrSvcMessages`] to send on the channel.
    ///
    /// Should be called by the clipboard implementation when file data is ready to sent it to the
    /// server. This should happen after [`CliprdrBackend::on_file_contents_request`] is called
    /// by [`Cliprdr`].
    ///
    /// If data is not available anymore, an error response should be sent instead.
    pub fn submit_file_contents(&self, response: FileContentsResponse<'static>) -> PduResult<CliprdrSvcMessages<R>> {
        ready_guard!(self, submit_file_contents);

        let pdu = ClipboardPdu::FileContentsResponse(response);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    pub fn capabilities(&self) -> PduResult<SvcMessage> {
        let pdu = ClipboardPdu::Capabilities(self.capabilities.clone());

        Ok(into_cliprdr_message(pdu))
    }

    pub fn monitor_ready(&self) -> PduResult<SvcMessage> {
        let pdu = ClipboardPdu::MonitorReady;

        Ok(into_cliprdr_message(pdu))
    }

    /// Starts processing of `CLIPRDR` copy command. Should be called by the clipboard
    /// implementation when user performs OS-specific copy command (e.g. `Ctrl+C` shortcut on
    /// keyboard)
    pub fn initiate_copy(&self, available_formats: &[ClipboardFormat]) -> PduResult<CliprdrSvcMessages<R>> {
        let mut pdus = Vec::new();

        if R::is_server() {
            pdus.push(ClipboardPdu::FormatList(
                self.build_format_list(available_formats).map_err(|e| encode_err!(e))?,
            ));
        } else {
            match self.state {
                CliprdrState::Ready => {
                    pdus.push(ClipboardPdu::FormatList(
                        self.build_format_list(available_formats).map_err(|e| encode_err!(e))?,
                    ));
                }
                CliprdrState::Initialization => {
                    // During initialization state, first copy action is synthetic and should be sent along with
                    // capabilities and temporary directory PDUs.
                    pdus.push(ClipboardPdu::Capabilities(self.capabilities.clone()));
                    pdus.push(ClipboardPdu::TemporaryDirectory(
                        ClientTemporaryDirectory::new(self.backend.temporary_directory())
                            .map_err(|e| encode_err!(e))?,
                    ));
                    pdus.push(ClipboardPdu::FormatList(
                        self.build_format_list(available_formats).map_err(|e| encode_err!(e))?,
                    ));
                }
                _ => {
                    error!(?self.state, "Attempted to initiate copy in incorrect state");
                }
            }
        }

        Ok(pdus.into_iter().map(into_cliprdr_message).collect::<Vec<_>>().into())
    }

    /// Starts processing of `CLIPRDR` paste command. Should be called by the clipboard
    /// implementation when user performs OS-specific paste command (e.g. `Ctrl+V` shortcut on
    /// keyboard)
    pub fn initiate_paste(&self, requested_format: ClipboardFormatId) -> PduResult<CliprdrSvcMessages<R>> {
        ready_guard!(self, initiate_paste);

        // When user initiates paste, we should send format data request to server, and expect to
        // receive response with contents via `FormatDataResponse` PDU.
        let pdu = ClipboardPdu::FormatDataRequest(FormatDataRequest {
            format: requested_format,
        });

        Ok(vec![into_cliprdr_message(pdu)].into())
    }
}

impl<R: Role> SvcProcessor for Cliprdr<R> {
    fn channel_name(&self) -> ChannelName {
        Self::CHANNEL_NAME
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.state != CliprdrState::Initialization {
            error!("Attempted to start clipboard static virtual channel in invalid state");
        }

        if R::is_server() {
            Ok(vec![self.capabilities()?, self.monitor_ready()?])
        } else {
            Ok(Vec::new())
        }
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode::<ClipboardPdu<'_>>(payload).map_err(|e| decode_err!(e))?;

        if self.state == CliprdrState::Failed {
            error!("Attempted to process clipboard static virtual channel in failed state");
            return Ok(Vec::new());
        }

        match pdu {
            ClipboardPdu::Capabilities(caps) => self.handle_server_capabilities(caps),
            ClipboardPdu::FormatList(format_list) => self.handle_format_list(format_list),
            ClipboardPdu::FormatListResponse(response) => self.handle_format_list_response(response),
            ClipboardPdu::MonitorReady => self.handle_monitor_ready(),
            ClipboardPdu::LockData(id) => {
                self.backend.on_lock(id);
                Ok(Vec::new())
            }
            ClipboardPdu::UnlockData(id) => {
                self.backend.on_unlock(id);
                Ok(Vec::new())
            }
            ClipboardPdu::FormatDataRequest(request) => {
                self.backend.on_format_data_request(request);

                // NOTE: An actual data should be sent later via `submit_format_data` method,
                // therefore we do not send anything immediately.
                Ok(Vec::new())
            }
            ClipboardPdu::FormatDataResponse(response) => {
                self.backend.on_format_data_response(response);
                Ok(Vec::new())
            }
            ClipboardPdu::FileContentsRequest(request) => {
                self.backend.on_file_contents_request(request);
                Ok(Vec::new())
            }
            ClipboardPdu::FileContentsResponse(response) => {
                self.backend.on_file_contents_response(response);
                Ok(Vec::new())
            }
            ClipboardPdu::TemporaryDirectory(_) => {
                // do nothing
                Ok(Vec::new())
            }
        }
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }
}

fn into_cliprdr_message(pdu: ClipboardPdu<'static>) -> SvcMessage {
    // Adding [`CHANNEL_FLAG_SHOW_PROTOCOL`] is a must for clipboard svc messages, because they
    // contain chunked data. This is the requirement from `MS_RDPBCGR` specification.
    SvcMessage::from(pdu).with_flags(ChannelFlags::SHOW_PROTOCOL)
}

#[derive(Debug)]
pub struct Client {}

impl Role for Client {
    fn is_server() -> bool {
        false
    }
}

#[derive(Debug)]
pub struct Server {}

impl Role for Server {
    fn is_server() -> bool {
        true
    }
}
