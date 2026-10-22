use std::collections::VecDeque;

use ironrdp_core::{AsAny, decode};
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::{PduError, PduErrorExt as _, PduErrorKind, PduResult, decode_err};
use ironrdp_rail::pdu::{
    ActivatePdu, ClientStatusPdu, ClientSystemParameter, ClientSystemParametersPdu, CloakPdu, CompartmentInfoPdu,
    ExecutePdu, ExecuteResultPdu, GetApplicationIdRequestPdu, GetApplicationIdResponseExPdu,
    GetApplicationIdResponsePdu, HandshakeExPdu, HandshakePdu, LanguageBarInfoPdu, LocalMoveSizePdu, MinMaxInfoPdu,
    NotifyEventPdu, PowerDisplayRequestPdu, RailPdu, ServerSystemParametersPdu, SnapArrangePdu, SystemCommandPdu,
    SystemMenuPdu, SystemParameterRectangle, TaskbarInfoPdu, WindowMovePdu, ZOrderSyncPdu,
};
use ironrdp_svc::{ChannelFlags, SvcClientProcessor, SvcMessage, SvcProcessor, SvcProcessorMessages};

/// Client-originated RAIL messages a host forwards to the server ([MS-RDPERP] 2.2.2).
///
/// System parameter updates go through [`RailClient::update_system_parameters`]
/// because the server's HandshakeEx flags decide which of them may be sent.
///
/// [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailInputEvent {
    Activate(ActivatePdu),
    SystemMenu(SystemMenuPdu),
    SystemCommand(SystemCommandPdu),
    NotifyEvent(NotifyEventPdu),
    GetApplicationId(GetApplicationIdRequestPdu),
    /// MS-RDPERP 2.2.2.7.4 Client Window Move PDU (TS_RAIL_ORDER_WINDOWMOVE): the final
    /// window rectangle after a local move or resize.
    WindowMove(WindowMovePdu),
    /// MS-RDPERP 2.2.2.7.5 Client Window Snap PDU (TS_RAIL_ORDER_SNAP_ARRANGE): a window
    /// repositioned by the local window manager's Snap feature.
    ///
    /// Sent as a Client Window Move PDU when the server did not advertise
    /// `TS_RAIL_ORDER_HANDSHAKE_EX_FLAGS_SNAP_ARRANGE_SUPPORTED` (MS-RDPERP 3.2.5.2.7.5).
    SnapArrange(SnapArrangePdu),
    /// MS-RDPERP 2.2.2.12.1 Window Cloak State Change PDU (TS_RAIL_ORDER_CLOAK): the local
    /// shell cloaked or uncloaked a window.
    Cloak(CloakPdu),
}

impl From<RailInputEvent> for RailPdu {
    fn from(event: RailInputEvent) -> Self {
        match event {
            RailInputEvent::Activate(pdu) => Self::Activate(pdu),
            RailInputEvent::SystemMenu(pdu) => Self::SystemMenu(pdu),
            RailInputEvent::SystemCommand(pdu) => Self::SystemCommand(pdu),
            RailInputEvent::NotifyEvent(pdu) => Self::NotifyEvent(pdu),
            RailInputEvent::GetApplicationId(pdu) => Self::GetApplicationIdRequest(pdu),
            RailInputEvent::WindowMove(pdu) => Self::WindowMove(pdu),
            RailInputEvent::SnapArrange(pdu) => Self::SnapArrange(pdu),
            RailInputEvent::Cloak(pdu) => Self::Cloak(pdu),
        }
    }
}

/// Portable server-originated RAIL controls ([MS-RDPERP] 2.2.2).
///
/// [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailControlEvent {
    SystemParameters(ServerSystemParametersPdu),
    LanguageBar(LanguageBarInfoPdu),
    Compartment(CompartmentInfoPdu),
    ZOrderSync(ZOrderSyncPdu),
    Cloak(CloakPdu),
    PowerDisplayRequest(PowerDisplayRequestPdu),
    /// MS-RDPERP 2.2.2.7.1 Server Min Max Info PDU (TS_RAIL_ORDER_MINMAXINFO): window sizing
    /// constraints for an upcoming local move or resize.
    MinMaxInfo(MinMaxInfoPdu),
    /// MS-RDPERP 2.2.2.7.2 Server Move/Size Start PDU and 2.2.2.7.3 Server Move/Size End PDU
    /// (TS_RAIL_ORDER_LOCALMOVESIZE): start or end of a server-initiated local move or resize.
    LocalMoveSize(LocalMoveSizePdu),
    /// MS-RDPERP 2.2.2.14.1 Taskbar Tab Info PDU (TS_RAIL_ORDER_TASKBARINFO): a taskbar tab
    /// registration or update.
    TaskbarInfo(TaskbarInfoPdu),
}

