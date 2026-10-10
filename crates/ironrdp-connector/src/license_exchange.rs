use core::fmt::Debug;
use core::panic::RefUnwindSafe;
use core::{fmt, mem};
use std::borrow::Cow;
use std::str;
use std::sync::Arc;

use ironrdp_core::{Encode, WriteBuf, encode_buf, encode_vec};
use ironrdp_pdu::PduHint;
use ironrdp_pdu::mcs;
use ironrdp_pdu::rdp::headers::BASIC_SECURITY_HEADER_SIZE;
use ironrdp_pdu::rdp::server_license::{self, LicenseInformation, LicensePdu, ServerLicenseError};
use ironrdp_pdu::x224::X224;
use ironrdp_rdpsec::{SEC_LICENSE_ENCRYPT_CS, SEC_LICENSE_PKT, SharedSecurity};
use rand::RngCore as _;
use tracing::{debug, error, info, trace};

use super::{ConnectorError, ConnectorErrorExt as _, custom_err, general_err};
use crate::{
    ConnectorResult, ConnectorResultExt as _, MonotonicInstant, Sequence, State, Written, encode_send_data_request,
};

#[derive(Default, Debug)]
#[non_exhaustive]
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
pub struct LicenseExchangeSequence {
    pub state: LicenseExchangeState,
    pub io_channel_id: u16,
    pub username: String,
    pub domain: Option<String>,
    pub hardware_id: [u32; 4],
    pub license_cache: Arc<dyn LicenseCache>,
    /// Standard RDP Security cryptor, carried over from the connector when the
    /// GCC exchange selected the standard path. `None` otherwise.
    pub security: Option<SharedSecurity>,
}

// Use RefUnwindSafe so that types that embed LicenseCache remain UnwindSafe
pub trait LicenseCache: Sync + Send + Debug + RefUnwindSafe {
    fn get_license(&self, license_info: LicenseInformation) -> ConnectorResult<Option<Vec<u8>>>;
    fn store_license(&self, license_info: LicenseInformation) -> ConnectorResult<()>;
}

#[derive(Debug)]
pub(crate) struct NoopLicenseCache;

impl LicenseCache for NoopLicenseCache {
    fn get_license(&self, _license_info: LicenseInformation) -> ConnectorResult<Option<Vec<u8>>> {
        Ok(None)
    }

    fn store_license(&self, _license_info: LicenseInformation) -> ConnectorResult<()> {
        Ok(())
    }
}

impl LicenseExchangeSequence {
    pub fn new(
        io_channel_id: u16,
        username: String,
        domain: Option<String>,
        hardware_id: [u32; 4],
        license_cache: Arc<dyn LicenseCache>,
    ) -> Self {
        Self {
            state: LicenseExchangeState::NewLicenseRequest,
            io_channel_id,
            username,
            domain,
            hardware_id,
            license_cache,
            security: None,
        }
    }

