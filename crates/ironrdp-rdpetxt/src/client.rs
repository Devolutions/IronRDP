//! Client side of the text input channels.
//!
//! The server sends on [`SERVER_TO_CLIENT_CHANNEL_NAME`] and the client answers on
//! [`CLIENT_TO_SERVER_CHANNEL_NAME`], so a reply cannot travel back on the channel its request
//! arrived on.
//! Both processors therefore share one [`TextInputClient`]: [`ServerToClientChannel`] feeds it
//! server PDUs, and the PDUs it queues leave through [`ClientToServerChannel`], when that channel
//! opens and then whenever the session drains [`ClientToServerChannel::pending_messages`].
//!
//! [`TextInputClient`] answers what [3.1.5] Message Processing Events and Sequencing Rules asks
//! of a client, tracks the focused edit control, and inserts committed text into it.
//! It hosts no input method, so it neither undoes nor replays pending key events after an update
//! collision ([1.3.2] Key Replay and Override).
//!
//! [3.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
//! [1.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, trace, warn};

use crate::channel::{lock, process_frames, versioned_messages};
use crate::pdu::{
    AcknowledgeHostOperationPdu, AcknowledgeOperationPdu, AcknowledgeUndoPendingKeyEventsPdu, AcknowledgementType,
    CandidateList, ClientOptions, ClientPdu, EditControlFocusPdu, EditControlInfo, EditControlRange,
    NotifyClientVersionPdu, PduLayout, ReconversionCandidatesPdu, ReportClientOptionsPdu, ServerPdu, TextInputRect,
    TextString, UpdateTextAndSelectionPdu,
};
use crate::{
    CLIENT_TO_SERVER_CHANNEL_NAME, MAX_MESSAGE_SIZE, MAX_PENDING_PDUS, QueueFull, SERVER_TO_CLIENT_CHANNEL_NAME,
};

/// Largest number of registered key targets tracked at once.
pub const MAX_KEY_TARGETS: usize = 1024;

/// Largest number of registered edit controls tracked at once.
pub const MAX_EDIT_CONTROLS: usize = 1024;

/// Why [`TextInputClient::commit_text`] sent nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitTextError {
    /// No remote edit control has focus.
    NoFocus,
    /// The caret after the text would not fit a 32-bit position.
    TooLong,
    /// The queue for the server is full.
    QueueFull,
}

impl core::fmt::Display for CommitTextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NoFocus => "no remote edit control has focus",
            Self::TooLong => "the text does not fit the edit control positions",
            Self::QueueFull => "too many text input PDUs are queued",
        })
    }
}

impl core::error::Error for CommitTextError {}

/// Version numbers announced by a client ([1.7] Versioning and Capability Negotiation).
///
/// [1.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextInputConfig {
    pub version_major: u32,
    /// Bitmask of the updates the client implements since `version_major`.
    pub version_minor: u32,
    /// Options reported after the version; none are reported when empty.
    pub options: ClientOptions,
}

impl Default for TextInputConfig {
    /// Version 1 with the only documented update, the **bcpTag** of `CoreInputProfile`
    /// (minor bit 0x00000002), and predicted key reporting.
    ///
    /// The client hosts no input method, so it has no local text input system to wait for the keys
    /// it sends.
    /// Without the option, the server routes keys back for the client input method with
    /// `RDPTXT_SEND_KEY_TO_HOST_PDU`, which this client cannot process ([2.2.2.54]).
    ///
    /// [2.2.2.54]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    fn default() -> Self {
        Self {
            version_major: 1,
            version_minor: 0x0000_0002,
            options: ClientOptions::ENABLE_PREDICTED_KEY_REPORTING,
        }
    }
}

/// The edit control holding focus on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusedEditControl {
    pub text_input_client_id: u32,
    pub info: EditControlInfo,
    pub bounds: TextInputRect,
    /// Selection, or caret when empty, as last reported by the server or set by
    /// [`TextInputClient::commit_text`].
    pub selection: EditControlRange,
}

impl FocusedEditControl {
    fn is(&self, text_input_client_id: u32, edit_control_id: u32) -> bool {
        self.text_input_client_id == text_input_client_id && self.info.id == edit_control_id
    }
}