/// Events emitted by a client-side RAIL channel processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailEvent {
    Handshake {
        handshake_ex_flags: Option<u32>,
        initialization_message_count: usize,
        queued_execute_count: usize,
    },
    DesktopSynchronized {
        released_execute_count: usize,
    },
    PostHandshakeQueueReleased {
        released_execute_count: usize,
    },
    ExecuteResult(ExecuteResultPdu),
    ApplicationId {
        window_id: u32,
        application_id: String,
        process_id: Option<u32>,
        process_image_name: Option<String>,
    },
    Control(RailControlEvent),
}

/// PDUs emitted by [`RailClient`].
pub type RailSvcMessages = SvcProcessorMessages<RailClient>;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RailState {
    WaitingForHandshake,
    Established,
}

#[derive(Debug)]
enum QueuedRailPdu {
    Execute(ExecutePdu),
    Input(RailInputEvent),
}

#[derive(Debug, PartialEq, Eq)]
struct PendingExecute {
    flags: u16,
    executable: String,
}

/// Client-side processor for the RAIL static virtual channel.
#[derive(Debug)]
pub struct RailClient {
    build_number: u32,
    desktop_width: u16,
    desktop_height: u16,
    client_status_flags: u32,
    /// The host's system parameters, one value per kind.
    ///
    /// INVARIANT: never contains `ClientSystemParameter::DisplayChange`, which always
    /// follows the desktop size (see [`Self::desktop_system_parameter_messages`]).
    system_parameters: Vec<ClientSystemParameter>,
    /// The server's HandshakeEx flags, or `0` after a plain Handshake.
    handshake_ex_flags: u32,
    state: RailState,
    desktop_synchronized: bool,
    queued_pdus: VecDeque<QueuedRailPdu>,
    pending_executes: VecDeque<PendingExecute>,
    events: VecDeque<RailEvent>,
}

impl RailClient {
    const CHANNEL_NAME: ChannelName = ChannelName::from_static(b"rail\0\0\0\0");
    const MAX_QUEUED_PDUS: usize = 128;
    const MAX_PENDING_EXECUTES: usize = 64;
    const MAX_EVENTS: usize = 256;

