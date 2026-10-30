use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, trace, warn};

use super::AnnouncedDevices;
use crate::FILE_REDIRECTOR_CHANNEL_NAME;
use crate::pdu::io::{
    CapabilitiesReply, CreateFileReply, CreateFileRequest, CustomEvent, IoControlReply, IoControlRequest, ReadReply,
    ReadRequest, RequestId, ServerIoPdu, VERSION_WITH_CUSTOM_EVENTS, WriteReply, WriteRequest,
};
use crate::pdu::{Guid, HResult};

/// Bounds the reassembled size of one request, which carries at most one write or IOControl buffer.
const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Hands the session loop the requests of `FileRedirectorChannel` instances made outside [`DvcProcessor::process`].
pub type IoUplink = Arc<dyn Fn(UplinkRequest) -> PduResult<()> + Send + Sync>;

/// A request of one `FileRedirectorChannel` instance for the session loop.
///
/// The instance can close while the request is queued, and the server can then reuse its channel ID
/// for another file. Resolve the request with [`UplinkRequest::into_action`] on the thread that runs the
/// DRDYNVC channel, right before acting on it.
pub struct UplinkRequest {
    shared: Arc<ChannelShared>,
    action: UplinkAction,
}

/// What the session loop does for an [`UplinkRequest`].
#[derive(Debug)]
pub enum UplinkAction {
    /// Writes these encoded DRDYNVC messages to the channel.
    Send(Vec<SvcMessage>),
    /// Closes the channel, which violated the protocol ([MS-RDPEPNP] sections 2.2.2.1.1 and 3.1.1).
    ///
    /// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
    Close,
}

impl UplinkRequest {
    pub fn channel_id(&self) -> u32 {
        self.shared.channel_id
    }

    /// Returns the action to take now, or `None` when the channel instance no longer needs it.
    ///
    /// Messages are sent only while the instance is open, and a close only until the channel closes.
    pub fn into_action(self) -> Option<UplinkAction> {
        let state = self.shared.state.lock();
        let current = match self.action {
            UplinkAction::Send(_) => state.open,
            UplinkAction::Close => !state.closed,
        };
        current.then_some(self.action)
    }
}

impl fmt::Debug for UplinkRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UplinkRequest")
            .field("channel_id", &self.shared.channel_id)
            .field("action", &self.action)
            .finish()
    }
}

/// Opens file handles on the client devices announced on the `PNPDR` channel.
pub trait PnpBackend: Send {
    /// Opens a handle on the device `request.device_id` for one `FileRedirectorChannel` instance.
    ///
    /// It runs on the session loop, which waits for it, so it must not block on device I/O.
    /// The returned error is sent to the server as the CreateFile Reply result.
    /// `events` sends the device's custom events on the same channel.
    fn create_file(
        &mut self,
        request: &CreateFileRequest,
        events: CustomEventSender,
    ) -> Result<Box<dyn PnpFile>, HResult>;
}

/// An open file handle on a client device.
///
/// Dropping the handle closes it; this happens when the server closes its channel.
/// Each request carries a completion that sends exactly one reply, from any thread and at any time.
/// A completion dropped without being completed replies with [`HResult::E_ABORT`].
pub trait PnpFile: Send {
    fn read(&mut self, request: ReadRequest, completion: ReadCompletion);

    fn write(&mut self, request: WriteRequest, completion: WriteCompletion);

    fn io_control(&mut self, request: IoControlRequest, completion: IoControlCompletion);

    /// Asks to cancel the pending request `request_id`, whose completion still has to reply.
    ///
    /// Only requests that have not replied yet are forwarded.
    fn cancel(&mut self, request_id: RequestId) {
        let _ = request_id;
    }
}

struct ChannelState {
    /// Whether replies are still sent: false once the channel closes or fails.
    open: bool,
    /// Whether the channel closed, after which the server can reuse its ID.
    closed: bool,
    /// Requests forwarded to the backend that have not replied yet.
    outstanding: BTreeSet<RequestId>,
    /// Replies completed while the channel processes a message, returned from that call.
    ///
    /// INVARIANT: `Some` exactly while [`FileRedirectorClient::process`] runs the backend.
    inline: Option<Vec<DvcMessage>>,
    /// Whether both sides announced a version that supports custom events.
    custom_events: bool,
}

