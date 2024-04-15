use std::mem;

use ironrdp_connector::{
    encode_x224_packet, reason_err, ConnectorError, ConnectorErrorExt, ConnectorResult, DesktopSize, Sequence, State,
    Written,
};
use ironrdp_pdu as pdu;
use ironrdp_svc::{StaticChannelSet, SvcServerProcessor};
use pdu::rdp::capability_sets::CapabilitySet;
use pdu::rdp::headers::ShareControlPdu;
use pdu::rdp::server_license::{LicensePdu, LicensingErrorMessage};
use pdu::write_buf::WriteBuf;
use pdu::{decode, gcc, mcs, nego, rdp};

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
    static_channels: StaticChannelSet,
}

#[derive(Debug)]
pub struct AcceptorResult {
    pub static_channels: StaticChannelSet,
    pub capabilities: Vec<CapabilitySet>,
    pub input_events: Vec<Vec<u8>>,
    pub user_channel_id: u16,
    pub io_channel_id: u16,
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
            static_channels: StaticChannelSet::new(),
        }
    }

    pub fn attach_static_channel<T>(&mut self, channel: T)
    where
        T: SvcServerProcessor + 'static,
    {
        self.static_channels.insert(channel);
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
                channels: _channels, // TODO: what about ChannelDef?
                client_capabilities,
                input_events,
            } => Some(AcceptorResult {
                static_channels: mem::take(&mut self.static_channels),
                capabilities: client_capabilities,
                input_events,
                user_channel_id: self.user_channel_id,
                io_channel_id: self.io_channel_id,
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
        channels: Vec<(u16, Option<gcc::ChannelDef>)>,
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
                let connection_request = decode::<nego::ConnectionRequest>(input).map_err(ConnectorError::pdu)?;

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
                let x224_payload = decode::<pdu::x224::X224Data<'_>>(input).map_err(ConnectorError::pdu)?;
                let settings_initial =
                    decode::<mcs::ConnectInitial>(x224_payload.data.as_ref()).map_err(ConnectorError::pdu)?;

                debug!(message = ?settings_initial, "Received");

                let early_capability = settings_initial
                    .conference_create_request
                    .gcc_blocks
                    .core
                    .optional_data
                    .early_capability_flags;

                let joined: Vec<_> = settings_initial
                    .conference_create_request
                    .gcc_blocks
                    .network
                    .map(|network| {
                        network
                            .channels
                            .into_iter()
                            .map(|c| {
                                self.static_channels
                                    .get_by_channel_name(&c.name)
                                    .map(|(type_id, _)| (type_id, c))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                #[allow(clippy::arithmetic_side_effects)] // IO channel ID is not big enough for overflowing
                let channels = joined
                    .into_iter()
                    .enumerate()
                    .map(|(i, channel)| {
                        let channel_id = u16::try_from(i).unwrap() + self.io_channel_id + 1;
                        if let Some((type_id, c)) = channel {
                            self.static_channels.attach_channel_id(type_id, channel_id);
                            (channel_id, Some(c))
                        } else {
                            (channel_id, None)
                        }
                    })
                    .collect();

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

                let skip_channel_join = early_capability
                    .is_some_and(|client| client.contains(gcc::ClientEarlyCapabilityFlags::SUPPORT_SKIP_CHANNELJOIN));

                let server_blocks = create_gcc_blocks(
                    self.io_channel_id,
                    channel_ids.clone(),
                    requested_protocol,
                    skip_channel_join,
                );

                let settings_response = mcs::ConnectResponse {
                    conference_create_response: gcc::ConferenceCreateResponse {
                        user_id: self.user_channel_id,
                        gcc_blocks: server_blocks,
                    },
                    called_connect_id: 1,
                    domain_parameters: mcs::DomainParameters::target(),
                };

                debug!(message = ?settings_response, "Send");

                let written = encode_x224_packet(&settings_response, output)?;
                let channels = channels.into_iter().filter_map(|(i, c)| c.map(|c| (i, c))).collect();

                (
                    Written::from_size(written)?,
                    AcceptorState::ChannelConnection {
                        early_capability,
                        channels,
                        connection: if skip_channel_join {
                            ChannelConnectionSequence::skip_channel_join(self.user_channel_id)
                        } else {
                            ChannelConnectionSequence::new(self.user_channel_id, self.io_channel_id, channel_ids)
                        },
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
                let data: pdu::mcs::SendDataRequest<'_> = decode(input).map_err(ConnectorError::pdu)?;
                let client_info: rdp::ClientInfoPdu = decode(data.user_data.as_ref()).map_err(ConnectorError::pdu)?;

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
                let license: LicensePdu = LicensingErrorMessage::new_valid_client()
                    .map_err(ConnectorError::pdu)?
                    .into();

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
                let message = decode::<mcs::McsMessage<'_>>(input).map_err(ConnectorError::pdu)?;

                match message {
                    mcs::McsMessage::SendDataRequest(data) => {
                        let capabilities_confirm = decode::<rdp::headers::ShareControlHeader>(data.user_data.as_ref())
                            .map_err(ConnectorError::pdu)?;

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
    skip_channel_join: bool,
) -> gcc::ServerGccBlocks {
    pdu::gcc::ServerGccBlocks {
        core: gcc::ServerCoreData {
            version: gcc::RdpVersion::V5_PLUS,
            optional_data: gcc::ServerCoreOptionalData {
                client_requested_protocols: Some(requested),
                early_capability_flags: skip_channel_join
                    .then_some(gcc::ServerEarlyCapabilityFlags::SKIP_CHANNELJOIN_SUPPORTED),
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