/// Callback notified of every server PDU, after [`TextInputClient`] processed it.
///
/// It runs while the [`SharedTextInputClient`] mutex is held, so it must not lock that client.
pub type TextInputObserver = Box<dyn FnMut(&ServerPdu) + Send>;

/// Client state shared by both text input channel processors.
pub type SharedTextInputClient = Arc<Mutex<TextInputClient>>;

/// Client protocol state for MS-RDPETXT.
pub struct TextInputClient {
    config: TextInputConfig,
    observer: Option<TextInputObserver>,
    server_version: Option<(u32, u32)>,
    /// Registered key targets (`TextInputHost`) and the text target on their thread.
    key_targets: BTreeMap<u32, u32>,
    /// Last nonzero **ordinal** seen per key target.
    host_ordinals: BTreeMap<u32, u32>,
    /// **editClientOperationId** of each registered edit control, by
    /// (**textInputClientId**, **editControlId**).
    edit_controls: BTreeMap<(u32, u32), u32>,
    /// Edit controls whose focus loss was acknowledged when the next control gained focus,
    /// before the server reported the loss itself.
    folded_control_losses: BTreeSet<(u32, u32)>,
    /// Key targets whose focus loss was acknowledged when an edit control of another key target
    /// gained focus, before the server reported the loss itself.
    folded_host_losses: BTreeSet<u32>,
    focused_host: Option<u32>,
    focus: Option<FocusedEditControl>,
    /// INVARIANT: never zero, which the PDUs without an operation identifier carry.
    next_operation_id: u32,
    /// INVARIANT: `pending.len() <= MAX_PENDING_PDUS`.
    pending: Vec<ClientPdu>,
}

impl TextInputClient {
    pub fn new(config: TextInputConfig) -> Self {
        Self {
            config,
            observer: None,
            server_version: None,
            key_targets: BTreeMap::new(),
            host_ordinals: BTreeMap::new(),
            edit_controls: BTreeMap::new(),
            folded_control_losses: BTreeSet::new(),
            folded_host_losses: BTreeSet::new(),
            focused_host: None,
            focus: None,
            next_operation_id: 1,
            pending: Vec::new(),
        }
    }