/// State shared by one channel instance and the completions of its requests.
struct ChannelShared {
    channel_id: u32,
    uplink: IoUplink,
    state: spin::Mutex<ChannelState>,
}

impl ChannelShared {
    /// Sends a message unless the channel was closed, inline when the channel is processing.
    fn send(self: &Arc<Self>, message: DvcMessage) -> PduResult<()> {
        let mut state = self.state.lock();
        if !state.open {
            return Err(pdu_other_err!("FileRedirectorChannel", "channel is closed"));
        }
        // UNTESTED: a reply completed on another thread while the channel processes a message is
        // returned by that call or sent through the uplink, never both and never neither, because
        // both the check here and `FileRedirectorClient::process` taking the queue hold the lock.
        // Losing a reply leaves the server application's request pending forever.
        // `reply_completed_on_another_thread_is_sent_exactly_once` races the two paths but cannot
        // force the interleaving where the call ends between the check and the push.
        // Keep the check and the push under one lock acquisition when changing this.
        if let Some(inline) = state.inline.as_mut() {
            inline.push(message);
            return Ok(());
        }
        drop(state);
        // The channel can close from here on; the session loop checks `into_action` before writing.
        let messages = encode_dvc_messages(self.channel_id, vec![message], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        (self.uplink)(UplinkRequest {
            shared: Arc::clone(self),
            action: UplinkAction::Send(messages),
        })
    }

    fn reply(self: &Arc<Self>, request_id: RequestId, message: DvcMessage) {
        if !self.state.lock().outstanding.remove(&request_id) {
            // The channel was closed or failed, which discards its outstanding requests.
            trace!(
                request_id = request_id.get(),
                "Dropping the reply of a discarded request"
            );
            return;
        }
        if let Err(error) = self.send(message) {
            warn!(%error, request_id = request_id.get(), "Unable to send a PnP I/O reply");
        }
    }
}

/// Sends the reply of one request, or an abort reply when dropped.
struct Completion {
    target: Option<(Arc<ChannelShared>, RequestId)>,
    abort: fn(RequestId) -> DvcMessage,
}

impl Completion {
    fn complete(mut self, reply: impl FnOnce(RequestId) -> DvcMessage) {
        if let Some((shared, request_id)) = self.target.take() {
            shared.reply(request_id, reply(request_id));
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some((shared, request_id)) = self.target.take() {
            shared.reply(request_id, (self.abort)(request_id));
        }
    }
}

impl fmt::Debug for Completion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Completion")
            .field(
                "request_id",
                &self.target.as_ref().map(|(_, request_id)| request_id.get()),
            )
            .finish()
    }
}

/// Completes a Read Request with a Read Reply message.
#[derive(Debug)]
pub struct ReadCompletion {
    inner: Completion,
    bytes_to_read: u32,
}

impl ReadCompletion {
    /// Sends the result and the data read, which must not exceed the requested size.
    ///
    /// Oversized data is replaced with an [`HResult::INSUFFICIENT_BUFFER`] reply.
    pub fn complete(self, result: HResult, data: Vec<u8>) {
        let (result, data) = if fits(data.len(), self.bytes_to_read) {
            (result, data)
        } else {
            warn!(len = data.len(), "PnP read returned more data than requested");
            (HResult::INSUFFICIENT_BUFFER, Vec::new())
        };
        self.inner.complete(|request_id| {
            Box::new(ReadReply {
                request_id,
                result,
                data,
            })
        });
    }
}

/// Completes a Write Request with a Write Reply message.
#[derive(Debug)]
pub struct WriteCompletion {
    inner: Completion,
}

impl WriteCompletion {
    pub fn complete(self, result: HResult, bytes_written: u32) {
        self.inner.complete(|request_id| {
            Box::new(WriteReply {
                request_id,
                result,
                bytes_written,
            })
        });
    }
}

/// Completes an IOControl Request with an IOControl Reply message.
#[derive(Debug)]
pub struct IoControlCompletion {
    inner: Completion,
    output_size: u32,
}

