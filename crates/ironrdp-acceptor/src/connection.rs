use ironrdp_connector::{
    legacy, reason_err, ConnectorError, ConnectorErrorExt, ConnectorResult, DesktopSize, Sequence, State, Written,
};
use ironrdp_pdu as pdu;
use pdu::rdp::capability_sets::CapabilitySet;
use pdu::rdp::headers::ShareControlPdu;
use pdu::write_buf::WriteBuf;
use pdu::{gcc, mcs, nego, rdp, PduParsing};

use super::channel_connection::ChannelConnectionSequence;
use super::finalization::FinalizationSequence;
use crate::util::{self, wrap_share_data};

const IO_CHANNEL_ID: u16 = 1003;
const USER_CHANNEL_ID: u16 = 1002;

pub struct Acceptor {
    state: AcceptorState,
    security: nego::SecurityProtocol,
    io_channel_id: u16,
    user_channel_id: u16,
    desktop_size: DesktopSize,
    server_capabilities: Vec<CapabilitySet>,
}

#[derive(Debug, Clone)]
pub struct AcceptorResult {
    pub channels: Vec<(u16, gcc::ChannelDef)>,
    pub capabilities: Vec<CapabilitySet>,
    pub input_events: Vec<Vec<u8>>,
}

impl Acceptor {
    pub fn new(security: nego::SecurityProtocol, desktop_size: DesktopSize, capabilities: Vec<CapabilitySet>) -> Self {
        Self {
            security,
            state: AcceptorState::InitiationWaitRequest,
            user_channel_id: USER_CHANNEL_ID,
            io_channel_id: IO_CHANNEL_ID,
            desktop_size,
            server_capabilities: capabilities,
        }
    }

    pub fn reached_security_upgrade(&self) -> Option<nego::SecurityProtocol> {
        match self.state {
            AcceptorState::SecurityUpgrade { .. } => Some(self.security),
            _ => None,
        }
    }

    pub fn get_result(&mut self) -> Option<AcceptorResult> {
        match std::mem::take(&mut self.state) {
            AcceptorState::Accepted {
                channels,
                client_capabilities,
                input_events,
            } => Some(AcceptorResult {
                channels,
                capabilities: client_capabilities,
                input_events,
            }),
            previous_state => {
                self.state = previous_state;
                None
            }
        }
    }
}

#[derive(Default, Debug)]
pub enum AcceptorState {
    #[default]
    Consumed,

