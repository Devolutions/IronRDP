use std::collections::VecDeque;

use ironrdp_core::{AsAny, decode};
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::{PduError, PduErrorExt as _, PduErrorKind, PduResult, decode_err};
use ironrdp_rail::pdu::{
    ActivatePdu, ClientStatusPdu, ClientSystemParameter, ClientSystemParametersPdu, CloakPdu, CompartmentInfoPdu,
    ExecutePdu, ExecuteResultPdu, GetApplicationIdRequestPdu, GetApplicationIdResponseExPdu,
    GetApplicationIdResponsePdu, HandshakeExPdu, HandshakePdu, LanguageBarInfoPdu, NotifyEventPdu,
    PowerDisplayRequestPdu, RailPdu, ServerSystemParametersPdu, SystemCommandPdu, SystemMenuPdu,
    SystemParameterRectangle, ZOrderSyncPdu,
};
use ironrdp_svc::{ChannelFlags, SvcClientProcessor, SvcMessage, SvcProcessor, SvcProcessorMessages};

/// Client-originated RAIL events used by the initial RemoteApp launch flow.
///
/// Other client-valid RAIL PDUs, including window move and IME controls, are
/// not yet exposed through this host-neutral boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailInputEvent {
    Activate(ActivatePdu),
    SystemMenu(SystemMenuPdu),
    SystemCommand(SystemCommandPdu),
    NotifyEvent(NotifyEventPdu),
    GetApplicationId(GetApplicationIdRequestPdu),
}

impl From<RailInputEvent> for RailPdu {
    fn from(event: RailInputEvent) -> Self {
        match event {
            RailInputEvent::Activate(pdu) => Self::Activate(pdu),
            RailInputEvent::SystemMenu(pdu) => Self::SystemMenu(pdu),
            RailInputEvent::SystemCommand(pdu) => Self::SystemCommand(pdu),
            RailInputEvent::NotifyEvent(pdu) => Self::NotifyEvent(pdu),
            RailInputEvent::GetApplicationId(pdu) => Self::GetApplicationIdRequest(pdu),
        }
    }
}