    /// Creates a RAIL static-channel processor for the client's build and desktop size.
    pub fn new(build_number: u32, desktop_width: u16, desktop_height: u16) -> Self {
        Self {
            build_number,
            desktop_width,
            desktop_height,
            client_status_flags: ClientStatusPdu::Z_ORDER_SYNC
                | ClientStatusPdu::POWER_DISPLAY_REQUEST
                | ClientStatusPdu::BIDIRECTIONAL_CLOAK,
            system_parameters: vec![
                ClientSystemParameter::FullWindowDrag(false),
                ClientSystemParameter::KeyboardCues(false),
                ClientSystemParameter::KeyboardPreference(false),
                ClientSystemParameter::MouseButtonSwap(false),
            ],
            handshake_ex_flags: 0,
            state: RailState::WaitingForHandshake,
            desktop_synchronized: false,
            queued_pdus: VecDeque::new(),
            pending_executes: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    /// Overrides the RAIL Client Status flags advertised during initialization.
    #[must_use]
    pub fn with_client_status_flags(mut self, flags: u32) -> Self {
        self.client_status_flags = flags;
        self
    }

    /// Replaces the system parameters sent during initialization with the host's own.
    ///
    /// `DisplayChange` always follows the desktop size, so a `DisplayChange` in
    /// `parameters` is discarded; use [`Self::update_desktop_size`] instead. `WorkArea`
    /// also follows the desktop size unless the host supplies one. Parameters the server
    /// does not advertise support for in its HandshakeEx PDU are not sent.
    ///
    /// Fails if a parameter cannot be encoded.
    pub fn with_system_parameters(mut self, parameters: Vec<ClientSystemParameter>) -> PduResult<Self> {
        Self::validate_system_parameters(&parameters)?;
        self.system_parameters = Vec::new();
        self.merge_system_parameters(parameters);
        Ok(self)
    }

    /// Records changed local system parameters and returns the messages that report them.
    ///
    /// Before the server handshake the values are kept for initialization. A
    /// `DisplayChange` is discarded rather than recorded or sent, because the display
    /// rectangle follows the desktop size; use [`Self::update_desktop_size`] instead.
    ///
    /// Fails, recording nothing, if a parameter cannot be encoded.
    pub fn update_system_parameters(&mut self, parameters: Vec<ClientSystemParameter>) -> PduResult<RailSvcMessages> {
        Self::validate_system_parameters(&parameters)?;
        let changed = self.merge_system_parameters(parameters);
        if !self.is_established() {
            return Ok(Vec::new().into());
        }

        Ok(changed
            .into_iter()
            .filter(|parameter| self.is_system_parameter_supported(parameter))
            .map(Self::system_parameter_message)
            .collect::<Vec<_>>()
            .into())
    }

    /// Queues a RemoteApp launch until initialization and desktop synchronization complete.
    pub fn queue_execute(&mut self, execute: ExecutePdu) -> PduResult<RailSvcMessages> {
        self.queue_pdu(QueuedRailPdu::Execute(execute))
    }

    /// Queues a client-originated RAIL input event.
    pub fn queue_input(&mut self, event: RailInputEvent) -> PduResult<RailSvcMessages> {
        self.queue_pdu(QueuedRailPdu::Input(event))
    }

    /// Releases queued RAIL input after the server completes desktop synchronization.
    pub fn complete_desktop_synchronization(&mut self) -> PduResult<RailSvcMessages> {
        if self.desktop_synchronized {
            return Ok(Vec::new().into());
        }

        self.desktop_synchronized = true;
        if !self.is_established() {
            return Ok(Vec::new().into());
        }

        let (messages, released_execute_count) = self.drain_queued_pdus()?;
        self.push_event(RailEvent::DesktopSynchronized { released_execute_count })?;
        Ok(messages.into())
    }

    /// Releases queued input after initialization when desktop synchronization is not observable.
    pub fn release_queued_after_handshake(&mut self) -> PduResult<RailSvcMessages> {
        if self.desktop_synchronized || !self.is_established() {
            return Ok(Vec::new().into());
        }

        self.desktop_synchronized = true;
        let (messages, released_execute_count) = self.drain_queued_pdus()?;
        self.push_event(RailEvent::PostHandshakeQueueReleased { released_execute_count })?;
        Ok(messages.into())
    }

    /// Updates the RAIL work area after the client desktop size changes.
    pub fn update_desktop_size(&mut self, desktop_width: u16, desktop_height: u16) -> RailSvcMessages {
        self.desktop_width = desktop_width;
        self.desktop_height = desktop_height;
        if !self.is_established() {
            return Vec::new().into();
        }

        self.desktop_system_parameter_messages().into()
    }

    /// Returns and clears server events accumulated while processing channel PDUs.
    pub fn take_events(&mut self) -> Vec<RailEvent> {
        self.events.drain(..).collect()
    }

    const fn is_established(&self) -> bool {
        matches!(self.state, RailState::Established)
    }

    fn queue_pdu(&mut self, pdu: QueuedRailPdu) -> PduResult<RailSvcMessages> {
        Self::validate_queued_pdu(&pdu)?;
        if !self.is_established() || !self.desktop_synchronized {
            if self.queued_pdus.len() >= Self::MAX_QUEUED_PDUS {
                return Err(Self::resource_limit_error("queued RAIL PDUs"));
            }
            if matches!(&pdu, QueuedRailPdu::Execute(_))
                && self.pending_executes.len() + self.queued_execute_count() >= Self::MAX_PENDING_EXECUTES
            {
                return Err(Self::resource_limit_error("RAIL execute requests"));
            }
            self.queued_pdus.push_back(pdu);
            return Ok(Vec::new().into());
        }

        Ok(self.send_pdu(pdu)?.into())
    }

    fn initialization_messages(&self) -> Vec<SvcMessage> {
        let mut messages = vec![
            Self::service_message(RailPdu::Handshake(HandshakePdu {
                build_number: self.build_number,
            })),
            Self::service_message(RailPdu::ClientStatus(ClientStatusPdu {
                flags: self.client_status_flags,
            })),
        ];
        messages.extend(self.desktop_system_parameter_messages());
        messages.extend(
            self.system_parameters
                .iter()
                .filter(|parameter| self.is_system_parameter_supported(parameter))
                .cloned()
                .map(Self::system_parameter_message),
        );
        messages
    }

    fn desktop_system_parameter_messages(&self) -> Vec<SvcMessage> {
        let screen = SystemParameterRectangle {
            left: 0,
            top: 0,
            right: self.desktop_width,
            bottom: self.desktop_height,
        };
        let has_host_work_area = self
            .system_parameters
            .iter()
            .any(|parameter| matches!(parameter, ClientSystemParameter::WorkArea(_)));
        [
            Some(ClientSystemParameter::DisplayChange(screen)),
            (!has_host_work_area).then_some(ClientSystemParameter::WorkArea(screen)),
        ]
        .into_iter()
        .flatten()
        .map(Self::system_parameter_message)
        .collect()
    }

    fn validate_system_parameters(parameters: &[ClientSystemParameter]) -> PduResult<()> {
        for parameter in parameters {
            RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: parameter.clone(),
            })
            .validate()
            .map_err(|error| PduError::encode("RAIL client system parameter", error))?;
        }
        Ok(())
    }