impl IoControlCompletion {
    /// Sends the result and the output data, which must not exceed the request's `cbOut`.
    ///
    /// Oversized output is replaced with an [`HResult::INSUFFICIENT_BUFFER`] reply, because
    /// the server terminates the channel on a reply larger than `cbOut`
    /// ([MS-RDPEPNP] section 3.3.5.2.2.8).
    ///
    /// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
    pub fn complete(self, result: HResult, data: Vec<u8>) {
        let (result, data) = if fits(data.len(), self.output_size) {
            (result, data)
        } else {
            warn!(len = data.len(), "PnP IOControl returned more data than cbOut");
            (HResult::INSUFFICIENT_BUFFER, Vec::new())
        };
        self.inner.complete(|request_id| {
            Box::new(IoControlReply {
                request_id,
                result,
                data,
            })
        });
    }
}

/// Sends Client Device Custom Event messages on one `FileRedirectorChannel` instance.
#[derive(Clone)]
pub struct CustomEventSender {
    shared: Arc<ChannelShared>,
}

impl CustomEventSender {
    /// Returns whether both sides negotiated version 6, which custom events require.
    pub fn is_supported(&self) -> bool {
        self.shared.state.lock().custom_events
    }

    /// Sends a custom event.
    ///
    /// Fails when custom events are not supported or the channel is closed.
    pub fn send(&self, event_guid: Guid, data: Vec<u8>) -> PduResult<()> {
        if !self.is_supported() {
            return Err(pdu_other_err!(
                "CustomEventSender::send",
                "the server does not support custom events"
            ));
        }
        self.shared.send(Box::new(CustomEvent { event_guid, data }))
    }
}

impl fmt::Debug for CustomEventSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomEventSender")
            .field("channel_id", &self.shared.channel_id)
            .finish_non_exhaustive()
    }
}

type SharedBackend = Arc<spin::Mutex<Box<dyn PnpBackend>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    WaitingCapabilities,
    WaitingCreateFile,
    Open,
    /// The channel violated the protocol or its file failed to open; further messages are ignored.
    Failed,
}

/// Client processor for one `FileRedirectorChannel` instance (PNP Device I/O Subprotocol).
///
/// A protocol violation terminates the channel: its file is closed, its pending replies are
/// discarded, later messages are ignored, and an [`UplinkAction::Close`] asks the session loop to
/// close it, since a processor error would end the whole session.
pub struct FileRedirectorClient {
    backend: SharedBackend,
    announced: AnnouncedDevices,
    uplink: IoUplink,
    state: State,
    shared: Option<Arc<ChannelShared>>,
    /// INVARIANT: `Some` while `state` is [`State::Open`], and `None` once the channel closes or fails.
    file: Option<Box<dyn PnpFile>>,
}

impl FileRedirectorClient {
    fn new(backend: SharedBackend, announced: AnnouncedDevices, uplink: IoUplink) -> Self {
        Self {
            backend,
            announced,
            uplink,
            state: State::WaitingCapabilities,
            shared: None,
            file: None,
        }
    }

