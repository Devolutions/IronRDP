use std::{borrow::Cow, io::Cursor};

use ironrdp_connector::{
    legacy, ConnectorError, ConnectorErrorExt, ConnectorResult, DesktopSize, Sequence, State, Written,
};
use ironrdp_pdu as pdu;
use pdu::{gcc, mcs, nego, rdp, PduParsing};

use crate::{capabilities, RdpServerOptions, RdpServerSecurity};

use super::channel_connection::ChannelConnectionSequence;
use super::finalization::FinalizationSequence;

impl RdpServerSecurity {
    pub fn flag(&self) -> nego::SecurityProtocol {
        match self {
            RdpServerSecurity::None => ironrdp_pdu::nego::SecurityProtocol::empty(),
            RdpServerSecurity::SSL(_) => ironrdp_pdu::nego::SecurityProtocol::SSL,
        }
    }
}

pub struct ServerAcceptor {
    opts: RdpServerOptions,
    state: AcceptorState,
    io_channel_id: u16,
    user_channel_id: u16,
    desktop_size: DesktopSize,
}

impl ServerAcceptor {
    pub fn new(opts: RdpServerOptions, size: DesktopSize) -> Self {
        Self {
            opts,
            state: AcceptorState::InitiationWaitRequest,
            user_channel_id: 1001,
            io_channel_id: 0,
            desktop_size: size,
        }
    }

    pub fn reached_security_upgrade(&self) -> Option<nego::SecurityProtocol> {
        match self.state {
            AcceptorState::SecurityUpgrade { security, .. } => Some(security),
            _ => None,
        }
    }

    pub fn is_done(&self) -> bool {
        self.state.is_terminal()
    }
}

#[derive(Default, Debug)]
pub enum AcceptorState {
    #[default]
    Consumed,