    /// Stores `parameters`, replacing earlier values of the same kind, and returns
    /// the ones whose value changed.
    fn merge_system_parameters(&mut self, parameters: Vec<ClientSystemParameter>) -> Vec<ClientSystemParameter> {
        let mut changed = Vec::new();
        for parameter in parameters {
            if matches!(parameter, ClientSystemParameter::DisplayChange(_)) {
                // Keeps the `system_parameters` invariant: the display rectangle follows the desktop size.
                continue;
            }
            let kind = core::mem::discriminant(&parameter);
            match self
                .system_parameters
                .iter_mut()
                .find(|existing| core::mem::discriminant(&**existing) == kind)
            {
                Some(existing) if *existing == parameter => continue,
                Some(existing) => *existing = parameter.clone(),
                None => self.system_parameters.push(parameter.clone()),
            }
            changed.push(parameter);
        }
        changed
    }

    /// Whether the server advertised support for `parameter`, per MS-RDPERP 2.2.2.4.1
    /// Client System Parameters Update PDU (TS_RAIL_ORDER_SYSPARAM).
    ///
    /// The parameters the specification lists without a HandshakeEx condition are always
    /// supported; the others require `EXTENDED_SPI`, `EXTENDED_SPI_2` or `EXTENDED_SPI_3`.
    fn is_system_parameter_supported(&self, parameter: &ClientSystemParameter) -> bool {
        use ClientSystemParameter as P;

        let required = match parameter {
            P::FullWindowDrag(_)
            | P::KeyboardCues(_)
            | P::KeyboardPreference(_)
            | P::WorkArea(_)
            | P::DisplayChange(_)
            | P::MouseButtonSwap(_)
            | P::TaskbarPosition(_)
            | P::HighContrast(_) => return true,
            P::CaretWidth(_) | P::StickyKeys(_) | P::ToggleKeys(_) | P::FilterKeys(_) => HandshakeExPdu::EXTENDED_SPI,
            P::DisplayAnimationsEnabled(_)
            | P::DisplayAdvancedEffectsEnabled(_)
            | P::DisplayAutoHideScrollbars(_)
            | P::DisplayMessageDuration(_)
            | P::ClosedCaptionFontColor(_)
            | P::ClosedCaptionFontOpacity(_)
            | P::ClosedCaptionFontSize(_)
            | P::ClosedCaptionFontStyle(_)
            | P::ClosedCaptionFontEdgeEffect(_)
            | P::ClosedCaptionBackgroundColor(_)
            | P::ClosedCaptionBackgroundOpacity(_)
            | P::ClosedCaptionRegionColor(_)
            | P::ClosedCaptionRegionOpacity(_) => HandshakeExPdu::EXTENDED_SPI_2,
            P::AccentColor(_) | P::SystemUsesLightTheme(_) | P::AppsUseLightTheme(_) => HandshakeExPdu::EXTENDED_SPI_3,
        };
        self.handshake_ex_flags & required != 0
    }