    /// Send a license PDU, applying the Standard RDP Security envelope when
    /// the cryptor is armed. License PDUs embed their own 4-byte security
    /// header; on the encrypted wire it dissolves into the envelope (type
    /// bits in `SEC_LICENSE_PKT | SEC_LICENSE_ENCRYPT_CS`) while the
    /// plaintext path keeps the full PDU shape. Licensing traffic rides the
    /// I/O channel and always carries the plain MAC.
    fn send_license_pdu<T: Encode>(
        &mut self,
        initiator_id: u16,
        channel_id: u16,
        msg: &T,
        output: &mut WriteBuf,
    ) -> ConnectorResult<usize> {
        let Some(security) = &self.security else {
            return encode_send_data_request(initiator_id, channel_id, msg, output);
        };

        let encoded = encode_vec(msg).map_err(ConnectorError::encode)?;
        let body = encoded
            .get(BASIC_SECURITY_HEADER_SIZE..)
            .ok_or_else(|| general_err!("license PDU too short to carry a BASIC_SECURITY_HEADER"))?;
        let envelope = security
            .lock()
            .unwrap()
            .encrypt_envelope(SEC_LICENSE_PKT | SEC_LICENSE_ENCRYPT_CS, body, false);

        let pdu = mcs::SendDataRequest {
            initiator_id,
            channel_id,
            user_data: Cow::Owned(envelope),
        };
        encode_buf(&X224(pdu), output).map_err(ConnectorError::encode)
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

    fn step(
        &mut self,
        input: &[u8],
        _received_at: Option<MonotonicInstant>,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            LicenseExchangeState::Consumed => {
                return Err(general_err!(
                    "license exchange sequence state is consumed (this is a bug)",
                ));
            }

            LicenseExchangeState::NewLicenseRequest => {
                let send_data_indication_ctx =
                    mcs::decode_send_data_indication(input).map_err(ConnectorError::decode)?;
                let license_pdu = send_data_indication_ctx
                    .decode_user_data::<LicensePdu>()
                    .map_err(ConnectorError::decode)
                    .with_context("decode during LicenseExchangeState::NewLicenseRequest")?;

                match license_pdu {
                    LicensePdu::ServerLicenseRequest(license_request) => {
                        let mut rng = rand::rng();
                        let mut client_random = [0u8; server_license::RANDOM_NUMBER_SIZE];
                        rng.fill_bytes(&mut client_random);

                        let mut premaster_secret = [0u8; server_license::PREMASTER_SECRET_SIZE];
                        rng.fill_bytes(&mut premaster_secret);

                        let license_info = license_request
                            .scope_list
                            .iter()
                            .filter_map(|scope| {
                                self.license_cache
                                    .get_license(LicenseInformation {
                                        version: license_request.product_info.version,
                                        scope: scope.0.clone(),
                                        company_name: license_request.product_info.company_name.clone(),
                                        product_id: license_request.product_info.product_id.clone(),
                                        license_info: vec![],
                                    })
                                    .transpose()
                            })
                            .next()
                            .transpose()?;

                        if let Some(info) = license_info {
                            match server_license::ClientLicenseInfo::from_server_license_request(
                                &license_request,
                                &client_random,
                                &premaster_secret,
                                self.hardware_id,
                                info,
                            ) {
                                Ok((client_license_info, encryption_data)) => {
                                    trace!(?encryption_data, "Successfully generated Client License Info");
                                    trace!(message = ?client_license_info, "Send");

                                    let pdu: LicensePdu = client_license_info.into();
                                    let written = self.send_license_pdu(
                                        send_data_indication_ctx.initiator_id,
                                        send_data_indication_ctx.channel_id,
                                        &pdu,
                                        output,
                                    )?;

                                    trace!(?written, "Written ClientLicenseInfo");

                                    (
                                        Written::from_size(written)?,
                                        LicenseExchangeState::PlatformChallenge { encryption_data },
                                    )
                                }
                                Err(err) => {
                                    return Err(custom_err!("ClientNewLicenseRequest", err));
                                }
                            }
                        } else {
                            let hwid = self.hardware_id;
                            match server_license::ClientNewLicenseRequest::from_server_license_request(
                                &license_request,
                                &client_random,
                                &premaster_secret,
                                &self.username,
                                &format!("{:X}-{:X}-{:X}-{:X}", hwid[0], hwid[1], hwid[2], hwid[3]),
                            ) {
                                Ok((new_license_request, encryption_data)) => {
                                    trace!(?encryption_data, "Successfully generated Client New License Request");
                                    trace!(message = ?new_license_request, "Send");

                                    let pdu: LicensePdu = new_license_request.into();
                                    let written = self.send_license_pdu(
                                        send_data_indication_ctx.initiator_id,
                                        send_data_indication_ctx.channel_id,
                                        &pdu,
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
                let send_data_indication_ctx =
                    mcs::decode_send_data_indication(input).map_err(ConnectorError::decode)?;

                let license_pdu = send_data_indication_ctx
                    .decode_user_data::<LicensePdu>()
                    .map_err(ConnectorError::decode)
                    .with_context("decode during LicenseExchangeState::PlatformChallenge")?;

                match license_pdu {
                    LicensePdu::ServerPlatformChallenge(challenge) => {
                        debug!(message = ?challenge, "Received");

                        let challenge_response =
                            server_license::ClientPlatformChallengeResponse::from_server_platform_challenge(
                                &challenge,
                                self.hardware_id,
                                &encryption_data,
                            )
                            .map_err(|e| custom_err!("ClientPlatformChallengeResponse", e))?;

                        debug!(message = ?challenge_response, "Send");

                        let pdu: LicensePdu = challenge_response.into();
                        let written = self.send_license_pdu(
                            send_data_indication_ctx.initiator_id,
                            send_data_indication_ctx.channel_id,
                            &pdu,
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
                let send_data_indication_ctx =
                    mcs::decode_send_data_indication(input).map_err(ConnectorError::decode)?;

                let license_pdu = send_data_indication_ctx
                    .decode_user_data::<LicensePdu>()
                    .map_err(ConnectorError::decode)
                    .with_context("decode during SERVER_NEW_LICENSE/LicenseExchangeState::UpgradeLicense")?;

                match license_pdu {
                    LicensePdu::ServerUpgradeLicense(upgrade_license) => {
                        debug!(message = ?upgrade_license, "Received");

                        upgrade_license
                            .verify_server_license(&encryption_data)
                            .map_err(|e| custom_err!("license verification", e))?;

                        debug!("License verified with success");

                        let license_info = upgrade_license
                            .new_license_info(&encryption_data)
                            .map_err(ConnectorError::decode)?;

                        self.license_cache.store_license(license_info)?
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