    InitiationWaitRequest,
    InitiationSendConfirm {
        requested_protocol: nego::SecurityProtocol,
    },
    SecurityUpgrade {
        requested_protocol: nego::SecurityProtocol,
    },
    BasicSettingsWaitInitial {
        requested_protocol: nego::SecurityProtocol,
    },
    BasicSettingsSendResponse {
        requested_protocol: nego::SecurityProtocol,
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    ChannelConnection {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
        connection: ChannelConnectionSequence,
    },
    RdpSecurityCommencement {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    SecureSettingsExchange {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    LicensingExchange {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    CapabilitiesSendServer {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    MonitorLayoutSend {
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    CapabilitiesWaitConfirm {
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    ConnectionFinalization {
        finalization: FinalizationSequence,
        channels: Vec<(u16, gcc::ChannelDef)>,
        client_capabilities: Vec<CapabilitySet>,
    },
    Accepted {
        channels: Vec<(u16, gcc::ChannelDef)>,
        client_capabilities: Vec<CapabilitySet>,
        input_events: Vec<Vec<u8>>,
    },
}

impl State for AcceptorState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::InitiationWaitRequest => "InitiationWaitRequest",
            Self::InitiationSendConfirm { .. } => "InitiationSendConfirm",
            Self::SecurityUpgrade { .. } => "SecurityUpgrade",
            Self::BasicSettingsWaitInitial { .. } => "BasicSettingsWaitInitial",
            Self::BasicSettingsSendResponse { .. } => "BasicSettingsSendResponse",
            Self::ChannelConnection { .. } => "ChannelConnection",
            Self::RdpSecurityCommencement { .. } => "RdpSecurityCommencement",
            Self::SecureSettingsExchange { .. } => "SecureSettingsExchange",
            Self::LicensingExchange { .. } => "LicensingExchange",
            Self::CapabilitiesSendServer { .. } => "CapabilitiesSendServer",
            Self::MonitorLayoutSend { .. } => "MonitorLayoutSend",
            Self::CapabilitiesWaitConfirm { .. } => "CapabilitiesWaitConfirm",
            Self::ConnectionFinalization { .. } => "ConnectionFinalization",
            Self::Accepted { .. } => "Connected",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Sequence for Acceptor {
    fn next_pdu_hint(&self) -> Option<&dyn pdu::PduHint> {
        match &self.state {
            AcceptorState::Consumed => None,
            AcceptorState::InitiationWaitRequest => Some(&pdu::X224_HINT),
            AcceptorState::InitiationSendConfirm { .. } => None,
            AcceptorState::SecurityUpgrade { .. } => None,
            AcceptorState::BasicSettingsWaitInitial { .. } => Some(&pdu::X224_HINT),
            AcceptorState::BasicSettingsSendResponse { .. } => None,
            AcceptorState::ChannelConnection { connection, .. } => connection.next_pdu_hint(),
            AcceptorState::RdpSecurityCommencement { .. } => None,
            AcceptorState::SecureSettingsExchange { .. } => Some(&pdu::X224_HINT),
            AcceptorState::LicensingExchange { .. } => None,
            AcceptorState::CapabilitiesSendServer { .. } => None,
            AcceptorState::MonitorLayoutSend { .. } => None,
            AcceptorState::CapabilitiesWaitConfirm { .. } => Some(&pdu::X224_HINT),
            AcceptorState::ConnectionFinalization { finalization, .. } => finalization.next_pdu_hint(),
            AcceptorState::Accepted { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match std::mem::take(&mut self.state) {
            AcceptorState::InitiationWaitRequest => {
                let connection_request =
                    ironrdp_pdu::decode::<nego::ConnectionRequest>(input).map_err(ConnectorError::pdu)?;

                debug!(message = ?connection_request, "Received");

                (
                    Written::Nothing,
                    AcceptorState::InitiationSendConfirm {
                        requested_protocol: connection_request.protocol,
                    },
                )
            }

            AcceptorState::InitiationSendConfirm { requested_protocol } => {
                let connection_confirm = nego::ConnectionConfirm::Response {
                    flags: nego::ResponseFlags::empty(),
                    protocol: self.security,
                };

                debug!(message = ?connection_confirm, "Send");

                let written = ironrdp_pdu::encode_buf(&connection_confirm, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::SecurityUpgrade { requested_protocol },
                )
            }

            AcceptorState::SecurityUpgrade { requested_protocol } => (
                Written::Nothing,
                AcceptorState::BasicSettingsWaitInitial { requested_protocol },
            ),

            AcceptorState::BasicSettingsWaitInitial { requested_protocol } => {
                let settings_initial = legacy::decode_x224_packet::<mcs::ConnectInitial>(input)?;

                debug!(message = ?settings_initial, "Received");

                let early_capability = settings_initial
                    .conference_create_request
                    .gcc_blocks
                    .core
                    .optional_data
                    .early_capability_flags;

                #[allow(clippy::arithmetic_side_effects)] // IO channel ID is not big enough for overflowing
                let channels = settings_initial
                    .conference_create_request
                    .gcc_blocks
                    .network
                    .map(|network| {
                        network
                            .channels
                            .into_iter()
                            .enumerate()
                            .map(|(i, c)| (u16::try_from(i).unwrap() + self.io_channel_id + 1, c))
                            .collect()
                    })
                    .unwrap_or_default();

                (
                    Written::Nothing,
                    AcceptorState::BasicSettingsSendResponse {
                        requested_protocol,
                        early_capability,
                        channels,
                    },
                )
            }

            AcceptorState::BasicSettingsSendResponse {
                requested_protocol,
                early_capability,
                channels,
            } => {
                let channel_ids: Vec<u16> = channels.iter().map(|&(i, _)| i).collect();
                let server_blocks = create_gcc_blocks(self.io_channel_id, channel_ids.clone(), requested_protocol);
                let settings_response = mcs::ConnectResponse {
                    conference_create_response: gcc::ConferenceCreateResponse {
                        user_id: self.user_channel_id,
                        gcc_blocks: server_blocks,
                    },
                    called_connect_id: 1,
                    domain_parameters: mcs::DomainParameters::target(),
                };

                debug!(message = ?settings_response, "Send");

                let written = legacy::encode_x224_packet(&settings_response, output)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::ChannelConnection {
                        early_capability,
                        channels,
                        connection: ChannelConnectionSequence::new(
                            self.user_channel_id,
                            self.io_channel_id,
                            channel_ids,
                        ),
                    },
                )
            }

            AcceptorState::ChannelConnection {
                early_capability,
                channels,
                mut connection,
            } => {
                let written = connection.step(input, output)?;
                let state = if connection.is_done() {
                    AcceptorState::RdpSecurityCommencement {
                        early_capability,
                        channels,
                    }
                } else {
                    AcceptorState::ChannelConnection {
                        early_capability,
                        channels,
                        connection,
                    }
                };

                (written, state)
            }

            AcceptorState::RdpSecurityCommencement {
                early_capability,
                channels,
                ..
            } => (
                Written::Nothing,
                AcceptorState::SecureSettingsExchange {
                    early_capability,
                    channels,
                },
            ),

            AcceptorState::SecureSettingsExchange {
                early_capability,
                channels,
            } => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest<'_>>(input).map_err(ConnectorError::pdu)?;

                let client_info = rdp::ClientInfoPdu::from_buffer(data.user_data.as_ref())?;

                debug!(message = ?client_info, "Received");

                (
                    Written::Nothing,
                    AcceptorState::LicensingExchange {
                        early_capability,
                        channels,
                    },
                )
            }

            AcceptorState::LicensingExchange {
                early_capability,
                channels,
            } => {
                let license = rdp::server_license::InitialServerLicenseMessage::new_status_valid_client_message();

                debug!(message = ?license, "Send");

                let written =
                    util::encode_send_data_indication(self.user_channel_id, self.io_channel_id, &license, output)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::CapabilitiesSendServer {
                        early_capability,
                        channels,
                    },
                )
            }

            AcceptorState::CapabilitiesSendServer {
                early_capability,
                channels,
            } => {
                let demand_active = rdp::headers::ShareControlHeader {
                    share_id: 0,
                    pdu_source: self.io_channel_id,
                    share_control_pdu: rdp::headers::ShareControlPdu::ServerDemandActive(
                        rdp::capability_sets::ServerDemandActive {
                            pdu: rdp::capability_sets::DemandActive {
                                source_descriptor: "".into(),
                                capability_sets: self.server_capabilities.clone(),
                            },
                        },
                    ),
                };

                debug!(message = ?demand_active, "Send");

                let written = util::encode_send_data_indication(
                    self.user_channel_id,
                    self.io_channel_id,
                    &demand_active,
                    output,
                )?;

                let layout_flag = gcc::ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU;
                let next_state = if early_capability.is_some_and(|c| c.contains(layout_flag)) {
                    AcceptorState::MonitorLayoutSend { channels }
                } else {
                    AcceptorState::CapabilitiesWaitConfirm { channels }
                };

                (Written::from_size(written)?, next_state)
            }

            AcceptorState::MonitorLayoutSend { channels } => {
                let monitor_layout =
                    rdp::headers::ShareDataPdu::MonitorLayout(rdp::finalization_messages::MonitorLayoutPdu {
                        monitors: vec![gcc::Monitor {
                            left: 0,
                            top: 0,
                            right: i32::from(self.desktop_size.width),
                            bottom: i32::from(self.desktop_size.height),
                            flags: gcc::MonitorFlags::PRIMARY,
                        }],
                    });

                debug!(message = ?monitor_layout, "Send");

                let share_data = wrap_share_data(monitor_layout, self.io_channel_id);

                let written =
                    util::encode_send_data_indication(self.user_channel_id, self.io_channel_id, &share_data, output)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::CapabilitiesWaitConfirm { channels },
                )
            }

            AcceptorState::CapabilitiesWaitConfirm { channels } => {
                let message = ironrdp_pdu::decode::<mcs::McsMessage<'_>>(input).map_err(ConnectorError::pdu)?;

                match message {
                    mcs::McsMessage::SendDataRequest(data) => {
                        let capabilities_confirm =
                            rdp::headers::ShareControlHeader::from_buffer(data.user_data.as_ref())?;

                        debug!(message = ?capabilities_confirm, "Received");

                        let ShareControlPdu::ClientConfirmActive(confirm) = capabilities_confirm.share_control_pdu
                        else {
                            return Err(ConnectorError::general("expected client confirm active"));
                        };

                        (
                            Written::Nothing,
                            AcceptorState::ConnectionFinalization {
                                channels,
                                finalization: FinalizationSequence::new(self.user_channel_id, self.io_channel_id),
                                client_capabilities: confirm.pdu.capability_sets,
                            },
                        )
                    }

                    mcs::McsMessage::DisconnectProviderUltimatum(ultimatum) => {
                        return Err(reason_err!("received disconnect ultimatum", "{:?}", ultimatum.reason))
                    }

                    _ => {
                        warn!(?message, "Unexpected MCS message received");

                        (Written::Nothing, AcceptorState::CapabilitiesWaitConfirm { channels })
                    }
                }
            }

            AcceptorState::ConnectionFinalization {
                mut finalization,
                channels,
                client_capabilities,
            } => {
                let written = finalization.step(input, output)?;
                let state = if finalization.is_done() {
                    AcceptorState::Accepted {
                        channels,
                        client_capabilities,
                        input_events: finalization.input_events,
                    }
                } else {
                    AcceptorState::ConnectionFinalization {
                        finalization,
                        channels,
                        client_capabilities,
                    }
                };

                (written, state)
            }

            _ => unreachable!(),
        };

        self.state = next_state;
        Ok(written)
    }
}

fn create_gcc_blocks(
    io_channel: u16,
    channel_ids: Vec<u16>,
    requested: nego::SecurityProtocol,
) -> gcc::ServerGccBlocks {
    pdu::gcc::ServerGccBlocks {
        core: gcc::ServerCoreData {
            version: gcc::RdpVersion::V5_PLUS,
            optional_data: gcc::ServerCoreOptionalData {
                client_requested_protocols: Some(requested),
                early_capability_flags: None,
            },
        },
        security: gcc::ServerSecurityData::no_security(),
        network: gcc::ServerNetworkData {
            channel_ids,
            io_channel,
        },
        message_channel: None,
        multi_transport_channel: None,
    }
}