    fn handle(&mut self, shared: &Arc<ChannelShared>, pdu: ServerIoPdu) -> Result<(), &'static str> {
        match (self.state, pdu) {
            (State::WaitingCapabilities, ServerIoPdu::Capabilities(request)) => {
                debug!(version = request.version, "Received the PnP I/O server capabilities");
                let mut state = shared.state.lock();
                state.custom_events = VERSION_WITH_CUSTOM_EVENTS <= request.version;
                push_inline(
                    &mut state,
                    Box::new(CapabilitiesReply {
                        request_id: request.request_id,
                        version: VERSION_WITH_CUSTOM_EVENTS,
                    }),
                );
                self.state = State::WaitingCreateFile;
                Ok(())
            }
            (State::WaitingCreateFile, ServerIoPdu::CreateFile(request)) => {
                let result = if self.announced.lock().contains(&request.device_id) {
                    let events = CustomEventSender {
                        shared: Arc::clone(shared),
                    };
                    self.backend.lock().create_file(&request, events)
                } else {
                    debug!(device_id = request.device_id, "CreateFile names an unannounced device");
                    Err(HResult::FILE_NOT_FOUND)
                };
                let result = match result {
                    Ok(file) => {
                        self.file = Some(file);
                        self.state = State::Open;
                        HResult::S_OK
                    }
                    Err(result) => {
                        debug!(
                            device_id = request.device_id,
                            result = result.0,
                            "PnP CreateFile failed"
                        );
                        // A channel carries a single CreateFile request.
                        self.state = State::Failed;
                        result
                    }
                };
                push_inline(
                    &mut shared.state.lock(),
                    Box::new(CreateFileReply {
                        request_id: request.request_id,
                        result,
                    }),
                );
                Ok(())
            }
            (State::Open, ServerIoPdu::SpecificIoCancel(request)) => {
                // A cancel for a request that already replied is ignored ([MS-RDPEPNP] section 3.2.5.2.2.9).
                let pending = shared.state.lock().outstanding.contains(&request.id_to_cancel);
                if let (true, Some(file)) = (pending, self.file.as_mut()) {
                    file.cancel(request.id_to_cancel);
                }
                Ok(())
            }
            (State::Open, ServerIoPdu::IoControl(request))
                if !request.output.is_empty() && !fits_exactly(request.output.len(), request.output_size) =>
            {
                // A DataOut that is present but not cbOut bytes long completes with
                // ERROR_INSUFFICIENT_BUFFER ([MS-RDPEPNP] section 3.2.5.2.2.7).
                let mut state = shared.state.lock();
                if state.outstanding.contains(&request.request_id) {
                    return Err("duplicate request ID");
                }
                push_inline(
                    &mut state,
                    Box::new(IoControlReply {
                        request_id: request.request_id,
                        result: HResult::INSUFFICIENT_BUFFER,
                        data: Vec::new(),
                    }),
                );
                Ok(())
            }
            (State::Open, request @ (ServerIoPdu::Read(_) | ServerIoPdu::Write(_) | ServerIoPdu::IoControl(_))) => {
                let request_id = request.request_id();
                // The client terminates the channel on a duplicate request ID ([MS-RDPEPNP] section 3.1.1).
                if !shared.state.lock().outstanding.insert(request_id) {
                    return Err("duplicate request ID");
                }
                let file = self.file.as_mut().expect("an open channel holds its file");
                let target = Some((Arc::clone(shared), request_id));
                match request {
                    ServerIoPdu::Read(request) => {
                        let bytes_to_read = request.bytes_to_read;
                        file.read(
                            request,
                            ReadCompletion {
                                inner: Completion {
                                    target,
                                    abort: |request_id| {
                                        Box::new(ReadReply {
                                            request_id,
                                            result: HResult::E_ABORT,
                                            data: Vec::new(),
                                        })
                                    },
                                },
                                bytes_to_read,
                            },
                        );
                    }
                    ServerIoPdu::Write(request) => file.write(
                        request,
                        WriteCompletion {
                            inner: Completion {
                                target,
                                abort: |request_id| {
                                    Box::new(WriteReply {
                                        request_id,
                                        result: HResult::E_ABORT,
                                        bytes_written: 0,
                                    })
                                },
                            },
                        },
                    ),
                    ServerIoPdu::IoControl(request) => {
                        let output_size = request.output_size;
                        file.io_control(
                            request,
                            IoControlCompletion {
                                inner: Completion {
                                    target,
                                    abort: |request_id| {
                                        Box::new(IoControlReply {
                                            request_id,
                                            result: HResult::E_ABORT,
                                            data: Vec::new(),
                                        })
                                    },
                                },
                                output_size,
                            },
                        );
                    }
                    _ => unreachable!("the outer match selects I/O requests"),
                }
                Ok(())
            }
            (State::WaitingCapabilities, _) => Err("the first message is not a Server Capabilities Request"),
            (State::WaitingCreateFile, _) => Err("an I/O request precedes the CreateFile Request"),
            (State::Open, _) => Err("a second initialization request on an open channel"),
            (State::Failed, _) => unreachable!("failed channels skip message handling"),
        }
    }

    /// Terminates the channel: closes its file, discards its pending replies, and asks the session loop to close it.
    fn fail(&mut self, reason: &str) {
        warn!(
            reason,
            "Closing a FileRedirectorChannel instance after a protocol violation"
        );
        self.state = State::Failed;
        self.discard();
        if let Some(shared) = &self.shared {
            let request = UplinkRequest {
                shared: Arc::clone(shared),
                action: UplinkAction::Close,
            };
            if let Err(error) = (shared.uplink)(request) {
                warn!(%error, "Unable to close a FileRedirectorChannel instance");
            }
        }
    }

    fn discard(&mut self) {
        if let Some(shared) = &self.shared {
            let mut state = shared.state.lock();
            state.open = false;
            state.outstanding.clear();
        }
        self.file = None;
    }
}

