//! Capture-backed, connector-driven offline workloads.
//!
//! These workloads retain the passive partial replay as a strict preflight, but
//! separately drive the production `ClientConnector` from X.224 negotiation to
//! `ConnectionResult`. TLS is an explicit completed boundary: no TLS handshake
//! or authentication traffic is measured or claimed.

use core::str::FromStr;
use std::fmt;

use ironrdp::connector::{ClientConnector, ClientConnectorState, Config, Credentials, DesktopSize, Sequence as _};
use ironrdp::core::{WriteBuf, decode, encode_vec};
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp::pdu::x224::{X224, X224Data};
use ironrdp::pdu::{Action, find_size, gcc, mcs, nego};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput};
use ironrdp::svc::{SvcClientProcessor, SvcMessage, SvcProcessor, impl_as_any};
use ironrdp_capture_replay::{PacketStream, decrypt_tls, read_capture};
use sha2::{Digest as _, Sha256};

use crate::replay::{PartialReplayId, PartialReplayWorkload};

const MAX_CONNECTOR_STEPS: usize = 128;
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
    expected: ConnectorReplayMeasurement,
}

impl ConnectorReplayWorkload {
    /// Verify, read, decrypt, and prepare the selected local capture.
    ///
    /// The strict passive replay preflight and all capture processing happen
    /// outside the focused connection-and-session measurement.
    pub fn prepare(id: ConnectorReplayId) -> ConnectorReplayResult<Self> {
        let passive = PartialReplayWorkload::prepare(PartialReplayId::NoNlaAccepted)
            .map_err(|error| ConnectorReplayError::new(error.to_string()))?;
        passive
            .verify()
            .map_err(|error| ConnectorReplayError::new(error.to_string()))?;

        let path = PartialReplayWorkload::cached_capture_path(PartialReplayId::NoNlaAccepted)
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
        if server_frames.is_empty() {
            return Err(ConnectorReplayError::new(
                "connector replay has no decrypted server frames".to_owned(),
            ));
        }

        let workload = Self {
            id,
            raw_server_confirm,
            server_frames,
            config,
            channel_names,
            expected: ConnectorReplayMeasurement::default(),
        };
        let expected = workload.replay_once()?;
        let repeated = workload.replay_once()?;
        if repeated != expected {
            return Err(ConnectorReplayError::new(format!(
                "connector replay did not reset to stable output: expected {expected:?}, got {repeated:?}"
            )));
        }
        Ok(Self { expected, ..workload })
    }

    /// Run one fresh connector and active-session execution.
    ///
    /// This includes constructing the connector, driving connection
    /// establishment, constructing the active stage from its result, and
    /// processing every decrypted server frame. TLS and CredSSP are not run.
    pub fn replay(&self) -> ConnectorReplayResult<ConnectorReplayMeasurement> {
        let measurement = self.replay_once()?;
        if measurement != self.expected {
            return Err(ConnectorReplayError::new(
                "connector replay output no longer matches its preflight".to_owned(),
            ));
        }
        Ok(measurement)
    }

    fn replay_once(&self) -> ConnectorReplayResult<ConnectorReplayMeasurement> {
        let mut connector = ClientConnector::new(self.config.clone(), "127.0.0.1:3389".parse().unwrap());
        for name in &self.channel_names {
            if !connector.attach_dynamic_static_channel(OpaqueStaticChannel { name: name.clone() }) {
                return Err(ConnectorReplayError::new(
                    "connector replay could not attach captured static channel".to_owned(),
                ));
            }
        }

        let mut output_fingerprint = Sha256::new();
        let mut outbound_frames = 0;
        let mut connector_steps = 0;

        let initial_request = step_no_input(&mut connector, "send connection request")?;
        connector_steps += 1;
        outbound_frames += validate_outbound_frames(&initial_request, "connection request", &mut output_fingerprint)?;
        validate_connection_request(&initial_request, &self.config)?;

        let confirm_output = step_input(&mut connector, &self.raw_server_confirm, "receive connection confirm")?;
        connector_steps += 1;
        outbound_frames += validate_outbound_frames(&confirm_output, "connection confirm", &mut output_fingerprint)?;
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
            outbound_frames += validate_outbound_frames(&output, "connector response", &mut output_fingerprint)?;
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
                    graphics_updates += outputs
                        .iter()
                        .filter(|output| matches!(output, ActiveStageOutput::GraphicsUpdate(_)))
                        .count();
                    active_response_frames += validate_active_outputs(&outputs, &mut output_fingerprint)?;
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
        let deterministic_graphics_updates = deterministic_outputs
            .iter()
            .filter(|output| matches!(output, ActiveStageOutput::GraphicsUpdate(_)))
            .count();
        active_response_frames += validate_active_outputs(&deterministic_outputs, &mut output_fingerprint)?;
        if deterministic_graphics_updates != 1 {
            return Err(ConnectorReplayError::new(format!(
                "deterministic active-stage bitmap produced {deterministic_graphics_updates} graphics updates"
            )));
        }
        Ok(ConnectorReplayMeasurement {
            connector_steps,
            outbound_frames,
            active_frames,
            active_x224_frames,
            active_fast_path_frames,
            active_response_frames,
            graphics_updates,
            deterministic_graphics_updates,
            output_fingerprint: output_fingerprint.finalize().into(),
        })
    }

