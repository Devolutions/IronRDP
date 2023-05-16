use std::mem;

use ironrdp_pdu::rdp::server_license;
use ironrdp_pdu::PduHint;
use rand_core::{OsRng, RngCore as _};

use super::legacy;
use crate::{ConnectorResult, Sequence, State, Written};

#[derive(Default, Debug)]
#[non_exhaustive]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum LicenseExchangeState {
    #[default]
    Consumed,

    NewLicenseRequest,
    PlatformChallenge {
        encryption_data: server_license::LicenseEncryptionData,
    },
    UpgradeLicense {
        encryption_data: server_license::LicenseEncryptionData,
    },
    LicenseExchanged,
}

impl State for LicenseExchangeState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::NewLicenseRequest => "NewLicenseRequest",
            Self::PlatformChallenge { .. } => "PlatformChallenge",
            Self::UpgradeLicense { .. } => "UpgradeLicense",
            Self::LicenseExchanged => "LicenseExchanged",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::LicenseExchanged)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct LicenseExchangeSequence {
    pub state: LicenseExchangeState,
    pub io_channel_id: u16,
    pub username: String,
    pub domain: Option<String>,
}

impl LicenseExchangeSequence {
    pub fn new(io_channel_id: u16, username: String, domain: Option<String>) -> Self {
        Self {
            state: LicenseExchangeState::NewLicenseRequest,
            io_channel_id,
            username,
            domain,
        }
    }
}

impl Sequence for LicenseExchangeSequence {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match self.state {
            LicenseExchangeState::Consumed => None,
            LicenseExchangeState::NewLicenseRequest => Some(&ironrdp_pdu::X224_HINT),
            LicenseExchangeState::PlatformChallenge { .. } => Some(&ironrdp_pdu::X224_HINT),
            LicenseExchangeState::UpgradeLicense { .. } => Some(&ironrdp_pdu::X224_HINT),
            LicenseExchangeState::LicenseExchanged => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            LicenseExchangeState::Consumed => {
                return Err(general_err!(
                    "license exchange sequence state is consumed (this is a bug)",
                ))
            }

            LicenseExchangeState::NewLicenseRequest => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;
                let initial_server_license =
                    send_data_indication_ctx.decode_user_data::<server_license::InitialServerLicenseMessage>()?;

                debug!(message = ?initial_server_license, "Received");

                match initial_server_license.message_type {
                    server_license::InitialMessageType::LicenseRequest(license_request) => {
                        let mut client_random = [0u8; server_license::RANDOM_NUMBER_SIZE];
                        OsRng.fill_bytes(&mut client_random);

                        let mut premaster_secret = [0u8; server_license::PREMASTER_SECRET_SIZE];
                        OsRng.fill_bytes(&mut premaster_secret);

                        let (new_license_request, encryption_data) =
                            server_license::ClientNewLicenseRequest::from_server_license_request(
                                &license_request,
                                &client_random,
                                &premaster_secret,
                                &self.username,
                                self.domain.as_deref().unwrap_or(""),
                            )
                            .map_err(|e| custom_err!("ClientNewLicenseRequest", e))?;

                        trace!(?encryption_data, "Successfully generated Client New License Request");
                        info!(message = ?new_license_request, "Send");

                        let written = legacy::encode_send_data_request(
                            send_data_indication_ctx.initiator_id,
                            send_data_indication_ctx.channel_id,
                            &new_license_request,
                            output,
                        )?;

                        (
                            Written::from_size(written)?,
                            LicenseExchangeState::PlatformChallenge { encryption_data },
                        )
                    }
                    server_license::InitialMessageType::StatusValidClient(_) => {
                        info!("Server did not initiate license exchange");

                        (Written::Nothing, LicenseExchangeState::LicenseExchanged)
                    }
                }
            }

            LicenseExchangeState::PlatformChallenge { encryption_data } => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;

                match send_data_indication_ctx.decode_user_data::<server_license::ServerPlatformChallenge>() {
                    Ok(challenge) => {
                        debug!(message = ?challenge, "Received");

                        let challenge_response =
                            server_license::ClientPlatformChallengeResponse::from_server_platform_challenge(
                                &challenge,
                                self.domain.as_deref().unwrap_or(""),
                                &encryption_data,
                            )
                            .map_err(|e| custom_err!("ClientPlatformChallengeResponse", e))?;

                        debug!(message = ?challenge_response, "Send");

                        let written = legacy::encode_send_data_request(
                            send_data_indication_ctx.initiator_id,
                            send_data_indication_ctx.channel_id,
                            &challenge_response,
                            output,
                        )?;

                        (
                            Written::from_size(written)?,
                            LicenseExchangeState::UpgradeLicense { encryption_data },
                        )
                    }
                    Err(error) => {
                        // In some cases, server does not send a platform challenge and a ServerLicenseError PDU
                        // with the VALID_CLIENT_STATUS flag is received.
                        if let Some(source) = std::error::Error::source(&error) {
                            match source.downcast_ref::<server_license::ServerLicenseError>() {
                                Some(server_license::ServerLicenseError::ValidClientStatus(
                                    licensing_error_message,
                                )) => {
                                    debug!(message = ?licensing_error_message, "Received");

                                    (Written::Nothing, LicenseExchangeState::LicenseExchanged)
                                }
                                _ => return Err(error),
                            }
                        } else {
                            return Err(error);
                        }
                    }
                }
            }

            LicenseExchangeState::UpgradeLicense { encryption_data } => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;
                let upgrade_license =
                    send_data_indication_ctx.decode_user_data::<server_license::ServerUpgradeLicense>()?;

                debug!(message = ?upgrade_license, "Received");

                upgrade_license
                    .verify_server_license(&encryption_data)
                    .map_err(|e| custom_err!("license verification", e))?;

                debug!("License verified with success");

                (Written::Nothing, LicenseExchangeState::LicenseExchanged)
            }

            LicenseExchangeState::LicenseExchanged => return Err(general_err!("license already exchanged")),
        };

        self.state = next_state;

        Ok(written)
    }
}
