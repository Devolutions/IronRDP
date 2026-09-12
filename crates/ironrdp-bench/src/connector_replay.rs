//! Capture-backed, connector-driven offline workloads.
//!
//! These workloads retain the passive partial replay as a strict preflight, but
//! separately drive the production `ClientConnector` from X.224 negotiation to
//! `ConnectionResult`. TLS is an explicit completed boundary: no TLS handshake
//! or authentication traffic is measured or claimed.

use core::net::SocketAddr;
use core::str::FromStr;
use std::fmt;

use ironrdp::connector::{ClientConnector, ClientConnectorState, Config, Credentials, DesktopSize, Sequence as _};
use ironrdp::core::{WriteBuf, decode, encode_vec};
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::ClientInfoPdu;
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp::pdu::rdp::headers::BASIC_SECURITY_HEADER_SIZE;
use ironrdp::pdu::rdp::server_license::{ClientNewLicenseRequest, LicensePdu, PREAMBLE_SIZE, PreambleType};
use ironrdp::pdu::x224::{X224, X224Data};
use ironrdp::pdu::{Action, find_size, gcc, mcs, nego};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp::svc::{SvcClientProcessor, SvcMessage, SvcProcessor, impl_as_any};
use ironrdp_capture_replay::{PacketStream, decrypt_tls, read_capture};
use sha2::{Digest as _, Sha256};

use crate::replay::{PartialReplayId, PartialReplayWorkload};

const MAX_CONNECTOR_STEPS: usize = 128;
const LICENSE_HEADER_SIZE: usize = BASIC_SECURITY_HEADER_SIZE /* Basic Security Header */ + PREAMBLE_SIZE /* License Preamble */;
const CLIENT_NEW_LICENSE_REQUEST_RANDOM_OFFSET: usize =
    LICENSE_HEADER_SIZE + 4 /* PreferredKeyExchangeAlg */ + 4 /* PlatformId */;
const CLIENT_NEW_LICENSE_REQUEST_RANDOM_SIZE: usize = 32;
const LICENSE_BLOB_HEADER_SIZE: usize = 2 /* blobType */ + 2 /* length */;
const ADMINISTRATIVE_DISCONNECT: &str = "[Protocol independent error] The disconnection was initiated by an administrative tool on the server running in the user's session";

const NO_NLA_ACCEPTED_CONTRACT: ConnectorReplayContract = ConnectorReplayContract {
    expected_measurement: ConnectorReplayMeasurement {
        connector_steps: 26,
        outbound_frames: 17,
        active_frames: 725,
        active_x224_frames: 705,
        active_fast_path_frames: 20,
        active_response_frames: 29,
        graphics_updates: 0,
        deterministic_graphics_updates: 1,
    },
    outbound_prefix: &[
        OutboundPdu::ConnectionRequest,
        OutboundPdu::ConnectInitial,
        OutboundPdu::ErectDomainRequest,
        OutboundPdu::AttachUserRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ChannelJoinRequest,
        OutboundPdu::ClientInfo,
    ],
    session_data_frames: 34,
    semantic_outputs: &[
        ActiveStageSemanticOutput::SaveSessionInfo { logon_complete: true },
        ActiveStageSemanticOutput::SaveSessionInfo { logon_complete: false },
        ActiveStageSemanticOutput::AutoReconnectCookie { logon_id: 2 },
        ActiveStageSemanticOutput::TerminateOther {
            description: ADMINISTRATIVE_DISCONNECT,
        },
        ActiveStageSemanticOutput::TerminateUserInitiated,
        ActiveStageSemanticOutput::GraphicsUpdate {
            left: 0,
            top: 0,
            right: 3,
            bottom: 3,
        },
    ],
    output_fingerprint: [
        136, 13, 81, 145, 199, 202, 22, 19, 202, 244, 96, 188, 49, 170, 187, 64, 199, 213, 18, 148, 125, 103, 197, 10,
        214, 215, 197, 191, 3, 121, 2, 194,
    ],
};
/// A connector-driven capture workload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectorReplayId {
    /// The TLS-backed no-NLA accepted capture, with TLS completed externally.
    NoNlaAccepted,
}

impl ConnectorReplayId {
    /// All connector-driven workloads suitable for one-scenario measurements.
    pub const ALL: [Self; 1] = [Self::NoNlaAccepted];

    /// Stable workload identifier shared by the CLI and Criterion.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoNlaAccepted => "no-nla-accepted",
        }
    }

    const fn partial_replay_id(self) -> PartialReplayId {
        match self {
            Self::NoNlaAccepted => PartialReplayId::NoNlaAccepted,
        }
    }
}

impl FromStr for ConnectorReplayId {
    type Err = ConnectorReplayError;

    fn from_str(value: &str) -> ConnectorReplayResult<Self> {
        match value {
            "no-nla-accepted" => Ok(Self::NoNlaAccepted),
            _ => Err(ConnectorReplayError::new(format!(
                "unknown connector replay workload: {value}"
            ))),
        }
    }
}

/// A prepared, capture-backed connector workload.
pub struct ConnectorReplayWorkload {
    id: ConnectorReplayId,
    raw_server_confirm: Vec<u8>,
    server_frames: Vec<Vec<u8>>,
    config: Config,
    channel_names: Vec<gcc::ChannelName>,
}