    fn queued_execute_count(&self) -> usize {
        self.queued_pdus
            .iter()
            .filter(|pdu| matches!(pdu, QueuedRailPdu::Execute(_)))
            .count()
    }

    fn finish_handshake(&mut self, handshake_ex_flags: Option<u32>) -> PduResult<Vec<SvcMessage>> {
        self.state = RailState::Established;
        self.handshake_ex_flags = handshake_ex_flags.unwrap_or(0);
        let queued_execute_count = self.queued_execute_count();
        let mut messages = self.initialization_messages();
        self.push_event(RailEvent::Handshake {
            handshake_ex_flags,
            initialization_message_count: messages.len(),
            queued_execute_count,
        })?;
        if self.desktop_synchronized {
            let (queued_messages, released_execute_count) = self.drain_queued_pdus()?;
            messages.extend(queued_messages);
            self.push_event(RailEvent::DesktopSynchronized { released_execute_count })?;
        }
        Ok(messages)
    }

    fn send_pdu(&mut self, pdu: QueuedRailPdu) -> PduResult<Vec<SvcMessage>> {
        match pdu {
            QueuedRailPdu::Execute(execute) => {
                if self.pending_executes.len() >= Self::MAX_PENDING_EXECUTES {
                    return Err(Self::resource_limit_error("pending RAIL execute requests"));
                }
                self.pending_executes.push_back(PendingExecute {
                    flags: execute.flags,
                    executable: execute.executable.clone(),
                });
                Ok(vec![Self::service_message(RailPdu::Execute(execute))])
            }
            QueuedRailPdu::Input(RailInputEvent::SnapArrange(snap))
                if self.handshake_ex_flags & HandshakeExPdu::SNAP_ARRANGE == 0 =>
            {
                // MS-RDPERP 3.2.5.2.7.5: without server Snap support, report the move instead.
                Ok(vec![Self::service_message(RailPdu::WindowMove(WindowMovePdu {
                    window_id: snap.window_id,
                    rectangle: snap.rectangle,
                }))])
            }
            QueuedRailPdu::Input(event) => Ok(vec![Self::service_message(RailPdu::from(event))]),
        }
    }

    fn drain_queued_pdus(&mut self) -> PduResult<(Vec<SvcMessage>, usize)> {
        let mut messages = Vec::new();
        let mut released_execute_count = 0;
        while let Some(pdu) = self.queued_pdus.pop_front() {
            if matches!(pdu, QueuedRailPdu::Execute(_)) {
                released_execute_count += 1;
            }
            messages.extend(self.send_pdu(pdu)?);
        }
        Ok((messages, released_execute_count))
    }

    fn push_event(&mut self, event: RailEvent) -> PduResult<()> {
        if self.events.len() >= Self::MAX_EVENTS {
            return Err(Self::resource_limit_error("queued RAIL events"));
        }
        self.events.push_back(event);
        Ok(())
    }

    fn validate_queued_pdu(pdu: &QueuedRailPdu) -> PduResult<()> {
        let pdu = match pdu {
            QueuedRailPdu::Execute(execute) => RailPdu::Execute(execute.clone()),
            QueuedRailPdu::Input(event) => RailPdu::from(*event),
        };
        pdu.validate()
            .map_err(|error| PduError::encode("RAIL client request", error))
    }

    fn service_message(pdu: RailPdu) -> SvcMessage {
        SvcMessage::from(pdu).with_flags(ChannelFlags::SHOW_PROTOCOL)
    }

    fn system_parameter_message(parameter: ClientSystemParameter) -> SvcMessage {
        Self::service_message(RailPdu::ClientSystemParameters(ClientSystemParametersPdu { parameter }))
    }

    fn resource_limit_error(description: &'static str) -> PduError {
        PduError::new("RAIL", PduErrorKind::Other { description })
    }
}