impl fmt::Debug for FileRedirectorClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRedirectorClient")
            .field("state", &self.state)
            .field("channel_id", &self.shared.as_ref().map(|shared| shared.channel_id))
            .finish_non_exhaustive()
    }
}

impl_as_any!(FileRedirectorClient);

impl DvcProcessor for FileRedirectorClient {
    fn channel_name(&self) -> &str {
        FILE_REDIRECTOR_CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_MESSAGE_SIZE)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.shared = Some(Arc::new(ChannelShared {
            channel_id,
            uplink: Arc::clone(&self.uplink),
            state: spin::Mutex::new(ChannelState {
                open: true,
                closed: false,
                outstanding: BTreeSet::new(),
                inline: None,
                custom_events: false,
            }),
        }));
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if self.state == State::Failed {
            trace!("Ignoring a message on a disabled FileRedirectorChannel instance");
            return Ok(Vec::new());
        }
        let shared = Arc::clone(
            self.shared
                .as_ref()
                .ok_or_else(|| pdu_other_err!("FileRedirectorClient", "channel is not started"))?,
        );
        let pdu = match decode::<ServerIoPdu>(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                // An unknown FunctionId terminates the channel ([MS-RDPEPNP] section 2.2.2.1.1).
                debug!(%error, "Malformed PnP I/O request");
                self.fail("malformed request");
                return Ok(Vec::new());
            }
        };

        shared.state.lock().inline = Some(Vec::new());
        // The lock is released while the backend runs, so that its completions can take it.
        let outcome = self.handle(&shared, pdu);
        let replies = shared.state.lock().inline.take().unwrap_or_default();
        match outcome {
            Ok(()) => Ok(replies),
            Err(reason) => {
                self.fail(reason);
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        if let Some(shared) = &self.shared {
            shared.state.lock().closed = true;
        }
        self.discard();
    }
}

impl DvcClientProcessor for FileRedirectorClient {}

/// Creates a [`FileRedirectorClient`] for each `FileRedirectorChannel` instance the server opens.
///
/// Created by [`PnpInfoClient::file_redirector_listener`](super::PnpInfoClient::file_redirector_listener).
pub struct FileRedirectorListener {
    backend: SharedBackend,
    announced: AnnouncedDevices,
    uplink: IoUplink,
}

impl FileRedirectorListener {
    pub(super) fn new(backend: Box<dyn PnpBackend>, announced: AnnouncedDevices, uplink: IoUplink) -> Self {
        Self {
            backend: Arc::new(spin::Mutex::new(backend)),
            announced,
            uplink,
        }
    }
}

impl fmt::Debug for FileRedirectorListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRedirectorListener").finish_non_exhaustive()
    }
}

impl DvcChannelListener for FileRedirectorListener {
    fn channel_name(&self) -> &str {
        FILE_REDIRECTOR_CHANNEL_NAME
    }

    fn create(&mut self, _channel_id: u32) -> Option<Box<dyn DvcClientProcessor>> {
        Some(Box::new(FileRedirectorClient::new(
            Arc::clone(&self.backend),
            Arc::clone(&self.announced),
            Arc::clone(&self.uplink),
        )))
    }
}

fn push_inline(state: &mut ChannelState, message: DvcMessage) {
    state
        .inline
        .as_mut()
        .expect("replies are pushed only while processing")
        .push(message);
}

fn fits(len: usize, limit: u32) -> bool {
    u32::try_from(len).is_ok_and(|len| len <= limit)
}

fn fits_exactly(len: usize, size: u32) -> bool {
    u32::try_from(len).is_ok_and(|len| len == size)
}