    /// Notifies `observer` of every server PDU.
    #[must_use]
    pub fn with_observer(mut self, observer: TextInputObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Wraps the client for sharing between [`ServerToClientChannel`] and [`ClientToServerChannel`].
    pub fn into_shared(self) -> SharedTextInputClient {
        Arc::new(Mutex::new(self))
    }

    /// Major and minor version announced by the server, once received.
    pub fn server_version(&self) -> Option<(u32, u32)> {
        self.server_version
    }

    /// The fields both peers support, or the base layout until the server announced its version.
    pub fn layout(&self) -> PduLayout {
        self.server_version.map_or(PduLayout::BASE, |server_version| {
            PduLayout::negotiated((self.config.version_major, self.config.version_minor), server_version)
        })
    }

    /// The edit control holding focus on the server.
    pub fn focused_edit_control(&self) -> Option<&FocusedEditControl> {
        self.focus.as_ref()
    }

    /// Queues `pdu` for the server, unless [`MAX_PENDING_PDUS`] PDUs are queued already.
    pub fn send(&mut self, pdu: impl Into<ClientPdu>) -> Result<(), QueueFull> {
        if self.pending.len() == MAX_PENDING_PDUS {
            return Err(QueueFull);
        }
        self.pending.push(pdu.into());
        Ok(())
    }

    /// Removes and returns the PDUs queued for the server.
    pub fn take_pending(&mut self) -> Vec<ClientPdu> {
        core::mem::take(&mut self.pending)
    }

    /// Replaces the selection of the focused edit control with `text` and places the caret
    /// after it.
    ///
    /// Returns the operation identifier the server acknowledges with
    /// `RDPTXT_ACKNOWLEDGE_REMOTE_OPERATION_PDU`, which reaches the observer; a rejection is also
    /// logged.
    /// Nothing is queued and the selection is left as is when this fails.
    pub fn commit_text(&mut self, text: &str) -> Result<u32, CommitTextError> {
        let focus = self.focus.ok_or(CommitTextError::NoFocus)?;
        let new_text = TextString::new(text);
        let begin = focus.selection.begin.min(focus.selection.end);
        let end = focus.selection.begin.max(focus.selection.end);
        let caret = u32::try_from(new_text.utf16_len())
            .ok()
            .and_then(|length| begin.checked_add(length))
            .ok_or(CommitTextError::TooLong)?;
        let operation_id = self.next_operation_id();
        let text_input_host_id = self.host_of(focus.text_input_client_id);
        self.send(UpdateTextAndSelectionPdu {
            text_input_client_id: focus.text_input_client_id,
            edit_control_id: focus.info.id,
            text_input_host_id,
            operation_id,
            replace_begin: begin,
            replace_end: end,
            new_text,
            selection_begin: caret,
            selection_end: caret,
        })
        .map_err(|QueueFull| CommitTextError::QueueFull)?;
        self.focus = Some(FocusedEditControl {
            selection: EditControlRange {
                begin: caret,
                end: caret,
            },
            ..focus
        });
        Ok(operation_id)
    }

    /// Applies a server PDU and queues the replies it calls for.
    ///
    /// Fails when a reply cannot be queued; the replies queued before it stay queued.
    pub fn process(&mut self, pdu: ServerPdu) -> Result<(), QueueFull> {
        trace!(pdu_id = pdu.pdu_id(), "Text input PDU received");
        match &pdu {
            ServerPdu::NotifyServerVersion(version) => {
                debug!(
                    major = version.version_major,
                    minor = version.version_minor,
                    "Text input server version received"
                );
                self.server_version = Some((version.version_major, version.version_minor));
                self.send(NotifyClientVersionPdu {
                    version_major: self.config.version_major,
                    version_minor: self.config.version_minor,
                })?;
                if self.config.options != ClientOptions::NONE {
                    self.send(ReportClientOptionsPdu {
                        options: self.config.options,
                    })?;
                }
            }
            ServerPdu::RefreshClient(_) => {
                // The server purges its registrations and sends them again.
                self.forget_registrations();
            }
            ServerPdu::RegisterRemoteKeyTarget(target) => {
                if !self.key_targets.contains_key(&target.object_id) && self.key_targets.len() == MAX_KEY_TARGETS {
                    warn!(
                        object_id = target.object_id,
                        "Ignoring a text input key target beyond the supported count"
                    );
                } else {
                    self.key_targets.insert(target.object_id, target.text_target_id);
                }
            }
            ServerPdu::UnregisterRemoteKeyTarget(target) => {
                self.key_targets.remove(&target.object_id);
                self.host_ordinals.remove(&target.object_id);
                if self.focused_host == Some(target.object_id) {
                    self.focused_host = None;
                }
            }
            ServerPdu::RegisterRemoteEditControl(control) => {
                let key = (control.text_input_client_id, control.edit_control_id);
                if !self.edit_controls.contains_key(&key) && self.edit_controls.len() == MAX_EDIT_CONTROLS {
                    warn!(
                        text_input_client_id = control.text_input_client_id,
                        edit_control_id = control.edit_control_id,
                        "Ignoring a text input edit control beyond the supported count"
                    );
                } else {
                    self.edit_controls.insert(key, control.edit_client_operation_id);
                }
            }
            ServerPdu::UnregisterRemoteTextTarget(target) => {
                self.edit_controls
                    .retain(|&(text_input_client_id, _), _| text_input_client_id != target.object_id);
                if self
                    .focus
                    .is_some_and(|focus| focus.text_input_client_id == target.object_id)
                {
                    self.focus = None;
                }
            }
            ServerPdu::UnregisterRemoteEditControl(control) => {
                self.edit_controls
                    .remove(&(control.text_input_client_id, control.edit_control_id));
                if self
                    .focus
                    .is_some_and(|focus| focus.is(control.text_input_client_id, control.edit_control_id))
                {
                    self.focus = None;
                }
            }
            ServerPdu::EditControlFocus(focus) => {
                if focus.gaining_focus {
                    self.fold_focus_losses(focus)?;
                    self.focus = Some(FocusedEditControl {
                        text_input_client_id: focus.text_input_client_id,
                        info: focus.edit_info,
                        bounds: focus.control_bounds,
                        selection: EditControlRange::default(),
                    });
                } else {
                    if self
                        .focus
                        .is_some_and(|current| current.is(focus.text_input_client_id, focus.edit_info.id))
                    {
                        self.focus = None;
                    }
                    if !self
                        .folded_control_losses
                        .remove(&(focus.text_input_client_id, focus.edit_info.id))
                    {
                        self.complete_focus_loss(focus.text_input_client_id, focus.edit_info.id)?;
                    }
                }
            }
            ServerPdu::HostFocus(host) => {
                let last_ordinal = self.host_ordinals.get(&host.text_input_host_id).copied();
                // An ordinal older than the last one seen marks a stale notification ([2.2.2.33]).
                let stale = host.ordinal != 0 && last_ordinal.is_some_and(|last| host.ordinal < last);
                if stale {
                    debug!(
                        text_input_host_id = host.text_input_host_id,
                        ordinal = host.ordinal,
                        "Ignoring a stale text input host focus change"
                    );
                } else {
                    if host.ordinal != 0 && self.key_targets.contains_key(&host.text_input_host_id) {
                        self.host_ordinals.insert(host.text_input_host_id, host.ordinal);
                    }
                    if host.gaining_focus {
                        self.focused_host = Some(host.text_input_host_id);
                    } else if self.focused_host == Some(host.text_input_host_id) {
                        self.focused_host = None;
                    }
                }
                // The acknowledgement reports receipt, so a stale notification is acknowledged too
                // ([3.1.5.2.6]), unless a focus gain already acknowledged this loss.
                if !host.gaining_focus && !self.folded_host_losses.remove(&host.text_input_host_id) {
                    self.send(AcknowledgeHostOperationPdu {
                        text_input_host_id: host.text_input_host_id,
                        acknowledgement_type: AcknowledgementType::FOCUS_LOSS,
                    })?;
                }
            }
            ServerPdu::SelectionChanged(selection) => {
                let range = u32::try_from(selection.selection_begin).and_then(|begin| {
                    Ok(EditControlRange {
                        begin,
                        end: u32::try_from(selection.selection_end)?,
                    })
                });
                if let Some(focus) = self
                    .focus
                    .as_mut()
                    .filter(|focus| focus.is(selection.text_input_client_id, selection.edit_control_id))
                {
                    match range {
                        Ok(range) => focus.selection = range,
                        // The positions are signed, but the specification gives no meaning to a
                        // negative one ([2.2.2.35]), so the last known selection stays.
                        Err(_) => debug!(
                            begin = selection.selection_begin,
                            end = selection.selection_end,
                            "Ignoring a negative text input selection"
                        ),
                    }
                }
                self.acknowledge_control(
                    selection.text_input_client_id,
                    selection.edit_control_id,
                    AcknowledgementType::SELECTION_CHANGE,
                )?;
            }
            ServerPdu::TextChanged(change) => {
                if let Some(focus) = self
                    .focus
                    .as_mut()
                    .filter(|focus| focus.is(change.text_input_client_id, change.edit_control_id))
                {
                    focus.selection = change.new_selection_range;
                }
                self.send(AcknowledgeOperationPdu {
                    text_input_client_id: change.text_input_client_id,
                    edit_control_id: change.edit_control_id,
                    acknowledgement_type: AcknowledgementType::TEXT_CHANGE,
                    operation_id: change.operation_id,
                })?;
            }
            ServerPdu::ControlConfigurationUpdated(update) => {
                if let Some(focus) = self
                    .focus
                    .as_mut()
                    .filter(|focus| focus.is(update.text_input_client_id, update.edit_info.id))
                {
                    focus.info = update.edit_info;
                }
            }
            ServerPdu::GeometryChanged(geometry) => {
                if let Some(focus) = self
                    .focus
                    .as_mut()
                    .filter(|focus| focus.is(geometry.text_input_client_id, geometry.edit_control_id))
                {
                    focus.bounds = geometry.control_bounds;
                }
            }
            ServerPdu::CompositionTerminated(termination) => self.acknowledge_control(
                termination.text_input_client_id,
                termination.edit_control_id,
                AcknowledgementType::COMPOSITION_TERMINATED,
            )?,
            ServerPdu::ControlConversionModeUpdated(update) => self.acknowledge_control(
                update.text_input_client_id,
                update.edit_control_id,
                AcknowledgementType::CONVERSION_MODE_CHANGED,
            )?,
            ServerPdu::UndoPendingKeyEvents(undo) => {
                // No key event is ever pending here, so there is nothing to undo ([3.1.5.2.7]).
                self.send(AcknowledgeUndoPendingKeyEventsPdu {
                    undo_request_id: undo.undo_request_id,
                })?;
            }
            ServerPdu::DoReconversion(request) => {
                if request.return_candidates || request.return_range {
                    // Without an input method there are no candidates; the range is the
                    // selection of the control when it has focus ([3.1.5.1]).
                    let reconvert_range = self
                        .focus
                        .filter(|focus| focus.is(request.text_input_client_id, request.edit_control_id))
                        .map(|focus| focus.selection)
                        .unwrap_or_default();
                    self.send(ReconversionCandidatesPdu {
                        text_input_client_id: request.text_input_client_id,
                        candidate_list: CandidateList::default(),
                        edit_control_id: request.edit_control_id,
                        reconvert_range,
                    })?;
                }
            }
            ServerPdu::AcknowledgeRemoteOperation(ack) if ack.error_code != 0 => {
                warn!(
                    operation_id = ack.operation_id,
                    error_code = ack.error_code,
                    "The server rejected a text input operation"
                );
            }
            ServerPdu::SendKeyToHost(key) => {
                debug!(op_id = key.op_id, "Ignoring a key routed to the client input method");
            }
            _ => {}
        }
        if let Some(observer) = self.observer.as_mut() {
            observer(&pdu);
        }
        Ok(())
    }

    /// Forgets the server state, as when the channels restart.
    ///
    /// The replies still queued answer notifications of the previous channel, so they are dropped.
    pub fn reset(&mut self) {
        self.server_version = None;
        self.forget_registrations();
        self.pending.clear();
    }

    fn forget_registrations(&mut self) {
        self.key_targets.clear();
        self.host_ordinals.clear();
        self.edit_controls.clear();
        self.folded_control_losses.clear();
        self.folded_host_losses.clear();
        self.focused_host = None;
        self.focus = None;
    }

    /// Acknowledges the focus loss of an edit control, which completes at once because no input
    /// method holds state for the control ([3.1.5.2.1]).
    ///
    /// [3.1.5.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    fn complete_focus_loss(&mut self, text_input_client_id: u32, edit_control_id: u32) -> Result<(), QueueFull> {
        for acknowledgement_type in [
            AcknowledgementType::FOCUS_LOSS,
            AcknowledgementType::FOCUS_LEAVE_COMPLETED,
        ] {
            self.acknowledge_control(text_input_client_id, edit_control_id, acknowledgement_type)?;
        }
        Ok(())
    }