    InitiationWaitRequest,
    InitiationSendConfirm {
        security: nego::SecurityProtocol,
    },
    SecurityUpgrade {
        security: nego::SecurityProtocol,
    },
    BasicSettingsWaitInitial {
        security: nego::SecurityProtocol,
    },
    BasicSettingsSendResponse {
        security: nego::SecurityProtocol,
        client_blocks: gcc::ClientGccBlocks,
    },
    ChannelConnection {
        security: nego::SecurityProtocol,
        connection: ChannelConnectionSequence,
    },
    RdpSecurityCommencement {
        security: nego::SecurityProtocol,
    },
    SecureSettingsExchange,
    LicensingExchange,
    CapabilitiesSendServer,
    CapabilitiesWaitConfirm,
    ConnectionFinalization {
        finalization: FinalizationSequence,
    },
    Accepted,
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

impl Sequence for ServerAcceptor {
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
            AcceptorState::CapabilitiesWaitConfirm { .. } => Some(&pdu::X224_HINT),
            AcceptorState::ConnectionFinalization { finalization, .. } => finalization.next_pdu_hint(),
            AcceptorState::Accepted { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> ConnectorResult<Written> {
        println!("{:?}", self.state);

        let (written, next_state) = match std::mem::take(&mut self.state) {
            AcceptorState::InitiationWaitRequest => {
                let connection_request =
                    ironrdp_pdu::decode::<nego::ConnectionRequest>(input).map_err(ConnectorError::pdu)?;

                println!("{:?}", connection_request);
                let security = connection_request.protocol;

                (Written::Nothing, AcceptorState::InitiationSendConfirm { security })
            }

            AcceptorState::InitiationSendConfirm { security } => {
                println!(
                    "security | ours: {:?} ; theirs: {:?}",
                    self.opts.security.flag(),
                    security,
                );
                let security = self.opts.security.flag().intersection(security);

                let connection_confirm = nego::ConnectionConfirm::Response {
                    flags: nego::ResponseFlags::empty(),
                    protocol: self.opts.security.flag(),
                };

                println!("{:?}", connection_confirm);

                let written = ironrdp_pdu::encode_buf(&connection_confirm, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::SecurityUpgrade { security },
                )
            }

            AcceptorState::SecurityUpgrade { security } => {
                (Written::Nothing, AcceptorState::BasicSettingsWaitInitial { security })
            }

            AcceptorState::BasicSettingsWaitInitial { security } => {
                let settings_initial = legacy::decode_x224_packet::<mcs::ConnectInitial>(input)?;

                println!("{:?}", settings_initial);

                (
                    Written::Nothing,
                    AcceptorState::BasicSettingsSendResponse {
                        security,
                        client_blocks: settings_initial.conference_create_request.gcc_blocks,
                    },
                )
            }

            AcceptorState::BasicSettingsSendResponse {
                security,
                client_blocks,
            } => {
                let server_blocks = create_gcc_blocks(&self.opts, &client_blocks);
                let settings_response = mcs::ConnectResponse {
                    conference_create_response: gcc::ConferenceCreateResponse {
                        user_id: self.user_channel_id,
                        gcc_blocks: server_blocks,
                    },
                    called_connect_id: 1,
                    domain_parameters: mcs::DomainParameters::target(),
                };

                println!("{:?}", settings_response);

                let written = legacy::encode_x224_packet(&settings_response, output)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::ChannelConnection {
                        security,
                        connection: ChannelConnectionSequence::new(self.io_channel_id, self.user_channel_id, vec![0]),
                    },
                )
            }

            AcceptorState::ChannelConnection {
                security,
                mut connection,
            } => {
                let written = connection.step(input, output)?;
                let state = if connection.is_done() {
                    AcceptorState::RdpSecurityCommencement { security }
                } else {
                    AcceptorState::ChannelConnection { security, connection }
                };

                (written, state)
            }

            AcceptorState::RdpSecurityCommencement { .. } => (Written::Nothing, AcceptorState::SecureSettingsExchange),

            AcceptorState::SecureSettingsExchange => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest>(input).map_err(ConnectorError::pdu)?;

                let client_info = rdp::ClientInfoPdu::from_buffer(Cursor::new(data.user_data))?;

                println!("{:?}", client_info);

                (Written::Nothing, AcceptorState::LicensingExchange)
            }

            AcceptorState::LicensingExchange => {
                let license = rdp::server_license::InitialServerLicenseMessage::new_status_valid_client_message();

                println!("{:?}", license);

                let mut buf = Vec::with_capacity(license.buffer_length());
                license.to_buffer(Cursor::new(&mut buf))?;

                let license_indication = pdu::mcs::SendDataIndication {
                    initiator_id: self.user_channel_id,
                    channel_id: 0,
                    user_data: Cow::Borrowed(&buf[..license.buffer_length()]),
                };

                let written = ironrdp_pdu::encode_buf(&license_indication, output).map_err(ConnectorError::pdu)?;

                (Written::from_size(written)?, AcceptorState::CapabilitiesSendServer)
            }

            AcceptorState::CapabilitiesSendServer => {
                let demand_active = rdp::headers::ShareControlHeader {
                    share_id: 1,
                    pdu_source: 1,
                    share_control_pdu: rdp::headers::ShareControlPdu::ServerDemandActive(
                        rdp::capability_sets::ServerDemandActive {
                            pdu: rdp::capability_sets::DemandActive {
                                source_descriptor: "desc".into(),
                                capability_sets: capabilities::capabilities(&self.opts, self.desktop_size.clone()),
                            },
                        },
                    ),
                };

                println!("{:?}", demand_active);

                let mut buf = Vec::with_capacity(demand_active.buffer_length());
                demand_active.to_buffer(Cursor::new(&mut buf))?;

                let capabilities_request = pdu::mcs::SendDataIndication {
                    initiator_id: self.user_channel_id,
                    channel_id: 0,
                    user_data: Cow::Borrowed(&buf[..demand_active.buffer_length()]),
                };

                let written = ironrdp_pdu::encode_buf(&capabilities_request, output).map_err(ConnectorError::pdu)?;

                (Written::from_size(written)?, AcceptorState::CapabilitiesWaitConfirm)
            }

            AcceptorState::CapabilitiesWaitConfirm => {
                let data = pdu::decode::<pdu::mcs::SendDataRequest>(input).map_err(ConnectorError::pdu)?;

                let capabilities_confirm = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;

                println!("{:?}", capabilities_confirm);

                (
                    Written::Nothing,
                    AcceptorState::ConnectionFinalization {
                        finalization: FinalizationSequence::new(self.user_channel_id),
                    },
                )
            }

            AcceptorState::ConnectionFinalization { mut finalization } => {
                let written = finalization.step(input, output)?;
                let state = if finalization.is_done() {
                    AcceptorState::Accepted
                } else {
                    AcceptorState::ConnectionFinalization { finalization }
                };

                (written, state)
            }

            _ => unreachable!(),
        };

        self.state = next_state;
        Ok(written)
    }
}

fn create_gcc_blocks(_opts: &RdpServerOptions, _client_blocks: &gcc::ClientGccBlocks) -> gcc::ServerGccBlocks {
    pdu::gcc::ServerGccBlocks {
        core: gcc::ServerCoreData {
            version: gcc::RdpVersion::V5_PLUS,
            optional_data: gcc::ServerCoreOptionalData {
                client_requested_protocols: None,
                early_capability_flags: Some(gcc::ServerEarlyCapabilityFlags::empty()),
            },
        },
        security: gcc::ServerSecurityData::no_security(),
        network: gcc::ServerNetworkData {
            channel_ids: vec![],
            io_channel: 0,
        },
        message_channel: None,
        multi_transport_channel: None,
    }
}
