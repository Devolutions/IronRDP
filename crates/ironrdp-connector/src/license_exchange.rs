use core::fmt;
use core::mem;

use ironrdp_core::WriteBuf;
use ironrdp_pdu::rdp::server_license::{self, LicensePdu, ServerLicenseError};
use ironrdp_pdu::PduHint;
use rand_core::{OsRng, RngCore as _};

use super::legacy;
use crate::{encode_send_data_request, ConnectorResult, ConnectorResultExt as _, Sequence, State, Written};

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

/// Client licensing sequence
///
/// Implements the state machine described in MS-RDPELE, section [3.1.5.3.1] Client State Transition.
///
/// [3.1.5.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/8f9b860a-3687-401d-b3bc-7e9f5d4f7528
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

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            LicenseExchangeState::Consumed => {
                return Err(general_err!(
                    "license exchange sequence state is consumed (this is a bug)",
                ))
            }

            LicenseExchangeState::NewLicenseRequest => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;
                let license_pdu = send_data_indication_ctx
                    .decode_user_data::<LicensePdu>()
                    .with_context("decode during LicenseExchangeState::NewLicenseRequest")?;

                match license_pdu {
                    LicensePdu::ServerLicenseRequest(license_request) => {
                        let mut client_random = [0u8; server_license::RANDOM_NUMBER_SIZE];
                        OsRng.fill_bytes(&mut client_random);

                        let mut premaster_secret = [0u8; server_license::PREMASTER_SECRET_SIZE];
                        OsRng.fill_bytes(&mut premaster_secret);

                        match server_license::ClientNewLicenseRequest::from_server_license_request(
                            &license_request,
                            &client_random,
                            &premaster_secret,
                            &self.username,
                            self.domain.as_deref().unwrap_or(""),
                        ) {
                            Ok((new_license_request, encryption_data)) => {
                                trace!(?encryption_data, "Successfully generated Client New License Request");
                                info!(message = ?new_license_request, "Send");

                                let written = encode_send_data_request::<LicensePdu>(
                                    send_data_indication_ctx.initiator_id,
                                    send_data_indication_ctx.channel_id,
                                    &new_license_request.into(),
                                    output,
                                )?;

                                (
                                    Written::from_size(written)?,
                                    LicenseExchangeState::PlatformChallenge { encryption_data },
                                )
                            }
                            Err(error) => {
                                if let ServerLicenseError::InvalidX509Certificate {
                                    source: error,
                                    cert_der,
                                } = &error
                                {
                                    struct BytesHexFormatter<'a>(&'a [u8]);

                                    impl fmt::Display for BytesHexFormatter<'_> {
                                        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                                            write!(f, "0x")?;
                                            self.0.iter().try_for_each(|byte| write!(f, "{byte:02X}"))
                                        }
                                    }

                                    error!(
                                        %error,
                                        cert_der = %BytesHexFormatter(cert_der),
                                        "Unsupported or invalid X509 certificate received during license exchange step"
                                    );
                                }

                                return Err(custom_err!("ClientNewLicenseRequest", error));
                            }
                        }
                    }
                    LicensePdu::LicensingErrorMessage(error_message) => {
                        if error_message.error_code != server_license::LicenseErrorCode::StatusValidClient {
                            return Err(custom_err!(
                                "LicensingErrorMessage",
                                ServerLicenseError::from(error_message)
                            ));
                        }
                        info!("Server did not initiate license exchange");
                        (Written::Nothing, LicenseExchangeState::LicenseExchanged)
                    }
                    _ => {
                        return Err(general_err!(
                            "unexpected PDU received during LicenseExchangeState::NewLicenseRequest"
                        ));
                    }
                }
            }

            LicenseExchangeState::PlatformChallenge { encryption_data } => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;

                let license_pdu = send_data_indication_ctx
                    .decode_user_data::<LicensePdu>()
                    .with_context("decode during LicenseExchangeState::PlatformChallenge")?;

                match license_pdu {
                    LicensePdu::ServerPlatformChallenge(challenge) => {
                        debug!(message = ?challenge, "Received");

                        let challenge_response =
                            server_license::ClientPlatformChallengeResponse::from_server_platform_challenge(
                                &challenge,
                                self.domain.as_deref().unwrap_or(""),
                                &encryption_data,
                            )
                            .map_err(|e| custom_err!("ClientPlatformChallengeResponse", e))?;

                        debug!(message = ?challenge_response, "Send");

                        let written = encode_send_data_request::<LicensePdu>(
                            send_data_indication_ctx.initiator_id,
                            send_data_indication_ctx.channel_id,
                            &challenge_response.into(),
                            output,
                        )?;

                        (
                            Written::from_size(written)?,
                            LicenseExchangeState::UpgradeLicense { encryption_data },
                        )
                    }
                    LicensePdu::LicensingErrorMessage(error_message) => {
                        if error_message.error_code != server_license::LicenseErrorCode::StatusValidClient {
                            return Err(custom_err!(
                                "LicensingErrorMessage",
                                ServerLicenseError::from(error_message)
                            ));
                        }
                        debug!(message = ?error_message, "Received");
                        info!("Client licensing completed");
                        (Written::Nothing, LicenseExchangeState::LicenseExchanged)
                    }
                    _ => {
                        return Err(general_err!(
                            "unexpected PDU received during LicenseExchangeState::PlatformChallenge"
                        ));
                    }
                }
            }

            LicenseExchangeState::UpgradeLicense { encryption_data } => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;

                let license_pdu = send_data_indication_ctx
                    .decode_user_data::<LicensePdu>()
                    .with_context("decode during SERVER_NEW_LICENSE/LicenseExchangeState::UpgradeLicense")?;

                match license_pdu {
                    LicensePdu::ServerUpgradeLicense(upgrade_license) => {
                        debug!(message = ?upgrade_license, "Received");

                        upgrade_license
                            .verify_server_license(&encryption_data)
                            .map_err(|e| custom_err!("license verification", e))?;

                        debug!("License verified with success");
                    }
                    LicensePdu::LicensingErrorMessage(error_message) => {
                        if error_message.error_code != server_license::LicenseErrorCode::StatusValidClient {
                            return Err(custom_err!(
                                "LicensingErrorMessage",
                                ServerLicenseError::from(error_message)
                            ));
                        }

                        debug!(message = ?error_message, "Received");
                        info!("Client licensing completed");
                    }
                    _ => {
                        return Err(general_err!(
                            "unexpected PDU received during LicenseExchangeState::UpgradeLicense"
                        ));
                    }
                }

                (Written::Nothing, LicenseExchangeState::LicenseExchanged)
            }

            LicenseExchangeState::LicenseExchanged => return Err(general_err!("license already exchanged")),
        };

        self.state = next_state;

        Ok(written)
    }
}