impl ConnectorReplayWorkload {
    /// Verify, read, decrypt, and prepare the selected local capture.
    ///
    /// The strict passive replay preflight and all capture processing happen
    /// outside the focused connection-and-session measurement.
    pub fn prepare(id: ConnectorReplayId) -> ConnectorReplayResult<Self> {
        let passive_id = id.partial_replay_id();
        let passive =
            PartialReplayWorkload::prepare(passive_id).map_err(|error| ConnectorReplayError::new(error.to_string()))?;
        passive
            .verify()
            .map_err(|error| ConnectorReplayError::new(error.to_string()))?;

        let path = PartialReplayWorkload::cached_capture_path(passive_id)
            .map_err(|error| ConnectorReplayError::new(error.to_string()))?;
        let capture = read_capture(&path)
            .map_err(|error| ConnectorReplayError::new(format!("read capture {}: {error}", path.display())))?;
        let raw_server_confirm = first_frame(&capture.flow.server_stream, "raw server stream")?;
        let plaintext = decrypt_tls(&capture)
            .map_err(|error| ConnectorReplayError::new(format!("decrypt {}: {error}", id.as_str())))?;
        let client_frames = framed_stream(&plaintext.client, "client plaintext")?;
        let server_frames = framed_stream(&plaintext.server, "server plaintext")?;
        let (config, channel_names) = config_from_captured_initial(&client_frames)?;

        validate_raw_negotiation(&capture.flow.client_stream, &raw_server_confirm, &config)?;
        Ok(Self {
            id,
            raw_server_confirm,
            server_frames,
            config,
            channel_names,
        })
    }

    /// Strictly verify a fresh connector and active-session execution.
    ///
    /// This hashes the validated semantic output outside the timed path.
    pub fn verify(&self) -> ConnectorReplayResult<ConnectorReplayMeasurement> {
        let (measurement, output_fingerprint, outbound_pdus) = self.replay_once(true)?;
        self.contract()
            .validate(measurement, output_fingerprint, &outbound_pdus)
    }

    /// Run one fresh, hash-free connector and active-session execution.
    ///
    /// This includes constructing the connector, driving connection
    /// establishment, constructing the active stage from its result, and
    /// processing every decrypted server frame. TLS and CredSSP are not run.
    pub fn replay(&self) -> ConnectorReplayResult<ConnectorReplayMeasurement> {
        let (measurement, _, outbound_pdus) = self.replay_once(false)?;
        self.contract().validate(measurement, None, &outbound_pdus)
    }

    fn replay_once(
        &self,
        verify_output: bool,
    ) -> ConnectorReplayResult<(ConnectorReplayMeasurement, Option<[u8; 32]>, Vec<OutboundPdu>)> {
        let mut connector = ClientConnector::new(self.config.clone(), SocketAddr::from(([127, 0, 0, 1], 3389)));
        for name in &self.channel_names {
            if !connector.attach_dynamic_static_channel(OpaqueStaticChannel { name: name.clone() }) {
                return Err(ConnectorReplayError::new(
                    "connector replay could not attach captured static channel".to_owned(),
                ));
            }
        }

        let mut output_fingerprint = verify_output.then(Sha256::new);
        let mut outbound_pdus = Vec::new();
        let mut semantic_outputs = ActiveOutputValidator::new(self.contract().semantic_outputs);
        let mut outbound_frames = 0;
        let mut connector_steps = 0;

        let initial_request = step_no_input(&mut connector, "send connection request")?;
        connector_steps += 1;
        outbound_frames += validate_outbound_frames(
            &initial_request,
            "connection request",
            output_fingerprint.as_mut(),
            &mut outbound_pdus,
        )?;
        validate_connection_request(&initial_request, &self.config)?;

        let confirm_output = step_input(&mut connector, &self.raw_server_confirm, "receive connection confirm")?;
        connector_steps += 1;
        outbound_frames += validate_outbound_frames(
            &confirm_output,
            "connection confirm",
            output_fingerprint.as_mut(),
            &mut outbound_pdus,
        )?;
        if !connector.should_perform_security_upgrade() {
            return Err(ConnectorReplayError::new(
                "connector did not stop at the TLS completion boundary".to_owned(),
            ));
        }

        // The capture's TLS application data is already authenticated and
        // decrypted during preparation. The TLS handshake is intentionally not
        // part of this workload.
        connector.mark_security_upgrade_as_done();

        let mut server_frames = self.server_frames.iter();
        loop {
            if connector_steps >= MAX_CONNECTOR_STEPS {
                return Err(ConnectorReplayError::new(
                    "connector replay exceeded its bounded step count".to_owned(),
                ));
            }
            if matches!(connector.state, ClientConnectorState::Connected { .. }) {
                break;
            }
            if connector.should_perform_credssp() {
                return Err(ConnectorReplayError::new(
                    "no-NLA connector replay unexpectedly reached CredSSP".to_owned(),
                ));
            }
            if connector.should_perform_multitransport() {
                return Err(ConnectorReplayError::new(
                    "connector replay reached a multitransport handoff without an explicit TCP-only decline".to_owned(),
                ));
            }

            let output = if connector.next_pdu_hint().is_some() {
                let input = server_frames.next().ok_or_else(|| {
                    ConnectorReplayError::new(format!(
                        "server transcript ended while connector awaited {}",
                        connector.state().name()
                    ))
                })?;
                step_input(&mut connector, input, "process server transcript frame")?
            } else {
                step_no_input(&mut connector, "produce connector output")?
            };
            connector_steps += 1;
            outbound_frames += validate_outbound_frames(
                &output,
                "connector response",
                output_fingerprint.as_mut(),
                &mut outbound_pdus,
            )?;
        }

        let ClientConnectorState::Connected { result } = connector.state else {
            return Err(ConnectorReplayError::new(
                "connector replay did not reach Connected".to_owned(),
            ));
        };
        let mut stage = ActiveStageBuilder {
            static_channels: result.static_channels,
            user_channel_id: result.user_channel_id,
            io_channel_id: result.io_channel_id,
            message_channel_id: result.message_channel_id,
            share_id: result.share_id,
            compression_type: result.compression_type,
            enable_server_pointer: result.enable_server_pointer,
            pointer_software_rendering: result.pointer_software_rendering,
        }
        .build();
        let mut image = DecodedImage::new(
            ironrdp::graphics::image_processing::PixelFormat::RgbA32,
            result.desktop_size.width,
            result.desktop_size.height,
        );

        let mut active_frames = 0;
        let mut active_x224_frames = 0;
        let mut active_fast_path_frames = 0;
        let mut graphics_updates = 0;
        let mut active_response_frames = 0;
        for frame in server_frames {
            let info = complete_frame_info(frame, "remaining server transcript frame")?;
            active_frames += 1;
            match info.action {
                Action::X224 => active_x224_frames += 1,
                Action::FastPath => active_fast_path_frames += 1,
            }
            match stage.process(&mut image, info.action, frame) {
                Ok(outputs) => {
                    let outputs = validate_active_outputs(
                        &outputs,
                        output_fingerprint.as_mut(),
                        &mut outbound_pdus,
                        &mut semantic_outputs,
                    )?;
                    graphics_updates += outputs.graphics_updates;
                    active_response_frames += outputs.response_frames;
                }
                Err(error) => {
                    return Err(ConnectorReplayError::new(format!(
                        "active stage rejected server transcript frame: {error}"
                    )));
                }
            }
        }

        if active_frames == 0 {
            return Err(ConnectorReplayError::new(
                "connector replay reached Connected without active-stage work".to_owned(),
            ));
        }
        let deterministic_outputs = stage
            .process(&mut image, Action::FastPath, &deterministic_bitmap_frame())
            .map_err(|error| {
                ConnectorReplayError::new(format!("active stage rejected deterministic bitmap frame: {error}"))
            })?;
        let deterministic_outputs = validate_active_outputs(
            &deterministic_outputs,
            output_fingerprint.as_mut(),
            &mut outbound_pdus,
            &mut semantic_outputs,
        )?;
        let deterministic_graphics_updates = deterministic_outputs.graphics_updates;
        active_response_frames += deterministic_outputs.response_frames;
        if deterministic_graphics_updates != 1 {
            return Err(ConnectorReplayError::new(format!(
                "deterministic active-stage bitmap produced {deterministic_graphics_updates} graphics updates"
            )));
        }
        semantic_outputs.finish()?;
        Ok((
            ConnectorReplayMeasurement {
                connector_steps,
                outbound_frames,
                active_frames,
                active_x224_frames,
                active_fast_path_frames,
                active_response_frames,
                graphics_updates,
                deterministic_graphics_updates,
            },
            output_fingerprint.map(|hasher| hasher.finalize().into()),
            outbound_pdus,
        ))
    }