    /// Acknowledges the focus losses a focus gain reports before, or instead of, their own
    /// notifications ([2.2.2.32] **losingFocusControlId** and **losingFocusTextInputHostId**).
    ///
    /// Only a loss the client has not seen yet is acknowledged here, and the late notification of
    /// that loss is then not acknowledged again.
    ///
    /// [2.2.2.32]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    fn fold_focus_losses(&mut self, gain: &EditControlFocusPdu) -> Result<(), QueueFull> {
        let losing_control = self
            .focus
            .filter(|current| current.info.id == gain.losing_focus_control_id)
            .filter(|current| !current.is(gain.text_input_client_id, gain.edit_info.id));
        if let Some(lost) = losing_control {
            self.complete_focus_loss(lost.text_input_client_id, lost.info.id)?;
            if self.folded_control_losses.len() < MAX_EDIT_CONTROLS {
                self.folded_control_losses
                    .insert((lost.text_input_client_id, lost.info.id));
            }
        }
        let losing_host = gain.losing_focus_text_input_host_id;
        if losing_host != 0 && self.focused_host == Some(losing_host) {
            self.send(AcknowledgeHostOperationPdu {
                text_input_host_id: losing_host,
                acknowledgement_type: AcknowledgementType::FOCUS_LOSS,
            })?;
            self.focused_host = None;
            if self.folded_host_losses.len() < MAX_KEY_TARGETS {
                self.folded_host_losses.insert(losing_host);
            }
        }
        Ok(())
    }

    /// Acknowledges a notification about an edit control that carries no operation identifier.
    ///
    /// The acknowledgement carries the **editClientOperationId** of the control's registration,
    /// which [2.2.2.26] says MUST be acknowledged back, or zero for an unregistered control.
    /// No acknowledgement type exists for the registration itself.
    ///
    /// [2.2.2.26]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    fn acknowledge_control(
        &mut self,
        text_input_client_id: u32,
        edit_control_id: u32,
        acknowledgement_type: AcknowledgementType,
    ) -> Result<(), QueueFull> {
        // UNTESTED: no live server confirms that it expects the registration identifier here.
        let operation_id = self
            .edit_controls
            .get(&(text_input_client_id, edit_control_id))
            .copied()
            .unwrap_or(0);
        self.send(AcknowledgeOperationPdu {
            text_input_client_id,
            edit_control_id,
            acknowledgement_type,
            operation_id,
        })
    }

    /// The key target to address for an edit control of `text_input_client_id`.
    ///
    /// The focused key target is preferred when it belongs to the same thread.
    fn host_of(&self, text_input_client_id: u32) -> u32 {
        let on_thread = |host: &u32| self.key_targets.get(host) == Some(&text_input_client_id);
        self.focused_host
            .filter(on_thread)
            .or_else(|| self.key_targets.keys().copied().find(on_thread))
            .or(self.focused_host)
            .unwrap_or(0)
    }

    fn next_operation_id(&mut self) -> u32 {
        let operation_id = self.next_operation_id;
        // Zero is left for notifications that carry no operation identifier.
        self.next_operation_id = self.next_operation_id.wrapping_add(1).max(1);
        operation_id
    }
}