/// Portable server-originated RAIL controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailControlEvent {
    SystemParameters(ServerSystemParametersPdu),
    LanguageBar(LanguageBarInfoPdu),
    Compartment(CompartmentInfoPdu),
    ZOrderSync(ZOrderSyncPdu),
    Cloak(CloakPdu),
    PowerDisplayRequest(PowerDisplayRequestPdu),
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
        messages.extend([
            Self::service_message(RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::FullWindowDrag(false),
            })),
            Self::service_message(RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::KeyboardCues(false),
            })),
            Self::service_message(RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::KeyboardPreference(false),
            })),
            Self::service_message(RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::MouseButtonSwap(false),
            })),
        ]);
        messages
    }

    fn desktop_system_parameter_messages(&self) -> Vec<SvcMessage> {
        let screen = SystemParameterRectangle {
            left: 0,
            top: 0,
            right: self.desktop_width,
            bottom: self.desktop_height,
        };
        [
            ClientSystemParameter::DisplayChange(screen),
            ClientSystemParameter::WorkArea(screen),
        ]
        .into_iter()
        .map(|parameter| {
            Self::service_message(RailPdu::ClientSystemParameters(ClientSystemParametersPdu { parameter }))
        })
        .collect()
    }

    fn queued_execute_count(&self) -> usize {
        self.queued_pdus
            .iter()
            .filter(|pdu| matches!(pdu, QueuedRailPdu::Execute(_)))
            .count()
    }

    fn finish_handshake(&mut self, handshake_ex_flags: Option<u32>) -> PduResult<Vec<SvcMessage>> {
        self.state = RailState::Established;
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
            (RailState::Established, _) => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};
    use ironrdp_rail::pdu::{
        ClientStatusPdu, ClientSystemParameter, ClientSystemParametersPdu, ExecutePdu, HandshakePdu, RailPdu,
        SystemParameterRectangle,
    };
    use ironrdp_svc::SvcProcessor as _;

    use super::{RailClient, RailEvent};

    #[test]
    fn queued_execute_is_released_after_handshake_fallback() {
        let mut client = RailClient::new(1, 1024, 768);
        client
            .queue_execute(ExecutePdu {
                flags: 0,
                executable: "app".to_owned(),
                working_directory: String::new(),
                arguments: String::new(),
            })
            .unwrap();
        client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();

        let messages: Vec<_> = client.release_queued_after_handshake().unwrap().into();
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            decode::<RailPdu>(&messages[0].encode_unframed_pdu().unwrap()).unwrap(),
            RailPdu::Execute(_)
        ));
        assert!(matches!(
            client.take_events().as_slice(),
            [
                RailEvent::Handshake {
                    queued_execute_count: 1,
                    ..
                },
                RailEvent::PostHandshakeQueueReleased {
                    released_execute_count: 1
                }
            ]
        ));
    }

    #[test]
    fn server_cannot_send_client_input() {
        let mut client = RailClient::new(1, 1, 1);
        client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();

        assert!(
            client
                .process(
                    &encode_vec(&RailPdu::Execute(ExecutePdu {
                        flags: 0,
                        executable: "app".to_owned(),
                        working_directory: String::new(),
                        arguments: String::new(),
                    }))
                    .unwrap()
                )
                .is_err()
        );
    }

    #[test]
    fn handshake_advertises_supported_server_controls() {
        let mut client = RailClient::new(1, 1, 1);
        let messages = client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();

        assert!(matches!(
                decode::<RailPdu>(&messages[1].encode_unframed_pdu().unwrap()).unwrap(),
                RailPdu::ClientStatus(ClientStatusPdu { flags })
                    if flags
                        == ClientStatusPdu::Z_ORDER_SYNC
                            | ClientStatusPdu::POWER_DISPLAY_REQUEST
                            | ClientStatusPdu::BIDIRECTIONAL_CLOAK
        ));
    }

    #[test]
    fn handshake_advertises_configured_client_status_flags() {
        let mut client = RailClient::new(1, 1, 1).with_client_status_flags(0);
        let messages = client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();

        assert!(matches!(
            decode::<RailPdu>(&messages[1].encode_unframed_pdu().unwrap()).unwrap(),
            RailPdu::ClientStatus(ClientStatusPdu { flags: 0 })
        ));
    }

    #[test]
    fn desktop_synchronization_releases_queued_execute() {
        let mut client = RailClient::new(1, 1, 1);
        client
            .queue_execute(ExecutePdu {
                flags: 0,
                executable: "app".to_owned(),
                working_directory: String::new(),
                arguments: String::new(),
            })
            .unwrap();
        client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();

        let messages: Vec<_> = client.complete_desktop_synchronization().unwrap().into();

        assert!(matches!(
            decode::<RailPdu>(&messages[0].encode_unframed_pdu().unwrap()).unwrap(),
            RailPdu::Execute(_)
        ));
        assert!(matches!(
            client.take_events().as_slice(),
            [
                RailEvent::Handshake { .. },
                RailEvent::DesktopSynchronized {
                    released_execute_count: 1
                }
            ]
        ));
    }

    #[test]
    fn queued_executes_do_not_exceed_pending_limit() {
        let mut client = RailClient::new(1, 1, 1);
        for _ in 0..RailClient::MAX_PENDING_EXECUTES {
            client
                .queue_execute(ExecutePdu {
                    flags: 0,
                    executable: "app".to_owned(),
                    working_directory: String::new(),
                    arguments: String::new(),
                })
                .unwrap();
        }

        assert!(
            client
                .queue_execute(ExecutePdu {
                    flags: 0,
                    executable: "app".to_owned(),
                    working_directory: String::new(),
                    arguments: String::new(),
                })
                .is_err()
        );

        client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();
        let messages: Vec<_> = client.complete_desktop_synchronization().unwrap().into();

        assert_eq!(messages.len(), RailClient::MAX_PENDING_EXECUTES);
    }

    #[test]
    fn desktop_size_update_sends_display_change_and_work_area() {
        let mut client = RailClient::new(1, 1, 1);
        client
            .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
            .unwrap();

        let messages: Vec<_> = client.update_desktop_size(1024, 768).into();

        assert!(matches!(
            decode::<RailPdu>(&messages[0].encode_unframed_pdu().unwrap()).unwrap(),
            RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::DisplayChange(SystemParameterRectangle {
                    right: 1024,
                    bottom: 768,
                    ..
                })
            })
        ));
        assert!(matches!(
            decode::<RailPdu>(&messages[1].encode_unframed_pdu().unwrap()).unwrap(),
            RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::WorkArea(SystemParameterRectangle {
                    right: 1024,
                    bottom: 768,
                    ..
                })
            })
        ));
    }
}