impl SvcClientProcessor for RailClient {}

impl AsAny for RailClient {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl SvcProcessor for RailClient {
    fn channel_name(&self) -> ChannelName {
        Self::CHANNEL_NAME
    }

    fn channel_options(&self) -> ChannelOptions {
        ChannelOptions::INITIALIZED | ChannelOptions::SHOW_PROTOCOL
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode::<RailPdu>(payload).map_err(|error| decode_err!(error))?;
        if self.is_established() && !pdu.is_server_to_client() {
            return Err(PduError::new(
                "RAIL",
                PduErrorKind::Other {
                    description: "received client-originated RAIL PDU from server",
                },
            ));
        }

        match (self.state, pdu) {
            (RailState::WaitingForHandshake, RailPdu::Handshake(HandshakePdu { .. })) => self.finish_handshake(None),
            (RailState::WaitingForHandshake, RailPdu::HandshakeEx(HandshakeExPdu { flags, .. })) => {
                self.finish_handshake(Some(flags))
            }
            (RailState::WaitingForHandshake, _) => Err(PduError::new(
                "RAIL",
                PduErrorKind::Other {
                    description: "received RAIL PDU before server handshake",
                },
            )),
            (RailState::Established, RailPdu::Handshake(_) | RailPdu::HandshakeEx(_)) => Err(PduError::new(
                "RAIL",
                PduErrorKind::Other {
                    description: "received duplicate RAIL server handshake",
                },
            )),
            (RailState::Established, RailPdu::ExecuteResult(result)) => {
                let Some(index) = self
                    .pending_executes
                    .iter()
                    .position(|request| request.flags == result.flags && request.executable == result.executable)
                else {
                    return Err(PduError::new(
                        "RAIL",
                        PduErrorKind::Other {
                            description: "received unmatched RAIL execute result",
                        },
                    ));
                };
                self.pending_executes.remove(index);
                self.push_event(RailEvent::ExecuteResult(result))?;
                Ok(Vec::new())
            }
            (
                RailState::Established,
                RailPdu::GetApplicationIdResponse(GetApplicationIdResponsePdu {
                    window_id,
                    application_id,
                }),
            ) => {
                self.push_event(RailEvent::ApplicationId {
                    window_id,
                    application_id,
                    process_id: None,
                    process_image_name: None,
                })?;
                Ok(Vec::new())
            }
            (
                RailState::Established,
                RailPdu::GetApplicationIdResponseEx(GetApplicationIdResponseExPdu {
                    window_id,
                    application_id,
                    process_id,
                    process_image_name,
                }),
            ) => {
                self.push_event(RailEvent::ApplicationId {
                    window_id,
                    application_id,
                    process_id: Some(process_id),
                    process_image_name: Some(process_image_name),
                })?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::ServerSystemParameters(parameters)) => {
                self.push_event(RailEvent::Control(RailControlEvent::SystemParameters(parameters)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::LanguageBarInfo(info)) => {
                self.push_event(RailEvent::Control(RailControlEvent::LanguageBar(info)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::CompartmentInfo(info)) => {
                self.push_event(RailEvent::Control(RailControlEvent::Compartment(info)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::ZOrderSync(sync)) => {
                self.push_event(RailEvent::Control(RailControlEvent::ZOrderSync(sync)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::Cloak(cloak)) => {
                self.push_event(RailEvent::Control(RailControlEvent::Cloak(cloak)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::PowerDisplayRequest(request)) => {
                self.push_event(RailEvent::Control(RailControlEvent::PowerDisplayRequest(request)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::MinMaxInfo(info)) => {
                self.push_event(RailEvent::Control(RailControlEvent::MinMaxInfo(info)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::LocalMoveSize(move_size)) => {
                self.push_event(RailEvent::Control(RailControlEvent::LocalMoveSize(move_size)))?;
                Ok(Vec::new())
            }
            (RailState::Established, RailPdu::TaskbarInfo(info)) => {
                self.push_event(RailEvent::Control(RailControlEvent::TaskbarInfo(info)))?;
                Ok(Vec::new())
            }
            (RailState::Established, _) => Ok(Vec::new()),
        }
    }
}