impl Default for TextInputClient {
    fn default() -> Self {
        Self::new(TextInputConfig::default())
    }
}

impl core::fmt::Debug for TextInputClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextInputClient")
            .field("config", &self.config)
            .field("server_version", &self.server_version)
            .field("focused_host", &self.focused_host)
            .field("focus", &self.focus)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

/// Client processor for [`SERVER_TO_CLIENT_CHANNEL_NAME`].
#[derive(Debug)]
pub struct ServerToClientChannel {
    client: SharedTextInputClient,
}

impl ServerToClientChannel {
    pub fn new(client: SharedTextInputClient) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &SharedTextInputClient {
        &self.client
    }
}

impl_as_any!(ServerToClientChannel);

impl DvcProcessor for ServerToClientChannel {
    fn channel_name(&self) -> &str {
        SERVER_TO_CLIENT_CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_MESSAGE_SIZE)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        lock(&self.client).reset();
        debug!(channel_id, "Text input server-to-client channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        process_frames(
            payload,
            &mut *lock(&self.client),
            TextInputClient::layout,
            ServerPdu::decode_body,
            TextInputClient::process,
        );
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        lock(&self.client).reset();
        debug!("Text input server-to-client channel closed");
    }
}

impl DvcClientProcessor for ServerToClientChannel {}