    /// Stable workload identifier.
    pub const fn id(&self) -> ConnectorReplayId {
        self.id
    }

    const fn contract(&self) -> &ConnectorReplayContract {
        match self.id {
            ConnectorReplayId::NoNlaAccepted => &NO_NLA_ACCEPTED_CONTRACT,
        }
    }
}

/// Payload-free observations from one connector-driven execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConnectorReplayMeasurement {
    /// Connector `step` and `step_no_input` calls required to reach `Connected`.
    pub connector_steps: usize,
    /// Complete outbound RDP frames produced while connecting.
    pub outbound_frames: usize,
    /// Decrypted server frames processed through `ActiveStage`.
    pub active_frames: usize,
    /// X.224 server frames processed through `ActiveStage`.
    pub active_x224_frames: usize,
    /// Fast-Path server frames processed through `ActiveStage`.
    pub active_fast_path_frames: usize,
    /// Complete active-stage response frames generated from server traffic.
    pub active_response_frames: usize,
    /// Rendered graphics updates produced by the active stage.
    pub graphics_updates: usize,
    /// Rendered updates produced by the deterministic post-capture bitmap.
    pub deterministic_graphics_updates: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConnectorReplayContract {
    expected_measurement: ConnectorReplayMeasurement,
    outbound_prefix: &'static [OutboundPdu],
    session_data_frames: usize,
    semantic_outputs: &'static [ActiveStageSemanticOutput],
    output_fingerprint: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboundPdu {
    ConnectionRequest,
    ConnectInitial,
    ErectDomainRequest,
    AttachUserRequest,
    ChannelJoinRequest,
    ClientInfo,
    ClientNewLicenseRequest,
    SessionData,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveStageSemanticOutput {
    GraphicsUpdate {
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    },
    SaveSessionInfo {
        logon_complete: bool,
    },
    AutoReconnectCookie {
        logon_id: u32,
    },
    TerminateOther {
        description: &'static str,
    },
    TerminateUserInitiated,
}

struct ActiveOutputValidator {
    expected: &'static [ActiveStageSemanticOutput],
    next: usize,
}

impl ActiveOutputValidator {
    const fn new(expected: &'static [ActiveStageSemanticOutput]) -> Self {
        Self { expected, next: 0 }
    }

    fn observe(&mut self, output: &ActiveStageOutput) -> ConnectorReplayResult<bool> {
        if let ActiveStageOutput::Terminate(reason) = output {
            return self.observe_termination(reason);
        }
        let actual = match output {
            ActiveStageOutput::ResponseFrame(_) => return Ok(false),
            ActiveStageOutput::GraphicsUpdate(rectangle) => ActiveStageSemanticOutput::GraphicsUpdate {
                left: rectangle.left,
                top: rectangle.top,
                right: rectangle.right,
                bottom: rectangle.bottom,
            },
            ActiveStageOutput::SaveSessionInfo { logon_complete } => ActiveStageSemanticOutput::SaveSessionInfo {
                logon_complete: *logon_complete,
            },
            ActiveStageOutput::AutoReconnectCookie(cookie) => ActiveStageSemanticOutput::AutoReconnectCookie {
                logon_id: cookie.logon_id,
            },
            _ => {
                return Err(ConnectorReplayError::new(format!(
                    "connector replay produced unexpected ActiveStage semantic output: {}",
                    active_stage_output_name(output)
                )));
            }
        };
        let expected = self.expected.get(self.next).ok_or_else(|| {
            ConnectorReplayError::new(format!(
                "connector replay produced unexpected ActiveStage semantic output: {actual:?}"
            ))
        })?;
        if actual != *expected {
            return Err(ConnectorReplayError::new(format!(
                "connector replay ActiveStage semantic output changed: expected {expected:?}, got {actual:?}"
            )));
        }
        self.next += 1;
        Ok(matches!(actual, ActiveStageSemanticOutput::GraphicsUpdate { .. }))
    }

    fn observe_termination(&mut self, reason: &GracefulDisconnectReason) -> ConnectorReplayResult<bool> {
        let expected = self.expected.get(self.next).ok_or_else(|| {
            ConnectorReplayError::new(format!(
                "connector replay produced unexpected ActiveStage termination: {reason:?}"
            ))
        })?;
        let matches = match (reason, expected) {
            (GracefulDisconnectReason::Other(actual), ActiveStageSemanticOutput::TerminateOther { description }) => {
                actual == description
            }
            (GracefulDisconnectReason::UserInitiated, ActiveStageSemanticOutput::TerminateUserInitiated) => true,
            _ => false,
        };
        if !matches {
            return Err(ConnectorReplayError::new(format!(
                "connector replay ActiveStage termination changed: expected {expected:?}, got {reason:?}"
            )));
        }
        self.next += 1;
        Ok(false)
    }

    fn finish(&self) -> ConnectorReplayResult<()> {
        if self.next != self.expected.len() {
            return Err(ConnectorReplayError::new(format!(
                "connector replay ActiveStage semantic output sequence ended early: expected {}, got {}",
                self.expected.len(),
                self.next
            )));
        }
        Ok(())
    }
}

const fn active_stage_output_name(output: &ActiveStageOutput) -> &'static str {
    match output {
        ActiveStageOutput::ResponseFrame(_) => "ResponseFrame",
        ActiveStageOutput::GraphicsUpdate(_) => "GraphicsUpdate",
        ActiveStageOutput::PointerDefault => "PointerDefault",
        ActiveStageOutput::PointerHidden => "PointerHidden",
        ActiveStageOutput::PointerPosition { .. } => "PointerPosition",
        ActiveStageOutput::PointerBitmap(_) => "PointerBitmap",
        ActiveStageOutput::MonitorLayout(_) => "MonitorLayout",
        ActiveStageOutput::WindowingOrders(_) => "WindowingOrders",
        ActiveStageOutput::Terminate(_) => "Terminate",
        ActiveStageOutput::SaveSessionInfo { .. } => "SaveSessionInfo",
        ActiveStageOutput::DeactivateAll => "DeactivateAll",
        ActiveStageOutput::MultitransportRequest(_) => "MultitransportRequest",
        ActiveStageOutput::AutoDetect(_) => "AutoDetect",
        ActiveStageOutput::AutoReconnectCookie(_) => "AutoReconnectCookie",
        ActiveStageOutput::AutoReconnectFailed => "AutoReconnectFailed",
    }
}

impl ConnectorReplayContract {
    fn validate(
        self,
        measurement: ConnectorReplayMeasurement,
        output_fingerprint: Option<[u8; 32]>,
        outbound_pdus: &[OutboundPdu],
    ) -> ConnectorReplayResult<ConnectorReplayMeasurement> {
        if measurement != self.expected_measurement {
            return Err(ConnectorReplayError::new(format!(
                "connector replay did not satisfy its immutable contract: expected {:?}, got {measurement:?}",
                self.expected_measurement
            )));
        }
        let (prefix, session_data) = outbound_pdus.split_at(outbound_pdus.len().min(self.outbound_prefix.len()));
        if prefix != self.outbound_prefix
            || session_data.len() != self.session_data_frames
            || session_data.iter().any(|pdu| *pdu != OutboundPdu::SessionData)
        {
            return Err(ConnectorReplayError::new(format!(
                "connector replay outbound PDU sequence changed: expected {:?} then {} session frames, got {outbound_pdus:?}",
                self.outbound_prefix, self.session_data_frames
            )));
        }
        if let Some(output_fingerprint) = output_fingerprint {
            if output_fingerprint != self.output_fingerprint {
                return Err(ConnectorReplayError::new(format!(
                    "connector replay output fingerprint changed: expected {:?}, got {output_fingerprint:?}",
                    self.output_fingerprint
                )));
            }
        }
        Ok(measurement)
    }
}

/// Error returned when a connector replay cannot satisfy its contract.
#[derive(Debug)]
pub struct ConnectorReplayError(String);

impl ConnectorReplayError {
    fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for ConnectorReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for ConnectorReplayError {}

/// Result returned by connector replay preparation and execution.
pub type ConnectorReplayResult<T> = Result<T, ConnectorReplayError>;

#[derive(Debug)]
struct OpaqueStaticChannel {
    name: gcc::ChannelName,
}

impl_as_any!(OpaqueStaticChannel);

impl SvcProcessor for OpaqueStaticChannel {
    fn channel_name(&self) -> gcc::ChannelName {
        self.name.clone()
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp::pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

impl SvcClientProcessor for OpaqueStaticChannel {}

fn config_from_captured_initial(client_frames: &[Vec<u8>]) -> ConnectorReplayResult<(Config, Vec<gcc::ChannelName>)> {
    let initial = client_frames
        .iter()
        .find_map(|frame| {
            let payload = decode::<X224<X224Data<'_>>>(frame).ok()?.0;
            decode::<mcs::ConnectInitial>(payload.data.as_ref()).ok()
        })
        .ok_or_else(|| ConnectorReplayError::new("client transcript has no MCS Connect Initial".to_owned()))?;
    let blocks = initial.conference_create_request.gcc_blocks();
    let core = &blocks.core;
    let channel_names = blocks
        .network
        .as_ref()
        .map(|network| network.channels.iter().map(|channel| channel.name.clone()).collect())
        .unwrap_or_default();

    Ok((
        Config {
            desktop_size: DesktopSize {
                width: core.desktop_width,
                height: core.desktop_height,
            },
            monitor_layout: None,
            desktop_scale_factor: core.optional_data.desktop_scale_factor.unwrap_or_default(),
            enable_tls: true,
            enable_credssp: false,
            enable_standard_rdp_security: false,
            credentials: Credentials::UsernamePassword {
                username: String::new(),
                password: String::new(),
            },
            domain: None,
            client_build: core.client_build,
            client_name: "ironrdp-bench".to_owned(),
            keyboard_type: core.keyboard_type,
            keyboard_subtype: core.keyboard_subtype,
            keyboard_functional_keys_count: core.keyboard_functional_keys_count,
            keyboard_layout: core.keyboard_layout,
            connection_type: core.optional_data.connection_type.unwrap_or(gcc::ConnectionType::Lan),
            ime_file_name: core.ime_file_name.clone(),
            bitmap: None,
            dig_product_id: core.optional_data.dig_product_id.clone().unwrap_or_default(),
            client_dir: String::new(),
            alternate_shell: String::new(),
            work_dir: String::new(),
            remote_application_mode: false,
            rail_support_level: RailSupportLevel::SUPPORTED,
            platform: MajorPlatformType::UNIX,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            enable_audio_capture: false,
            performance_flags: Default::default(),
            license_cache: None,
            timezone_info: Default::default(),
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
            multitransport_flags: None,
            support_dyn_vc_gfx_protocol: false,
        },
        channel_names,
    ))
}

fn validate_raw_negotiation(
    raw_client_stream: &PacketStream,
    raw_server_confirm: &[u8],
    config: &Config,
) -> ConnectorReplayResult<()> {
    let request = first_frame(raw_client_stream, "raw client stream")?;
    validate_connection_request(&request, config)?;
    let confirm = decode::<X224<nego::ConnectionConfirm>>(raw_server_confirm)
        .map_err(|error| ConnectorReplayError::new(format!("decode connection confirm: {error}")))?
        .0;
    match confirm {
        nego::ConnectionConfirm::Response {
            protocol: nego::SecurityProtocol::SSL,
            ..
        } => Ok(()),
        nego::ConnectionConfirm::Response { protocol, .. } => Err(ConnectorReplayError::new(format!(
            "no-NLA connector replay requires TLS-only confirmation, got {protocol}"
        ))),
        nego::ConnectionConfirm::Failure { code } => Err(ConnectorReplayError::new(format!(
            "capture rejected connection negotiation: {code}"
        ))),
    }
}

fn validate_connection_request(frame: &[u8], config: &Config) -> ConnectorReplayResult<()> {
    let request = decode::<X224<nego::ConnectionRequest>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode connection request: {error}")))?
        .0;
    let expected = if config.enable_tls {
        nego::SecurityProtocol::SSL
    } else {
        nego::SecurityProtocol::empty()
    };
    if request.protocol != expected {
        return Err(ConnectorReplayError::new(format!(
            "connection request protocol changed: expected {expected}, got {}",
            request.protocol
        )));
    }
    Ok(())
}

fn first_frame(stream: &PacketStream, description: &str) -> ConnectorReplayResult<Vec<u8>> {
    let bytes: Vec<u8> = stream.iter().flat_map(|(_, bytes)| bytes).copied().collect();
    let info = find_size(&bytes)
        .map_err(|error| ConnectorReplayError::new(format!("frame {description}: {error}")))?
        .ok_or_else(|| ConnectorReplayError::new(format!("truncated {description}")))?;
    if info.action != Action::X224 || bytes.len() < info.length {
        return Err(ConnectorReplayError::new(format!(
            "invalid {description} initial frame"
        )));
    }
    Ok(bytes[..info.length].to_vec())
}

fn framed_stream(stream: &PacketStream, description: &str) -> ConnectorReplayResult<Vec<Vec<u8>>> {
    let bytes: Vec<u8> = stream.iter().flat_map(|(_, bytes)| bytes).copied().collect();
    let mut offset = 0;
    let mut frames = Vec::new();
    while offset < bytes.len() {
        let info = find_size(&bytes[offset..])
            .map_err(|error| ConnectorReplayError::new(format!("frame {description}: {error}")))?
            .ok_or_else(|| ConnectorReplayError::new(format!("truncated {description}")))?;
        let end = offset
            .checked_add(info.length)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| ConnectorReplayError::new(format!("truncated {description}")))?;
        frames.push(bytes[offset..end].to_vec());
        offset = end;
    }
    if frames.is_empty() {
        return Err(ConnectorReplayError::new(format!("{description} has no frames")));
    }
    Ok(frames)
}

fn step_no_input(connector: &mut ClientConnector, description: &str) -> ConnectorReplayResult<Vec<u8>> {
    let mut output = WriteBuf::new();
    connector
        .step_no_input(&mut output)
        .map_err(|error| ConnectorReplayError::new(format!("{description}: {error}")))?;
    Ok(output.into_inner())
}

fn step_input(connector: &mut ClientConnector, input: &[u8], description: &str) -> ConnectorReplayResult<Vec<u8>> {
    let mut output = WriteBuf::new();
    connector
        .step(input, None, &mut output)
        .map_err(|error| ConnectorReplayError::new(format!("{description}: {error}")))?;
    Ok(output.into_inner())
}

fn validate_outbound_frames(
    frames: &[u8],
    description: &str,
    output_fingerprint: Option<&mut Sha256>,
    outbound_pdus: &mut Vec<OutboundPdu>,
) -> ConnectorReplayResult<usize> {
    if frames.is_empty() {
        return Ok(0);
    }
    let mut offset = 0;
    let mut count = 0;
    let mut output_fingerprint = output_fingerprint;
    while offset < frames.len() {
        let info = find_size(&frames[offset..])
            .map_err(|error| ConnectorReplayError::new(format!("frame {description}: {error}")))?
            .ok_or_else(|| ConnectorReplayError::new(format!("truncated {description}")))?;
        if info.action != Action::X224 {
            return Err(ConnectorReplayError::new(format!(
                "{description} unexpectedly produced a Fast-Path frame"
            )));
        }
        let end = offset
            .checked_add(info.length)
            .filter(|&end| end <= frames.len())
            .ok_or_else(|| ConnectorReplayError::new(format!("truncated {description}")))?;
        outbound_pdus.push(inspect_outbound_frame(&frames[offset..end])?);
        if let Some(output_fingerprint) = output_fingerprint.as_deref_mut() {
            update_outbound_fingerprint(&frames[offset..end], output_fingerprint)?;
        }
        offset = end;
        count += 1;
    }
    Ok(count)
}

fn inspect_outbound_frame(frame: &[u8]) -> ConnectorReplayResult<OutboundPdu> {
    if decode::<X224<nego::ConnectionRequest>>(frame).is_ok() {
        return Ok(OutboundPdu::ConnectionRequest);
    }
    let payload = decode::<X224<X224Data<'_>>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode outbound X.224 frame: {error}")))?
        .0;
    if decode::<mcs::ConnectInitial>(payload.data.as_ref()).is_ok() {
        return Ok(OutboundPdu::ConnectInitial);
    }
    let message = decode::<X224<mcs::McsMessage<'_>>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode outbound MCS frame: {error}")))?;
    match message.0 {
        mcs::McsMessage::ErectDomainRequest(_) => Ok(OutboundPdu::ErectDomainRequest),
        mcs::McsMessage::AttachUserRequest(_) => Ok(OutboundPdu::AttachUserRequest),
        mcs::McsMessage::ChannelJoinRequest(_) => Ok(OutboundPdu::ChannelJoinRequest),
        mcs::McsMessage::SendDataRequest(data) => inspect_send_data(data.user_data.as_ref()),
        _ => Err(ConnectorReplayError::new(
            "connector replay produced an unexpected server-direction MCS frame".to_owned(),
        )),
    }
}

fn inspect_send_data(user_data: &[u8]) -> ConnectorReplayResult<OutboundPdu> {
    if let Ok(client_info) = decode::<ClientInfoPdu>(user_data) {
        assert_client_info_is_normalized(&client_info)?;
        return Ok(OutboundPdu::ClientInfo);
    }
    if let Ok(license) = decode::<LicensePdu>(user_data) {
        if !matches!(license, LicensePdu::ClientNewLicenseRequest(_)) {
            return Err(ConnectorReplayError::new(
                "connector replay produced an unexpected licensing PDU".to_owned(),
            ));
        }
        return Ok(OutboundPdu::ClientNewLicenseRequest);
    }
    Ok(OutboundPdu::SessionData)
}

fn update_outbound_fingerprint(frame: &[u8], output_fingerprint: &mut Sha256) -> ConnectorReplayResult<()> {
    if decode::<X224<nego::ConnectionRequest>>(frame).is_ok() {
        output_fingerprint.update(b"ConnectionRequest");
        output_fingerprint.update(frame);
        return Ok(());
    }
    let payload = decode::<X224<X224Data<'_>>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode outbound X.224 frame: {error}")))?
        .0;
    if decode::<mcs::ConnectInitial>(payload.data.as_ref()).is_ok() {
        output_fingerprint.update(b"ConnectInitial");
        output_fingerprint.update(frame);
        return Ok(());
    }
    let message = decode::<X224<mcs::McsMessage<'_>>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode outbound MCS frame: {error}")))?;
    match message.0 {
        mcs::McsMessage::ErectDomainRequest(_) => {
            output_fingerprint.update(b"ErectDomainRequest");
        }
        mcs::McsMessage::AttachUserRequest(_) => {
            output_fingerprint.update(b"AttachUserRequest");
        }
        mcs::McsMessage::ChannelJoinRequest(_) => {
            output_fingerprint.update(b"ChannelJoinRequest");
        }
        mcs::McsMessage::SendDataRequest(data) => {
            update_send_data_fingerprint(
                data.user_data.as_ref(),
                data.initiator_id,
                data.channel_id,
                output_fingerprint,
            )?;
        }
        _ => {
            return Err(ConnectorReplayError::new(
                "connector replay produced an unexpected server-direction MCS frame".to_owned(),
            ));
        }
    }
    Ok(())
}

fn update_send_data_fingerprint(
    user_data: &[u8],
    initiator_id: u16,
    channel_id: u16,
    output_fingerprint: &mut Sha256,
) -> ConnectorReplayResult<()> {
    output_fingerprint.update(b"SendDataRequest");
    output_fingerprint.update(initiator_id.to_le_bytes());
    output_fingerprint.update(channel_id.to_le_bytes());

    if let Ok(client_info) = decode::<ClientInfoPdu>(user_data) {
        assert_client_info_is_normalized(&client_info)?;
        output_fingerprint.update(b"ClientInfo");
        output_fingerprint.update(user_data);
        return Ok(());
    }

    if let Ok(license) = decode::<LicensePdu>(user_data) {
        let LicensePdu::ClientNewLicenseRequest(request) = license else {
            return Err(ConnectorReplayError::new(
                "connector replay produced an unexpected licensing PDU".to_owned(),
            ));
        };
        return update_client_new_license_request_fingerprint(user_data, &request, output_fingerprint);
    }

    output_fingerprint.update(b"SessionData");
    output_fingerprint.update(user_data);
    Ok(())
}

fn assert_client_info_is_normalized(client_info: &ClientInfoPdu) -> ConnectorReplayResult<()> {
    if !client_info.client_info.credentials.username.is_empty()
        || !client_info.client_info.credentials.password.is_empty()
        || client_info.client_info.credentials.domain.is_some()
    {
        return Err(ConnectorReplayError::new(
            "connector replay produced non-normalized Client Info credentials".to_owned(),
        ));
    }
    Ok(())
}

fn update_client_new_license_request_fingerprint(
    user_data: &[u8],
    request: &ClientNewLicenseRequest,
    output_fingerprint: &mut Sha256,
) -> ConnectorReplayResult<()> {
    // MS-RDPELE 3.3.5.1/.2 requires a NEW_LICENSE_REQUEST when no cached
    // license exists and specifies newly generated ClientRandom and premaster
    // secret values. Those are the only intentionally omitted fields.
    if request.license_header.preamble_message_type != PreambleType::NewLicenseRequest {
        return Err(ConnectorReplayError::new(
            "connector replay licensing output was not NEW_LICENSE_REQUEST".to_owned(),
        ));
    }
    let [client_random, encrypted_premaster_secret] = client_new_license_request_secret_ranges(user_data)?;
    output_fingerprint.update(b"ClientNewLicenseRequest");
    output_fingerprint.update(&user_data[..client_random.start]);
    output_fingerprint.update(b"ClientRandom");
    output_fingerprint.update(&user_data[client_random.end..encrypted_premaster_secret.start]);
    output_fingerprint.update(b"EncryptedPreMasterSecret");
    output_fingerprint.update(&user_data[encrypted_premaster_secret.end..]);
    Ok(())
}

fn client_new_license_request_secret_ranges(user_data: &[u8]) -> ConnectorReplayResult<[core::ops::Range<usize>; 2]> {
    let client_random = CLIENT_NEW_LICENSE_REQUEST_RANDOM_OFFSET
        ..CLIENT_NEW_LICENSE_REQUEST_RANDOM_OFFSET
            .checked_add(CLIENT_NEW_LICENSE_REQUEST_RANDOM_SIZE)
            .ok_or_else(|| ConnectorReplayError::new("client random range overflows".to_owned()))?;
    let encrypted_premaster_secret_header = client_random.end;
    let encrypted_premaster_secret_length = encrypted_premaster_secret_header
        .checked_add(2 /* blobType */)
        .and_then(|offset| user_data.get(offset..offset.checked_add(2 /* length */)?))
        .ok_or_else(|| ConnectorReplayError::new("truncated encrypted premaster secret header".to_owned()))?;
    let encrypted_premaster_secret_length = usize::from(u16::from_le_bytes(
        encrypted_premaster_secret_length
            .try_into()
            .map_err(|_| ConnectorReplayError::new("invalid encrypted premaster secret length".to_owned()))?,
    ));
    let encrypted_premaster_secret_start = encrypted_premaster_secret_header
        .checked_add(LICENSE_BLOB_HEADER_SIZE)
        .ok_or_else(|| ConnectorReplayError::new("encrypted premaster secret range overflows".to_owned()))?;
    let encrypted_premaster_secret = encrypted_premaster_secret_start
        ..encrypted_premaster_secret_start
            .checked_add(encrypted_premaster_secret_length)
            .filter(|&end| end <= user_data.len())
            .ok_or_else(|| ConnectorReplayError::new("truncated encrypted premaster secret".to_owned()))?;
    if client_random.end > user_data.len() {
        return Err(ConnectorReplayError::new("truncated client random".to_owned()));
    }
    Ok([client_random, encrypted_premaster_secret])
}

fn complete_frame_info(frame: &[u8], description: &str) -> ConnectorReplayResult<ironrdp::pdu::PduInfo> {
    let info = find_size(frame)
        .map_err(|error| ConnectorReplayError::new(format!("frame {description}: {error}")))?
        .ok_or_else(|| ConnectorReplayError::new(format!("truncated {description}")))?;
    if info.length != frame.len() {
        return Err(ConnectorReplayError::new(format!(
            "{description} does not contain exactly one complete frame"
        )));
    }
    Ok(info)
}

fn validate_active_outputs(
    outputs: &[ActiveStageOutput],
    output_fingerprint: Option<&mut Sha256>,
    outbound_pdus: &mut Vec<OutboundPdu>,
    semantic_outputs: &mut ActiveOutputValidator,
) -> ConnectorReplayResult<ActiveOutputMeasurement> {
    let mut measurement = ActiveOutputMeasurement::default();
    let mut output_fingerprint = output_fingerprint;
    for output in outputs {
        if let ActiveStageOutput::ResponseFrame(frame) = output {
            measurement.response_frames += validate_outbound_frames(
                frame,
                "active-stage response",
                output_fingerprint.as_deref_mut(),
                outbound_pdus,
            )?;
        } else if semantic_outputs.observe(output)? {
            measurement.graphics_updates += 1;
        }
    }
    Ok(measurement)
}

#[derive(Default)]
struct ActiveOutputMeasurement {
    response_frames: usize,
    graphics_updates: usize,
}

/// Build one deterministic Fast-Path bitmap frame for active-stage coverage.
fn deterministic_bitmap_frame() -> Vec<u8> {
    let pixels = vec![0x40; 4 * 4 * 4];
    let update = BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 3,
            },
            width: 4,
            height: 4,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &pixels,
        }],
    };
    let update = encode_vec(&FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::Bitmap,
        compression_flags: None,
        compression_type: None,
        data: &encode_vec(&update).expect("deterministic bitmap update must encode"),
    })
    .expect("deterministic Fast-Path update must encode");
    let mut frame = encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), update.len()))
        .expect("deterministic Fast-Path header must encode");
    frame.extend(update);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp::pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
    use ironrdp::pdu::rdp::server_license::{PreambleFlags, PreambleVersion};

    #[test]
    fn rejects_unknown_connector_replay_id() {
        let error = "not-a-capture"
            .parse::<ConnectorReplayId>()
            .expect_err("unknown workload must fail");
        assert_eq!(error.to_string(), "unknown connector replay workload: not-a-capture");
    }

    #[test]
    fn rejects_empty_and_truncated_transcripts() {
        assert!(framed_stream(&Vec::new(), "test").is_err());
        assert!(framed_stream(&vec![(1, vec![0x03, 0x00, 0x00, 0x06])], "test").is_err());
    }

    #[test]
    fn deterministic_active_input_is_one_complete_frame() {
        let frame = deterministic_bitmap_frame();
        assert_eq!(
            complete_frame_info(&frame, "test")
                .expect("deterministic bitmap frame must be complete")
                .action,
            Action::FastPath
        );
    }

    #[test]
    fn rejects_malformed_active_response() {
        let mut semantic_outputs = ActiveOutputValidator::new(&[]);
        assert!(
            validate_active_outputs(
                &[ActiveStageOutput::ResponseFrame(vec![1])],
                Some(&mut Sha256::new()),
                &mut Vec::new(),
                &mut semantic_outputs,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_unexpected_active_semantic_output() {
        let mut semantic_outputs = ActiveOutputValidator::new(&[]);
        assert!(
            validate_active_outputs(
                &[ActiveStageOutput::PointerDefault],
                None,
                &mut Vec::new(),
                &mut semantic_outputs,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_fixed_contract_mismatch() {
        let contract = ConnectorReplayContract {
            expected_measurement: ConnectorReplayMeasurement {
                connector_steps: 1,
                outbound_frames: 1,
                active_frames: 1,
                active_x224_frames: 1,
                active_fast_path_frames: 0,
                active_response_frames: 1,
                graphics_updates: 0,
                deterministic_graphics_updates: 1,
            },
            outbound_prefix: &[],
            session_data_frames: 0,
            semantic_outputs: &[],
            output_fingerprint: [0; 32],
        };
        let measurement = ConnectorReplayMeasurement {
            connector_steps: 2,
            ..contract.expected_measurement
        };
        assert!(contract.validate(measurement, None, &[]).is_err());
    }

    #[test]
    fn semantic_fingerprint_rejects_same_length_difference() {
        assert_ne!(
            license_fingerprint(&encoded_client_new_license_request("first")),
            license_fingerprint(&encoded_client_new_license_request("other"))
        );
    }

    #[test]
    fn semantic_fingerprint_rejects_platform_id_change() {
        let first = encoded_client_new_license_request("first");
        let mut second = first.clone();
        second[LICENSE_HEADER_SIZE + 4 /* PreferredKeyExchangeAlg */..CLIENT_NEW_LICENSE_REQUEST_RANDOM_OFFSET]
            .copy_from_slice(&0x0402_1234u32.to_le_bytes());
        assert_ne!(license_fingerprint(&first), license_fingerprint(&second));
    }

    #[test]
    fn semantic_fingerprint_ignores_only_license_secrets() {
        let first = encoded_client_new_license_request("first");
        let mut second = first.clone();
        for secret_range in
            client_new_license_request_secret_ranges(&second).expect("test request must contain both licensing secrets")
        {
            second[secret_range].fill(2);
        }
        assert_eq!(license_fingerprint(&first), license_fingerprint(&second));
    }

    fn license_fingerprint(user_data: &[u8]) -> [u8; 32] {
        let license = decode::<LicensePdu>(user_data).expect("test request must decode");
        let LicensePdu::ClientNewLicenseRequest(request) = license else {
            panic!("test request must be a Client New License Request");
        };
        let mut fingerprint = Sha256::new();
        update_client_new_license_request_fingerprint(user_data, &request, &mut fingerprint)
            .expect("test request must fingerprint");
        fingerprint.finalize().into()
    }

    fn encoded_client_new_license_request(client_username: &str) -> Vec<u8> {
        let request = ClientNewLicenseRequest {
            license_header: ironrdp::pdu::rdp::server_license::LicenseHeader {
                security_header: BasicSecurityHeader {
                    flags: BasicSecurityHeaderFlags::LICENSE_PKT,
                },
                preamble_message_type: PreambleType::NewLicenseRequest,
                preamble_flags: PreambleFlags::empty(),
                preamble_version: PreambleVersion::V3,
                preamble_message_size: 128,
            },
            client_random: vec![1; 32],
            encrypted_premaster_secret: vec![1; 64],
            client_username: client_username.to_owned(),
            client_machine_name: "machine".to_owned(),
        };
        encode_vec(&LicensePdu::from(request)).expect("test request must encode")
    }
}