    /// Stable workload identifier.
    pub const fn id(&self) -> ConnectorReplayId {
        self.id
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
    /// SHA-256 over the deterministic semantic envelope of outbound frames.
    pub output_fingerprint: [u8; 32],
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
    output_fingerprint: &mut Sha256,
) -> ConnectorReplayResult<usize> {
    if frames.is_empty() {
        return Ok(0);
    }
    let mut offset = 0;
    let mut count = 0;
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
        update_outbound_fingerprint(&frames[offset..end], output_fingerprint)?;
        offset = end;
        count += 1;
    }
    Ok(count)
}

fn update_outbound_fingerprint(frame: &[u8], output_fingerprint: &mut Sha256) -> ConnectorReplayResult<()> {
    if decode::<X224<nego::ConnectionRequest>>(frame).is_ok() {
        output_fingerprint.update(frame);
        return Ok(());
    }
    let payload = decode::<X224<X224Data<'_>>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode outbound X.224 frame: {error}")))?
        .0;
    if decode::<mcs::ConnectInitial>(payload.data.as_ref()).is_ok() {
        output_fingerprint.update(b"ConnectInitial");
        output_fingerprint.update((frame.len() as u64).to_le_bytes());
        return Ok(());
    }
    let message = decode::<X224<mcs::McsMessage<'_>>>(frame)
        .map_err(|error| ConnectorReplayError::new(format!("decode outbound MCS frame: {error}")))?;
    output_fingerprint.update((frame.len() as u64).to_le_bytes());
    match message.0 {
        mcs::McsMessage::ErectDomainRequest(_) => output_fingerprint.update(b"ErectDomainRequest"),
        mcs::McsMessage::AttachUserRequest(_) => output_fingerprint.update(b"AttachUserRequest"),
        mcs::McsMessage::ChannelJoinRequest(_) => output_fingerprint.update(b"ChannelJoinRequest"),
        mcs::McsMessage::SendDataRequest(data) => {
            // The production license state machine generates fresh secrets for
            // its ClientNewLicenseRequest. Retain a deterministic envelope for
            // every SendData request so the fingerprint remains comparable.
            output_fingerprint.update(b"SendDataRequest");
            output_fingerprint.update(data.initiator_id.to_le_bytes());
            output_fingerprint.update(data.channel_id.to_le_bytes());
            output_fingerprint.update((data.user_data.len() as u64).to_le_bytes());
        }
        _ => {
            return Err(ConnectorReplayError::new(
                "connector replay produced an unexpected server-direction MCS frame".to_owned(),
            ));
        }
    }
    Ok(())
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
    output_fingerprint: &mut Sha256,
) -> ConnectorReplayResult<usize> {
    let mut response_frames = 0;
    for output in outputs {
        if let ActiveStageOutput::ResponseFrame(frame) = output {
            output_fingerprint.update(frame);
            response_frames += validate_outbound_frames(frame, "active-stage response", output_fingerprint)?;
        }
    }
    Ok(response_frames)
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
        assert!(validate_active_outputs(&[ActiveStageOutput::ResponseFrame(vec![1])], &mut Sha256::new()).is_err());
    }
}