/// Client processor for [`CLIENT_TO_SERVER_CHANNEL_NAME`].
#[derive(Debug)]
pub struct ClientToServerChannel {
    client: SharedTextInputClient,
    channel_id: Option<u32>,
}

impl ClientToServerChannel {
    pub fn new(client: SharedTextInputClient) -> Self {
        Self {
            client,
            channel_id: None,
        }
    }

    pub fn client(&self) -> &SharedTextInputClient {
        &self.client
    }

    /// Encodes the PDUs queued for the server, or nothing while the channel is closed.
    pub fn pending_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        let Some(channel_id) = self.channel_id else {
            return Ok(Vec::new());
        };
        let messages = take_dvc_messages(&self.client);
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|error| encode_err!(error))
    }
}

impl_as_any!(ClientToServerChannel);

impl DvcProcessor for ClientToServerChannel {
    fn channel_name(&self) -> &str {
        CLIENT_TO_SERVER_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        debug!(channel_id, "Text input client-to-server channel started");
        // PDUs queued before the channel opened, such as the version reply, leave now.
        Ok(take_dvc_messages(&self.client))
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        warn!(
            channel_id,
            size = payload.len(),
            "Ignoring data received on the text input client-to-server channel"
        );
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        // Replies queued for this channel answer notifications of a connection that ended.
        lock(&self.client).take_pending();
        debug!("Text input client-to-server channel closed");
    }
}

impl DvcClientProcessor for ClientToServerChannel {}

fn take_dvc_messages(client: &Mutex<TextInputClient>) -> Vec<DvcMessage> {
    let mut client = lock(client);
    let layout = client.layout();
    versioned_messages(client.take_pending(), layout)
}
