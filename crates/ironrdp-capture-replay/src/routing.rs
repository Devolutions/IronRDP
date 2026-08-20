use core::any::TypeId;
use core::convert::Infallible;
use core::mem;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::BTreeSet;
use std::sync::Arc;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::pdu::DrdynvcServerPdu;
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineClient, GraphicsPipelineHandler, Surface};
use ironrdp_egfx::compositor::OutputUpdate;
use ironrdp_egfx::decode::{DecodedFrame, DecoderError, DecoderResult, H264Decoder};
use ironrdp_egfx::pdu::{CapabilitySet, GfxPdu};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::client_info::{ClientInfoFlags, CompressionType};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{Action, decode_err, find_size, mcs};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_svc::{StaticChannelSet, StaticVirtualChannel, SvcMessage, SvcProcessor};

use crate::tls::{decrypt_tls, decrypt_tls_streams};
use crate::{Capture, NegotiatedState, PacketStream, Plaintext, ReplayError, gateway, recover_negotiated_state};

const MAX_DESKTOP_DIM: u16 = 8192;
const MAX_EGFX_OUTPUT_DIM: u16 = 32_766;
// Match the EGFX compositor's budget for full exported RGBA snapshots.
const MAX_REPLAY_FRAMEBUFFER_BYTES: usize = 256 * 1024 * 1024;

/// Captured connection state used to construct an offline active-stage router.
#[derive(Clone, Debug)]
pub struct CapturedActivation {
    /// RDP state recovered from the captured connection sequence.
    pub state: NegotiatedState,
    /// Bulk compression negotiated for the captured session.
    pub compression_type: Option<CompressionType>,
}

impl From<NegotiatedState> for CapturedActivation {
    fn from(state: NegotiatedState) -> Self {
        Self {
            state,
            compression_type: None,
        }
    }
}

/// Direction of a captured transport stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayDirection {
    /// Client-to-server traffic.
    Client,
    /// Server-to-client traffic.
    Server,
}

/// The routing path selected for a captured RDP PDU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayRoute {
    /// Connection or activation traffic that precedes the recovered active stage.
    Connection,
    /// A client message retained only for ordering and provenance.
    ClientObservation,
    /// A server Fast-Path PDU.
    FastPath,
    /// A server MCS I/O channel PDU.
    IoChannel,
    /// A server MCS message-channel PDU.
    MessageChannel,
    /// A server static virtual channel PDU.
    StaticChannel,
    /// A server MCS message routed outside the captured active channels.
    OtherServerMessage,
}

/// Metadata for a routed captured PDU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayEvent {
    /// Packet number that contributed the first byte of this PDU.
    pub packet: usize,
    /// Captured transport direction.
    pub direction: ReplayDirection,
    /// RDP transport action selected by its framing header.
    pub action: Action,
    /// Routing path used for the PDU.
    pub route: ReplayRoute,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ReplayFrame {
    pub packet: usize,
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

impl core::fmt::Debug for ReplayFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReplayFrame")
            .field("packet", &self.packet)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &"<redacted>")
            .finish()
    }
}

/// Classifies an explicitly recorded replay gap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayGapKind {
    /// Bytes could not be framed as an RDP PDU.
    Framing,
    /// A PDU ended before its declared boundary.
    TruncatedPdu,
    /// A static virtual-channel PDU was malformed.
    StaticChannel,
    /// A DVC message was malformed or could not be attached.
    DynamicChannel,
    /// The active session processor rejected a captured server message.
    Session,
    /// The capture ended before activation or reactivation completed.
    IncompleteActivation,
    /// A protocol path is outside direct TCP replay support.
    Unsupported,
}

/// A safe, payload-free description of an unreplayable captured message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayGap {
    /// Packet number where the gap began.
    pub packet: usize,
    /// Stream direction containing the gap.
    pub direction: ReplayDirection,
    /// Layer that could not be replayed.
    pub kind: ReplayGapKind,
    /// Bytes skipped while resynchronizing after a framing error.
    pub skipped_bytes: usize,
}

/// A dynamic channel recovered from a recorded DVC create request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedDynamicChannel {
    /// Server-assigned dynamic channel ID.
    pub id: u32,
    /// Name sent in the recorded DVC create request.
    pub name: String,
}

/// Ordered replay metadata and explicit gaps.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplayReport {
    /// Routed RDP PDUs in capture order.
    pub events: Vec<ReplayEvent>,
    /// Messages that were not safely replayable.
    pub gaps: Vec<ReplayGap>,
    /// Dynamic channels attached from recorded DVC create requests.
    pub dynamic_channels: Vec<CapturedDynamicChannel>,
}

/// Offline router configured exclusively from captured RDP state.
pub struct ReplayRouter {
    activation: CapturedActivation,
    stage: ActiveStage,
    image: DecodedImage,
    drdynvc_channel_id: Option<u16>,
    attached_dynamic_channels: BTreeSet<u32>,
    egfx_dynamic_channels: BTreeSet<u32>,
    egfx_framebuffer: ReplayFramebuffer,
    dvc_lifecycle: StaticVirtualChannel,
}

impl ReplayRouter {
    /// Build a router from captured activation state without live negotiation.
    pub fn new(activation: CapturedActivation) -> Result<Self, ReplayError> {
        let state = &activation.state;
        if !valid_desktop_dimensions(state.width, state.height) {
            return Err(ReplayError::ContradictoryRoutingState);
        }

        let mut ids = BTreeSet::from([state.user_channel_id, state.io_channel_id]);
        if ids.len() != 2 {
            return Err(ReplayError::ContradictoryRoutingState);
        }
        if let Some(message_channel_id) = state.message_channel_id {
            if !ids.insert(message_channel_id) {
                return Err(ReplayError::ContradictoryRoutingState);
            }
        }
        let mut channels = StaticChannelSet::new();
        let mut drdynvc_channel_id = None;
        for channel in &state.static_channels {
            if !ids.insert(channel.id) {
                return Err(ReplayError::ContradictoryRoutingState);
            }
            if channel.name == DrdynvcClient::NAME {
                if drdynvc_channel_id.replace(channel.id).is_some() {
                    return Err(ReplayError::ContradictoryRoutingState);
                }
                channels.insert(DrdynvcClient::new());
                if channels.get_by_type::<DrdynvcClient>().is_none() {
                    return Err(ReplayError::ContradictoryRoutingState);
                }
                channels.attach_channel_id(TypeId::of::<DrdynvcClient>(), channel.id);
                if channels.get_channel_id_by_type::<DrdynvcClient>() != Some(channel.id) {
                    return Err(ReplayError::ContradictoryRoutingState);
                }
            } else {
                let key = channels
                    .insert_dynamic(OpaqueStaticChannel {
                        name: channel.name.clone(),
                    })
                    .ok_or(ReplayError::ContradictoryRoutingState)?;
                channels.attach_channel_id_by_key(key, channel.id);
                if channels.get_key_by_channel_id(channel.id) != Some(key) {
                    return Err(ReplayError::ContradictoryRoutingState);
                }
            }
        }

        let active_stage = ActiveStageBuilder {
            static_channels: channels,
            user_channel_id: state.user_channel_id,
            io_channel_id: state.io_channel_id,
            message_channel_id: state.message_channel_id,
            share_id: state.share_id,
            compression_type: activation.compression_type,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        Ok(Self {
            image: DecodedImage::new(PixelFormat::RgbA32, state.width, state.height),
            activation,
            stage: active_stage,
            drdynvc_channel_id,
            attached_dynamic_channels: BTreeSet::new(),
            egfx_dynamic_channels: BTreeSet::new(),
            egfx_framebuffer: ReplayFramebuffer::default(),
            dvc_lifecycle: StaticVirtualChannel::new(DynamicChannelDiscovery::default()),
        })
    }

    /// Attach an already-created captured DVC without replaying live creation negotiation.
    pub fn attach_established_dynamic_channel<T>(&mut self, channel_id: u32, channel: T) -> Result<(), ReplayError>
    where
        T: DvcClientProcessor + 'static,
    {
        if !self.attached_dynamic_channels.insert(channel_id) {
            return Err(ReplayError::DynamicChannelAttachment);
        }
        let Some(drdynvc) = self.stage.get_svc_processor_mut::<DrdynvcClient>() else {
            self.attached_dynamic_channels.remove(&channel_id);
            return Err(ReplayError::MissingDrdynvcChannel);
        };
        let result = drdynvc.attach_established_dynamic_channel(channel_id, channel);
        if result.is_err() {
            self.attached_dynamic_channels.remove(&channel_id);
            return Err(ReplayError::DynamicChannelAttachment);
        }
        Ok(())
    }

    /// Route one recorded plaintext capture in packet order.
    ///
    /// Construct a new router for each capture so session and channel state cannot cross capture boundaries.
    pub fn route_plaintext(&mut self, plaintext: &Plaintext) -> ReplayReport {
        match self.route_plaintext_with_frame_sink(plaintext, &mut |_| Ok::<_, Infallible>(())) {
            Ok(report) => report,
            Err(error) => match error {},
        }
    }

    pub(crate) fn route_plaintext_with_frame_sink<E>(
        &mut self,
        plaintext: &Plaintext,
        frame_sink: &mut impl FnMut(ReplayFrame) -> Result<(), E>,
    ) -> Result<ReplayReport, E> {
        let mut report = ReplayReport::default();
        let mut messages = framed_stream(&plaintext.client, ReplayDirection::Client, &mut report.gaps);
        messages.extend(framed_stream(
            &plaintext.server,
            ReplayDirection::Server,
            &mut report.gaps,
        ));
        messages.sort_by_key(|message| message.packet);

        let mut awaiting_finalization = false;
        let mut activated = false;
        let mut ever_activated = false;
        let mut activation_viable = true;
        let mut activation_packet = None;
        let mut first_server_packet = None;
        for message in messages {
            let route = if message.direction == ReplayDirection::Client {
                ReplayRoute::ClientObservation
            } else if !activated {
                first_server_packet.get_or_insert(message.packet);
                if let Some(demand_active) = captured_demand_active(&message, self.activation.state.io_channel_id) {
                    awaiting_finalization = true;
                    activation_packet = Some(message.packet);
                    activation_viable = !ever_activated || self.reactivate(demand_active);
                } else if awaiting_finalization && is_server_font_map(&message, self.activation.state.io_channel_id) {
                    activated = activation_viable;
                    ever_activated |= activated;
                }
                ReplayRoute::Connection
            } else {
                let (route, deactivated) = self.route_server_message(&message, &mut report, frame_sink)?;
                if deactivated {
                    activated = false;
                    awaiting_finalization = false;
                    activation_packet = None;
                }
                route
            };
            report.events.push(ReplayEvent {
                packet: message.packet,
                direction: message.direction,
                action: message.action,
                route,
            });
        }
        if !activated && (activation_packet.is_some() || !ever_activated) {
            if let Some(packet) = activation_packet.or(first_server_packet) {
                report.gaps.push(ReplayGap {
                    packet,
                    direction: ReplayDirection::Server,
                    kind: ReplayGapKind::IncompleteActivation,
                    skipped_bytes: 0,
                });
            }
        }
        report.gaps.sort_by_key(|gap| gap.packet);
        Ok(report)
    }

    fn route_server_message<E>(
        &mut self,
        message: &CapturedPdu,
        report: &mut ReplayReport,
        frame_sink: &mut impl FnMut(ReplayFrame) -> Result<(), E>,
    ) -> Result<(ReplayRoute, bool), E> {
        let route = match message.action {
            Action::FastPath => ReplayRoute::FastPath,
            Action::X224 => match mcs::decode_send_data_indication(&message.bytes) {
                Ok(context) if context.channel_id == self.activation.state.io_channel_id => ReplayRoute::IoChannel,
                Ok(context) if Some(context.channel_id) == self.activation.state.message_channel_id => {
                    ReplayRoute::MessageChannel
                }
                Ok(context)
                    if Some(context.channel_id) == self.drdynvc_channel_id
                        || self
                            .activation
                            .state
                            .static_channels
                            .iter()
                            .any(|channel| channel.id == context.channel_id) =>
                {
                    ReplayRoute::StaticChannel
                }
                _ => ReplayRoute::OtherServerMessage,
            },
        };
        if route == ReplayRoute::StaticChannel {
            self.observe_dvc_lifecycle(message, report);
        }
        if route == ReplayRoute::OtherServerMessage {
            return Ok((route, false));
        }
        match self.stage.process(&mut self.image, message.action, &message.bytes) {
            Ok(outputs) => {
                if outputs
                    .iter()
                    .any(|output| matches!(output, ActiveStageOutput::GraphicsUpdate(_)))
                {
                    frame_sink(ReplayFrame {
                        packet: message.packet,
                        width: self.image.width(),
                        height: self.image.height(),
                        pixels: self.image.data().to_vec(),
                    })?;
                }
                self.drain_egfx_output(message.packet, report, frame_sink)?;
                Ok((
                    route,
                    outputs
                        .iter()
                        .any(|output| matches!(output, ActiveStageOutput::DeactivateAll)),
                ))
            }
            Err(_) => {
                report.gaps.push(ReplayGap {
                    packet: message.packet,
                    direction: ReplayDirection::Server,
                    kind: if route == ReplayRoute::StaticChannel && self.take_unsupported_egfx_codec() {
                        ReplayGapKind::Unsupported
                    } else if route == ReplayRoute::StaticChannel {
                        ReplayGapKind::StaticChannel
                    } else {
                        ReplayGapKind::Session
                    },
                    skipped_bytes: 0,
                });
                Ok((route, false))
            }
        }
    }

    fn reactivate(&mut self, demand_active: CapturedDemandActive) -> bool {
        if let Some((width, height)) = demand_active.desktop_size {
            if !valid_desktop_dimensions(width, height) {
                return false;
            }
            if self.image.width() != width || self.image.height() != height {
                self.image = DecodedImage::new(PixelFormat::RgbA32, width, height);
            }
            self.activation.state.width = width;
            self.activation.state.height = height;
        }
        let static_channel_chunk_size = self.stage.static_channel_chunk_size();
        if !self.stage.reactivate(
            self.activation.state.io_channel_id,
            self.activation.state.user_channel_id,
            demand_active.share_id,
            false,
            false,
            static_channel_chunk_size,
        ) {
            return false;
        }
        self.activation.state.share_id = demand_active.share_id;
        true
    }

    fn observe_dvc_lifecycle(&mut self, message: &CapturedPdu, report: &mut ReplayReport) {
        let Ok(context) = mcs::decode_send_data_indication(&message.bytes) else {
            return;
        };
        if Some(context.channel_id) != self.drdynvc_channel_id {
            return;
        }
        self.dvc_lifecycle
            .channel_processor_downcast_mut::<DynamicChannelDiscovery>()
            .expect("DynamicChannelDiscovery must retain its concrete type")
            .packet = message.packet;
        if self.dvc_lifecycle.process(context.user_data.as_ref()).is_err() {
            report.gaps.push(ReplayGap {
                packet: message.packet,
                direction: message.direction,
                kind: ReplayGapKind::DynamicChannel,
                skipped_bytes: 0,
            });
            return;
        }
        let changes = self
            .dvc_lifecycle
            .channel_processor_downcast_mut::<DynamicChannelDiscovery>()
            .expect("DynamicChannelDiscovery must retain its concrete type")
            .take_changes();
        for channel_id in changes.closed {
            self.attached_dynamic_channels.remove(&channel_id);
            self.egfx_dynamic_channels.remove(&channel_id);
        }
        for channel in changes.created {
            report.dynamic_channels.push(CapturedDynamicChannel {
                id: channel.id,
                name: channel.name.clone(),
            });
            if self.attached_dynamic_channels.contains(&channel.id) {
                continue;
            }

            let is_egfx = channel.name == ironrdp_egfx::CHANNEL_NAME;
            let result = if is_egfx {
                self.attach_established_dynamic_channel(channel.id, ReplayEgfxChannel::new())
            } else {
                self.attach_established_dynamic_channel(channel.id, OpaqueDynamicChannel { name: channel.name })
            };
            if result.is_ok() {
                if is_egfx {
                    self.egfx_dynamic_channels.insert(channel.id);
                }
            } else {
                report.gaps.push(ReplayGap {
                    packet: channel.packet,
                    direction: ReplayDirection::Server,
                    kind: ReplayGapKind::DynamicChannel,
                    skipped_bytes: 0,
                });
            }
        }
    }

    fn drain_egfx_output<E>(
        &mut self,
        packet: usize,
        report: &mut ReplayReport,
        frame_sink: &mut impl FnMut(ReplayFrame) -> Result<(), E>,
    ) -> Result<(), E> {
        let channel_ids = self.egfx_dynamic_channels.iter().copied().collect::<Vec<_>>();
        for channel_id in channel_ids {
            let Some((reset, output)) = self.stage.get_svc_processor_mut::<DrdynvcClient>().and_then(|drdynvc| {
                drdynvc
                    .get_dvc_by_channel_id_mut::<ReplayEgfxChannel>(channel_id)
                    .map(|mut channel| channel.processor_mut().drain_output())
            }) else {
                continue;
            };

            if let Some((width, height)) = reset {
                if !self.egfx_framebuffer.reset(width, height) {
                    report.gaps.push(ReplayGap {
                        packet,
                        direction: ReplayDirection::Server,
                        kind: ReplayGapKind::Unsupported,
                        skipped_bytes: 0,
                    });
                    continue;
                }
            }
            let mut has_output = false;
            for update in &output {
                has_output |= self.egfx_framebuffer.apply(update);
            }
            if has_output {
                frame_sink(ReplayFrame {
                    packet,
                    width: self.egfx_framebuffer.width,
                    height: self.egfx_framebuffer.height,
                    pixels: self.egfx_framebuffer.pixels.clone(),
                })?;
            }
        }
        Ok(())
    }

    fn take_unsupported_egfx_codec(&mut self) -> bool {
        let channel_ids = self.egfx_dynamic_channels.iter().copied().collect::<Vec<_>>();
        channel_ids.into_iter().any(|channel_id| {
            self.stage
                .get_svc_processor_mut::<DrdynvcClient>()
                .and_then(|drdynvc| {
                    drdynvc
                        .get_dvc_by_channel_id_mut::<ReplayEgfxChannel>(channel_id)
                        .map(|mut channel| channel.processor_mut().take_unsupported_codec())
                })
                .unwrap_or(false)
        })
    }
}

/// Decrypt, recover captured activation state, and route a direct TCP RDP capture.
pub fn replay_capture(capture: &Capture) -> Result<ReplayReport, ReplayError> {
    let (mut router, plaintext) = prepare_replay_capture(capture)?;
    Ok(router.route_plaintext(&plaintext))
}

pub(crate) fn prepare_replay_capture(capture: &Capture) -> Result<(ReplayRouter, Plaintext), ReplayError> {
    let mut plaintext = decrypt_tls(capture)?;
    if gateway::is_gateway_tunnel(&plaintext) {
        let tunneled = gateway::extract_tunneled_rdp(&plaintext)?;
        plaintext = decrypt_tls_streams(&tunneled.client, &tunneled.server, capture.tls_key_log.as_str())?;
    }
    let router = ReplayRouter::new(CapturedActivation {
        state: recover_negotiated_state(&plaintext)?,
        compression_type: captured_compression_type(&plaintext),
    })?;
    Ok((router, plaintext))
}

fn captured_compression_type(plaintext: &Plaintext) -> Option<CompressionType> {
    let mut gaps = Vec::new();
    framed_stream(&plaintext.client, ReplayDirection::Client, &mut gaps)
        .into_iter()
        .find_map(|message| {
            let request = decode::<X224<mcs::SendDataRequest<'_>>>(&message.bytes).ok()?.0;
            let client_info = decode::<ClientInfoPdu>(request.user_data.as_ref()).ok()?;

            client_info
                .client_info
                .flags
                .contains(ClientInfoFlags::COMPRESSION)
                .then_some(client_info.client_info.compression_type)
        })
}

#[derive(Debug)]
struct CapturedPdu {
    packet: usize,
    direction: ReplayDirection,
    action: Action,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct CapturedDemandActive {
    share_id: u32,
    desktop_size: Option<(u16, u16)>,
}

fn valid_desktop_dimensions(width: u16, height: u16) -> bool {
    width != 0 && height != 0 && width <= MAX_DESKTOP_DIM && height <= MAX_DESKTOP_DIM
}

fn framed_stream(stream: &PacketStream, direction: ReplayDirection, gaps: &mut Vec<ReplayGap>) -> Vec<CapturedPdu> {
    let mut bytes = Vec::new();
    let mut boundaries = Vec::with_capacity(stream.len());
    for (packet, payload) in stream {
        bytes.extend_from_slice(payload);
        boundaries.push((bytes.len(), *packet));
    }
    let packet_at = |offset: usize| {
        boundaries
            .get(boundaries.partition_point(|(end, _)| *end <= offset))
            .map_or(0, |(_, packet)| *packet)
    };
    let mut messages = Vec::new();
    let mut offset = 0;
    let mut unframed = false;
    while offset < bytes.len() {
        match find_size(&bytes[offset..]) {
            Ok(Some(info)) if info.length != 0 && info.length <= bytes.len() - offset => {
                messages.push(CapturedPdu {
                    packet: packet_at(offset),
                    direction,
                    action: info.action,
                    bytes: bytes[offset..offset + info.length].to_vec(),
                });
                offset += info.length;
                unframed = false;
            }
            Ok(Some(_)) | Ok(None) => {
                gaps.push(ReplayGap {
                    packet: packet_at(offset),
                    direction,
                    kind: ReplayGapKind::TruncatedPdu,
                    skipped_bytes: 0,
                });
                break;
            }
            Err(_) => {
                if !unframed {
                    gaps.push(ReplayGap {
                        packet: packet_at(offset),
                        direction,
                        kind: ReplayGapKind::Framing,
                        skipped_bytes: 1,
                    });
                    unframed = true;
                } else if let Some(gap) = gaps.last_mut() {
                    gap.skipped_bytes += 1;
                }
                offset += 1;
            }
        }
    }
    messages
}

fn captured_demand_active(message: &CapturedPdu, io_channel_id: u16) -> Option<CapturedDemandActive> {
    let Ok(message) = mcs::decode_send_data_indication(&message.bytes) else {
        return None;
    };
    let Ok(context) = ironrdp_pdu::rdp::headers::decode_share_control(message) else {
        return None;
    };
    if context.channel_id != io_channel_id {
        return None;
    }
    let ironrdp_pdu::rdp::headers::ShareControlPdu::ServerDemandActive(demand_active) = context.pdu else {
        return None;
    };
    let desktop_size = demand_active.pdu.capability_sets.iter().find_map(|capability| {
        let ironrdp_pdu::rdp::capability_sets::CapabilitySet::Bitmap(bitmap) = capability else {
            return None;
        };
        Some((bitmap.desktop_width, bitmap.desktop_height))
    });
    Some(CapturedDemandActive {
        share_id: context.share_id,
        desktop_size,
    })
}

fn is_server_font_map(message: &CapturedPdu, io_channel_id: u16) -> bool {
    let Ok(message) = mcs::decode_send_data_indication(&message.bytes) else {
        return false;
    };
    ironrdp_pdu::rdp::headers::decode_share_control(message).is_ok_and(|context| {
        context.channel_id == io_channel_id
            && matches!(
                context.pdu,
                ironrdp_pdu::rdp::headers::ShareControlPdu::Data(ironrdp_pdu::rdp::headers::ShareDataHeader {
                    share_data_pdu: ironrdp_pdu::rdp::headers::ShareDataPdu::FontMap(_),
                    ..
                })
            )
    })
}

#[derive(Clone, Debug)]
struct DiscoveredDynamicChannel {
    packet: usize,
    id: u32,
    name: String,
}

#[derive(Debug, Default)]
struct DynamicChannelDiscovery {
    packet: usize,
    active_ids: BTreeSet<u32>,
    created: Vec<DiscoveredDynamicChannel>,
    closed: Vec<u32>,
}

impl_as_any!(DynamicChannelDiscovery);

impl SvcProcessor for DynamicChannelDiscovery {
    fn channel_name(&self) -> ironrdp_pdu::gcc::ChannelName {
        DrdynvcClient::NAME
    }

    fn process(&mut self, payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        match decode(payload).map_err(|error| decode_err!(error))? {
            DrdynvcServerPdu::Create(create) => {
                if !self.active_ids.insert(create.channel_id()) {
                    return Err(ironrdp_pdu::pdu_other_err!("duplicate captured dynamic channel ID"));
                }
                self.created.push(DiscoveredDynamicChannel {
                    packet: self.packet,
                    id: create.channel_id(),
                    name: create.channel_name().to_owned(),
                });
            }
            DrdynvcServerPdu::Close(close) => {
                let channel_id = close.channel_id();
                self.active_ids.remove(&channel_id);
                self.closed.push(channel_id);
            }
            _ => {}
        }
        Ok(Vec::new())
    }
}

impl DynamicChannelDiscovery {
    fn take_changes(&mut self) -> DynamicChannelChanges {
        DynamicChannelChanges {
            created: mem::take(&mut self.created),
            closed: mem::take(&mut self.closed),
        }
    }
}

#[derive(Debug, Default)]
struct DynamicChannelChanges {
    created: Vec<DiscoveredDynamicChannel>,
    closed: Vec<u32>,
}

#[derive(Debug)]
struct OpaqueStaticChannel {
    name: ironrdp_pdu::gcc::ChannelName,
}

impl_as_any!(OpaqueStaticChannel);

impl SvcProcessor for OpaqueStaticChannel {
    fn channel_name(&self) -> ironrdp_pdu::gcc::ChannelName {
        self.name.clone()
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

#[derive(Debug)]
struct OpaqueDynamicChannel {
    name: String,
}

impl_as_any!(OpaqueDynamicChannel);

impl DvcProcessor for OpaqueDynamicChannel {
    fn channel_name(&self) -> &str {
        &self.name
    }

    fn start(&mut self, _channel_id: u32) -> ironrdp_pdu::PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for OpaqueDynamicChannel {}

struct ReplayGraphicsOutput {
    width: AtomicU32,
    height: AtomicU32,
    reset_generation: AtomicUsize,
    unsupported_avc420: AtomicBool,
}

struct ReplayGraphicsHandler {
    output: Arc<ReplayGraphicsOutput>,
}

impl GraphicsPipelineHandler for ReplayGraphicsHandler {
    fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {}

    fn on_reset_graphics(&mut self, width: u32, height: u32) {
        self.output.width.store(width, Ordering::SeqCst);
        self.output.height.store(height, Ordering::SeqCst);
        self.output.reset_generation.fetch_add(1, Ordering::SeqCst);
    }

    fn on_surface_created(&mut self, _surface: &Surface) {}

    fn on_surface_deleted(&mut self, _surface_id: u16) {}

    fn on_surface_mapped(&mut self, _surface_id: u16, _x: u32, _y: u32) {}

    fn on_bitmap_updated(&mut self, _update: &BitmapUpdate) {}

    fn on_frame_complete(&mut self, _frame_id: u32) {}

    fn on_close(&mut self) {}

    fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {}
}

struct ReplayEgfxChannel {
    client: GraphicsPipelineClient,
    output: Arc<ReplayGraphicsOutput>,
    observed_reset_generation: usize,
}

impl ReplayEgfxChannel {
    fn new() -> Self {
        let output = Arc::new(ReplayGraphicsOutput {
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            reset_generation: AtomicUsize::new(0),
            unsupported_avc420: AtomicBool::new(false),
        });
        let handler = ReplayGraphicsHandler {
            output: Arc::clone(&output),
        };
        Self {
            client: GraphicsPipelineClient::new(
                Box::new(handler),
                Some(Box::new(UnsupportedH264Decoder {
                    unsupported: Arc::clone(&output),
                })),
            ),
            output,
            observed_reset_generation: 0,
        }
    }

    fn drain_output(&mut self) -> (Option<(u32, u32)>, Vec<OutputUpdate>) {
        let reset_generation = self.output.reset_generation.load(Ordering::SeqCst);
        let reset = (reset_generation != self.observed_reset_generation).then(|| {
            self.observed_reset_generation = reset_generation;
            (
                self.output.width.load(Ordering::SeqCst),
                self.output.height.load(Ordering::SeqCst),
            )
        });
        (reset, self.client.drain_output())
    }

    fn take_unsupported_codec(&mut self) -> bool {
        self.output.unsupported_avc420.swap(false, Ordering::SeqCst)
    }
}

struct UnsupportedH264Decoder {
    unsupported: Arc<ReplayGraphicsOutput>,
}

impl H264Decoder for UnsupportedH264Decoder {
    fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
        self.unsupported.unsupported_avc420.store(true, Ordering::SeqCst);
        Err(DecoderError::msg("AVC420 replay is unsupported"))
    }
}

impl_as_any!(ReplayEgfxChannel);

impl DvcProcessor for ReplayEgfxChannel {
    fn channel_name(&self) -> &str {
        ironrdp_egfx::CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> ironrdp_pdu::PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<DvcMessage>> {
        self.client.process(channel_id, payload).map(|_| Vec::new())
    }

    fn close(&mut self, channel_id: u32) {
        self.client.close(channel_id);
    }
}

impl DvcClientProcessor for ReplayEgfxChannel {}

#[derive(Default)]
struct ReplayFramebuffer {
    width: u16,
    height: u16,
    pixels: Vec<u8>,
}

impl ReplayFramebuffer {
    fn reset(&mut self, width: u32, height: u32) -> bool {
        let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
            self.clear();
            return false;
        };
        if width == 0 || height == 0 || width > MAX_EGFX_OUTPUT_DIM || height > MAX_EGFX_OUTPUT_DIM {
            self.clear();
            return false;
        }
        let Some(pixel_count) = usize::from(width).checked_mul(usize::from(height)) else {
            self.clear();
            return false;
        };
        let Some(byte_count) = pixel_count.checked_mul(4) else {
            self.clear();
            return false;
        };
        if byte_count > MAX_REPLAY_FRAMEBUFFER_BYTES {
            self.clear();
            return false;
        }

        let mut pixels = Vec::new();
        if pixels.try_reserve_exact(byte_count).is_err() {
            self.clear();
            return false;
        }
        pixels.resize(byte_count, 0);
        self.width = width;
        self.height = height;
        self.pixels = pixels;
        true
    }

    fn apply(&mut self, update: &OutputUpdate) -> bool {
        let region = &update.region;
        if region.left >= region.right
            || region.top >= region.bottom
            || region.right > self.width
            || region.bottom > self.height
        {
            return false;
        }
        let region_width = usize::from(region.right - region.left);
        let region_height = usize::from(region.bottom - region.top);
        let Some(row_bytes) = region_width.checked_mul(4) else {
            return false;
        };
        let Some(expected_bytes) = row_bytes.checked_mul(region_height) else {
            return false;
        };
        if update.data.len() != expected_bytes {
            return false;
        }

        for (row, source) in update.data.chunks_exact(row_bytes).enumerate() {
            let Some(y) = usize::from(region.top).checked_add(row) else {
                return false;
            };
            let Some(pixel_offset) = y
                .checked_mul(usize::from(self.width))
                .and_then(|offset| offset.checked_add(usize::from(region.left)))
            else {
                return false;
            };
            let Some(byte_offset) = pixel_offset.checked_mul(4) else {
                return false;
            };
            let Some(byte_end) = byte_offset.checked_add(row_bytes) else {
                return false;
            };
            let Some(destination) = self.pixels.get_mut(byte_offset..byte_end) else {
                return false;
            };
            destination.copy_from_slice(source);
        }
        true
    }

    fn clear(&mut self) {
        self.width = 0;
        self.height = 0;
        self.pixels.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Mutex;

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::pdu::{CreateRequestPdu, DataPdu, DrdynvcDataPdu};
    use ironrdp_egfx::pdu::{
        Avc420Region, CapabilitiesConfirmPdu, CapabilitiesV8Flags, CapabilitySet as EgfxCapabilitySet, Codec1Type,
        CreateSurfacePdu, EndFramePdu, MapSurfaceToOutputPdu, PixelFormat as EgfxPixelFormat, ResetGraphicsPdu,
        StartFramePdu, Timestamp, WireToSurface1Pdu, encode_avc420_bitmap_stream,
    };
    use ironrdp_graphics::zgfx::wrap_uncompressed;
    use ironrdp_pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
    use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
    use ironrdp_pdu::gcc::ChannelName;
    use ironrdp_pdu::geometry::{ExclusiveRectangle, InclusiveRectangle};
    use ironrdp_pdu::mcs::{DisconnectProviderUltimatum, DisconnectReason, McsMessage};
    use ironrdp_pdu::rdp::capability_sets::{
        Bitmap, BitmapDrawingFlags, CapabilitySet, DemandActive, ServerDemandActive,
    };
    use ironrdp_pdu::rdp::client_info::{
        AddressFamily, ClientInfo, Credentials, ExtendedClientInfo, ExtendedClientOptionalInfo,
    };
    use ironrdp_pdu::rdp::finalization_messages::FontPdu;
    use ironrdp_pdu::rdp::headers::{
        BasicSecurityHeader, BasicSecurityHeaderFlags, CompressionFlags, ShareControlHeader, ShareControlPdu,
        ShareDataHeader, ShareDataPdu, StreamPriority,
    };
    use ironrdp_svc::server_encode_svc_messages;

    use super::*;

    #[test]
    fn records_truncated_frames_as_gaps() {
        let mut gaps = Vec::new();
        let messages = framed_stream(&vec![(9, vec![3, 0, 0, 8])], ReplayDirection::Server, &mut gaps);
        assert!(messages.is_empty());
        assert_eq!(gaps[0].packet, 9);
        assert_eq!(gaps[0].kind, ReplayGapKind::TruncatedPdu);
    }

    #[test]
    fn preserves_packet_order_and_captured_static_ids() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let report = router.route_plaintext(&Plaintext {
            client: vec![(
                3,
                encode_vec(&X224(mcs::AttachUserConfirm {
                    result: 0,
                    initiator_id: 1_001,
                }))
                .unwrap(),
            )],
            server: vec![
                (1, demand_active()),
                (2, font_map()),
                (4, static_message(1_006, create(7, "opaque"))),
            ],
        });

        assert_eq!(
            report.events.iter().map(|event| event.packet).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(report.events[2].route, ReplayRoute::ClientObservation);
        assert_eq!(report.events[3].route, ReplayRoute::StaticChannel);
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn streams_graphics_updates_in_capture_order() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let mut frames = Vec::new();
        let report = router
            .route_plaintext_with_frame_sink(
                &Plaintext {
                    client: Vec::new(),
                    server: vec![
                        (1, demand_active()),
                        (2, font_map()),
                        (3, bitmap_fast_path([0x11, 0x22, 0x33, 0xff])),
                        (4, bitmap_fast_path([0x44, 0x55, 0x66, 0xff])),
                    ],
                },
                &mut |frame| {
                    frames.push(frame);
                    Ok::<_, ()>(())
                },
            )
            .unwrap();

        assert_eq!(
            report.events.iter().map(|event| event.packet).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(frames.iter().map(|frame| frame.packet).collect::<Vec<_>>(), vec![3, 4]);
        assert!(frames.iter().all(|frame| (frame.width, frame.height) == (32, 32)));
        assert_eq!(&frames[0].pixels[..4], [0x33, 0x22, 0x11, 0xff]);
        assert_eq!(&frames[1].pixels[..4], [0x66, 0x55, 0x44, 0xff]);
    }

    #[test]
    fn discovers_and_attaches_recorded_dynamic_channels() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![
                (1, demand_active()),
                (2, font_map()),
                (3, static_message(1_005, create(7, "recorded"))),
            ],
        });

        assert_eq!(
            report.dynamic_channels,
            vec![CapturedDynamicChannel {
                id: 7,
                name: "recorded".to_owned(),
            }]
        );
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn routes_recorded_egfx_to_replay_frames() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let mut frames = Vec::new();
        let channel_id = 43;
        let report = router
            .route_plaintext_with_frame_sink(
                &Plaintext {
                    client: Vec::new(),
                    server: vec![
                        (1, demand_active()),
                        (2, font_map()),
                        (3, static_message(1_005, create(channel_id, ironrdp_egfx::CHANNEL_NAME))),
                        (
                            4,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(
                                        &EgfxCapabilitySet::V8 {
                                            flags: CapabilitiesV8Flags::empty(),
                                        },
                                    )),
                                ),
                            ),
                        ),
                        (
                            5,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::ResetGraphics(ResetGraphicsPdu {
                                        width: 3,
                                        height: 1,
                                        monitors: Vec::new(),
                                    }),
                                ),
                            ),
                        ),
                        (
                            6,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::CreateSurface(CreateSurfacePdu {
                                        surface_id: 7,
                                        width: 3,
                                        height: 1,
                                        pixel_format: EgfxPixelFormat::XRgb,
                                    }),
                                ),
                            ),
                        ),
                        (
                            7,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
                                        surface_id: 7,
                                        output_origin_x: 0,
                                        output_origin_y: 0,
                                    }),
                                ),
                            ),
                        ),
                        (
                            8,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::StartFrame(StartFramePdu {
                                        timestamp: Timestamp {
                                            milliseconds: 0,
                                            seconds: 0,
                                            minutes: 0,
                                            hours: 0,
                                        },
                                        frame_id: 9,
                                    }),
                                ),
                            ),
                        ),
                        (
                            9,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::WireToSurface1(WireToSurface1Pdu {
                                        surface_id: 7,
                                        codec_id: Codec1Type::Uncompressed,
                                        pixel_format: EgfxPixelFormat::XRgb,
                                        destination_rectangle: ExclusiveRectangle {
                                            left: 0,
                                            top: 0,
                                            right: 1,
                                            bottom: 1,
                                        },
                                        bitmap_data: vec![0x33, 0x22, 0x11, 0xff],
                                    }),
                                ),
                            ),
                        ),
                        (
                            10,
                            static_message(
                                1_005,
                                egfx_data(channel_id, GfxPdu::EndFrame(EndFramePdu { frame_id: 9 })),
                            ),
                        ),
                        (
                            11,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::StartFrame(StartFramePdu {
                                        timestamp: Timestamp {
                                            milliseconds: 0,
                                            seconds: 0,
                                            minutes: 0,
                                            hours: 0,
                                        },
                                        frame_id: 10,
                                    }),
                                ),
                            ),
                        ),
                        (
                            12,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::WireToSurface1(WireToSurface1Pdu {
                                        surface_id: 7,
                                        codec_id: Codec1Type::Uncompressed,
                                        pixel_format: EgfxPixelFormat::XRgb,
                                        destination_rectangle: ExclusiveRectangle {
                                            left: 1,
                                            top: 0,
                                            right: 2,
                                            bottom: 1,
                                        },
                                        bitmap_data: vec![0x66, 0x55, 0x44, 0xff],
                                    }),
                                ),
                            ),
                        ),
                        (
                            13,
                            static_message(
                                1_005,
                                egfx_data(
                                    channel_id,
                                    GfxPdu::WireToSurface1(WireToSurface1Pdu {
                                        surface_id: 7,
                                        codec_id: Codec1Type::Uncompressed,
                                        pixel_format: EgfxPixelFormat::XRgb,
                                        destination_rectangle: ExclusiveRectangle {
                                            left: 2,
                                            top: 0,
                                            right: 3,
                                            bottom: 1,
                                        },
                                        bitmap_data: vec![0x99, 0x88, 0x77, 0xff],
                                    }),
                                ),
                            ),
                        ),
                        (
                            14,
                            static_message(
                                1_005,
                                egfx_data(channel_id, GfxPdu::EndFrame(EndFramePdu { frame_id: 10 })),
                            ),
                        ),
                    ],
                },
                &mut |frame| {
                    frames.push(frame);
                    Ok::<_, ()>(())
                },
            )
            .unwrap();

        assert_eq!(
            report.dynamic_channels,
            vec![CapturedDynamicChannel {
                id: channel_id,
                name: ironrdp_egfx::CHANNEL_NAME.to_owned(),
            }]
        );
        assert!(report.gaps.is_empty());
        assert_eq!(frames.iter().map(|frame| frame.packet).collect::<Vec<_>>(), [10, 14]);
        assert!(frames.iter().all(|frame| (frame.width, frame.height) == (3, 1)));
        assert_eq!(
            frames[0].pixels,
            [0x11, 0x22, 0x33, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            frames[1].pixels,
            [0x11, 0x22, 0x33, 0xff, 0x44, 0x55, 0x66, 0xff, 0x77, 0x88, 0x99, 0xff]
        );
        assert!(router.egfx_dynamic_channels.contains(&channel_id));
    }

    #[test]
    fn keeps_unknown_recorded_dvcs_opaque() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let mut frames = Vec::new();
        let channel_id = 47;
        let report = router
            .route_plaintext_with_frame_sink(
                &Plaintext {
                    client: Vec::new(),
                    server: vec![
                        (1, demand_active()),
                        (2, font_map()),
                        (3, static_message(1_005, create(channel_id, "unknown"))),
                        (
                            4,
                            static_message(
                                1_005,
                                egfx_data(channel_id, GfxPdu::EndFrame(EndFramePdu { frame_id: 9 })),
                            ),
                        ),
                    ],
                },
                &mut |frame| {
                    frames.push(frame);
                    Ok::<_, ()>(())
                },
            )
            .unwrap();

        assert!(report.gaps.is_empty());
        assert!(frames.is_empty());
        assert!(!router.egfx_dynamic_channels.contains(&channel_id));
        let drdynvc = router.stage.get_svc_processor_mut::<DrdynvcClient>().unwrap();
        assert!(
            drdynvc
                .get_dvc_by_channel_id_mut::<OpaqueDynamicChannel>(channel_id)
                .is_some()
        );
    }

    #[test]
    fn passive_egfx_channel_does_not_start_negotiation() {
        assert!(ReplayEgfxChannel::new().start(43).unwrap().is_empty());
    }

    #[test]
    fn replay_framebuffer_accepts_maximum_egfx_dimensions() {
        let mut framebuffer = ReplayFramebuffer::default();
        assert!(framebuffer.reset(u32::from(MAX_EGFX_OUTPUT_DIM), 1));

        assert_eq!(framebuffer.width, MAX_EGFX_OUTPUT_DIM);
        assert_eq!(framebuffer.height, 1);
        assert_eq!(framebuffer.pixels.len(), usize::from(MAX_EGFX_OUTPUT_DIM) * 4);
    }

    #[test]
    fn reports_unbufferable_egfx_output_as_unsupported() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let channel_id = 47;
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![
                (1, demand_active()),
                (2, font_map()),
                (3, static_message(1_005, create(channel_id, ironrdp_egfx::CHANNEL_NAME))),
                (
                    4,
                    static_message(
                        1_005,
                        egfx_data(
                            channel_id,
                            GfxPdu::ResetGraphics(ResetGraphicsPdu {
                                width: u32::from(MAX_EGFX_OUTPUT_DIM),
                                height: u32::from(MAX_EGFX_OUTPUT_DIM),
                                monitors: Vec::new(),
                            }),
                        ),
                    ),
                ),
            ],
        });

        assert_eq!(
            report.gaps,
            [ReplayGap {
                packet: 4,
                direction: ReplayDirection::Server,
                kind: ReplayGapKind::Unsupported,
                skipped_bytes: 0,
            }]
        );
    }

    #[test]
    fn reports_avc420_as_an_unsupported_replay_gap() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let channel_id = 47;
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![
                (1, demand_active()),
                (2, font_map()),
                (3, static_message(1_005, create(channel_id, ironrdp_egfx::CHANNEL_NAME))),
                (
                    4,
                    static_message(
                        1_005,
                        egfx_data(
                            channel_id,
                            GfxPdu::ResetGraphics(ResetGraphicsPdu {
                                width: 1,
                                height: 1,
                                monitors: Vec::new(),
                            }),
                        ),
                    ),
                ),
                (
                    5,
                    static_message(
                        1_005,
                        egfx_data(
                            channel_id,
                            GfxPdu::CreateSurface(CreateSurfacePdu {
                                surface_id: 7,
                                width: 1,
                                height: 1,
                                pixel_format: EgfxPixelFormat::XRgb,
                            }),
                        ),
                    ),
                ),
                (
                    6,
                    static_message(
                        1_005,
                        egfx_data(
                            channel_id,
                            GfxPdu::WireToSurface1(WireToSurface1Pdu {
                                surface_id: 7,
                                codec_id: Codec1Type::Avc420,
                                pixel_format: EgfxPixelFormat::XRgb,
                                destination_rectangle: ExclusiveRectangle {
                                    left: 0,
                                    top: 0,
                                    right: 1,
                                    bottom: 1,
                                },
                                bitmap_data: encode_avc420_bitmap_stream(&[Avc420Region::full_frame(1, 1, 22)], &[]),
                            }),
                        ),
                    ),
                ),
            ],
        });

        assert_eq!(
            report.gaps,
            [ReplayGap {
                packet: 6,
                direction: ReplayDirection::Server,
                kind: ReplayGapKind::Unsupported,
                skipped_bytes: 0,
            }]
        );
    }

    #[test]
    fn delivers_data_to_an_explicitly_attached_dvc() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        router
            .attach_established_dynamic_channel(
                7,
                TestDvc {
                    received: Arc::clone(&received),
                },
            )
            .unwrap();
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![
                (1, demand_active()),
                (2, font_map()),
                (
                    3,
                    static_message(
                        1_005,
                        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(7, b"data".to_vec()))),
                    ),
                ),
            ],
        });

        assert_eq!(&*received.lock().unwrap(), &[b"start".to_vec(), b"data".to_vec()]);
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn rejects_oversized_captured_desktops() {
        let mut oversized_dimension = activation();
        oversized_dimension.state.width = MAX_DESKTOP_DIM + 1;
        assert!(matches!(
            ReplayRouter::new(oversized_dimension),
            Err(ReplayError::ContradictoryRoutingState)
        ));
    }

    #[test]
    fn rejects_captured_channel_id_collisions() {
        let mut activation = activation();
        activation.state.static_channels[0].id = activation.state.io_channel_id;
        assert!(matches!(
            ReplayRouter::new(activation),
            Err(ReplayError::ContradictoryRoutingState)
        ));
    }

    #[test]
    fn rolls_back_missing_drdynvc_attachment() {
        let mut activation = activation();
        activation.state.static_channels.remove(0);
        let mut router = ReplayRouter::new(activation).unwrap();

        assert!(matches!(
            router.attach_established_dynamic_channel(
                7,
                OpaqueDynamicChannel {
                    name: "test".to_owned()
                }
            ),
            Err(ReplayError::MissingDrdynvcChannel)
        ));
        assert!(matches!(
            router.attach_established_dynamic_channel(
                7,
                OpaqueDynamicChannel {
                    name: "test".to_owned()
                }
            ),
            Err(ReplayError::MissingDrdynvcChannel)
        ));
    }

    #[test]
    fn reports_the_incomplete_activation_packet() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![(17, demand_active())],
        });

        assert!(report.gaps.contains(&ReplayGap {
            packet: 17,
            direction: ReplayDirection::Server,
            kind: ReplayGapKind::IncompleteActivation,
            skipped_bytes: 0,
        }));
    }

    #[test]
    fn reports_framing_bytes_skipped_during_resynchronization() {
        let valid_pdu = encode_vec(&X224(McsMessage::DisconnectProviderUltimatum(
            DisconnectProviderUltimatum::from_reason(DisconnectReason::UserRequested),
        )))
        .unwrap();
        let mut gaps = Vec::new();
        let messages = framed_stream(
            &vec![(9, [vec![1], valid_pdu].concat())],
            ReplayDirection::Server,
            &mut gaps,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(
            gaps,
            vec![ReplayGap {
                packet: 9,
                direction: ReplayDirection::Server,
                kind: ReplayGapKind::Framing,
                skipped_bytes: 1,
            }]
        );
    }

    #[test]
    fn routes_deactivate_all_through_reactivation() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![
                (1, demand_active()),
                (2, font_map()),
                (3, deactivate_all()),
                (4, demand_active_with(0x5566_7788, Some((64, 48)))),
                (5, font_map()),
            ],
        });

        assert_eq!(
            report.events.iter().map(|event| event.route).collect::<Vec<_>>(),
            vec![
                ReplayRoute::Connection,
                ReplayRoute::Connection,
                ReplayRoute::IoChannel,
                ReplayRoute::Connection,
                ReplayRoute::Connection,
            ]
        );
        assert!(report.gaps.is_empty());
        assert_eq!(router.activation.state.share_id, 0x5566_7788);
        assert_eq!((router.image.width(), router.image.height()), (64, 48));
    }

    #[test]
    fn retains_non_session_mcs_messages_without_active_stage_errors() {
        let mut router = ReplayRouter::new(activation()).unwrap();
        let disconnect = encode_vec(&X224(McsMessage::DisconnectProviderUltimatum(
            DisconnectProviderUltimatum::from_reason(DisconnectReason::UserRequested),
        )))
        .unwrap();
        let report = router.route_plaintext(&Plaintext {
            client: Vec::new(),
            server: vec![(1, demand_active()), (2, font_map()), (3, disconnect)],
        });

        assert_eq!(report.events[2].route, ReplayRoute::OtherServerMessage);
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn recovers_client_info_compression_type() {
        let client_info = ClientInfoPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::INFO_PKT,
            },
            client_info: ClientInfo {
                credentials: Credentials {
                    username: String::new(),
                    password: String::new(),
                    domain: None,
                },
                code_page: 0,
                flags: ClientInfoFlags::UNICODE | ClientInfoFlags::COMPRESSION,
                compression_type: CompressionType::K64,
                alternate_shell: String::new(),
                work_dir: String::new(),
                extra_info: ExtendedClientInfo {
                    address_family: AddressFamily::INET,
                    address: String::new(),
                    dir: String::new(),
                    optional_data: ExtendedClientOptionalInfo::default(),
                },
            },
        };
        let user_data = encode_vec(&client_info).unwrap();
        let client_pdu = encode_vec(&X224(McsMessage::SendDataRequest(mcs::SendDataRequest {
            initiator_id: 1_001,
            channel_id: 1_003,
            user_data: Cow::Owned(user_data),
        })))
        .unwrap();

        assert_eq!(
            captured_compression_type(&Plaintext {
                client: vec![(1, client_pdu)],
                server: Vec::new(),
            }),
            Some(CompressionType::K64)
        );
    }

    fn activation() -> CapturedActivation {
        CapturedActivation {
            state: NegotiatedState {
                user_channel_id: 1_001,
                io_channel_id: 1_003,
                message_channel_id: None,
                share_id: 0x1122_3344,
                width: 32,
                height: 32,
                static_channels: vec![
                    crate::StaticChannel {
                        name: DrdynvcClient::NAME,
                        id: 1_005,
                    },
                    crate::StaticChannel {
                        name: ChannelName::from_static(b"opaque\0\0"),
                        id: 1_006,
                    },
                ],
            },
            compression_type: None,
        }
    }

    fn demand_active() -> Vec<u8> {
        demand_active_with(0x1122_3344, None)
    }

    fn demand_active_with(share_id: u32, desktop_size: Option<(u16, u16)>) -> Vec<u8> {
        let capability_sets = desktop_size
            .map(|(desktop_width, desktop_height)| {
                vec![CapabilitySet::Bitmap(Bitmap {
                    pref_bits_per_pix: 32,
                    desktop_width,
                    desktop_height,
                    desktop_resize_flag: false,
                    drawing_flags: BitmapDrawingFlags::empty(),
                })]
            })
            .unwrap_or_default();
        let user_data = encode_vec(&ShareControlHeader {
            pdu_source: 1_001,
            share_id,
            share_control_pdu: ShareControlPdu::ServerDemandActive(ServerDemandActive {
                pdu: DemandActive {
                    source_descriptor: "test".to_owned(),
                    capability_sets,
                },
            }),
        })
        .unwrap();
        encode_vec(&X224(McsMessage::SendDataIndication(mcs::SendDataIndication {
            initiator_id: 1_001,
            channel_id: 1_003,
            user_data: Cow::Owned(user_data),
        })))
        .unwrap()
    }

    fn deactivate_all() -> Vec<u8> {
        let user_data = encode_vec(&ShareControlHeader {
            pdu_source: 1_001,
            share_id: 0x1122_3344,
            share_control_pdu: ShareControlPdu::ServerDeactivateAll(ironrdp_pdu::rdp::headers::ServerDeactivateAll),
        })
        .unwrap();
        encode_vec(&X224(McsMessage::SendDataIndication(mcs::SendDataIndication {
            initiator_id: 1_001,
            channel_id: 1_003,
            user_data: Cow::Owned(user_data),
        })))
        .unwrap()
    }

    fn font_map() -> Vec<u8> {
        let user_data = encode_vec(&ShareControlHeader {
            pdu_source: 1_001,
            share_id: 0x1122_3344,
            share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
                share_data_pdu: ShareDataPdu::FontMap(FontPdu::default()),
                stream_priority: StreamPriority::Medium,
                compression_flags: CompressionFlags::empty(),
                compression_type: CompressionType::K8,
            }),
        })
        .unwrap();
        encode_vec(&X224(McsMessage::SendDataIndication(mcs::SendDataIndication {
            initiator_id: 1_001,
            channel_id: 1_003,
            user_data: Cow::Owned(user_data),
        })))
        .unwrap()
    }

    fn bitmap_fast_path(pixel: [u8; 4]) -> Vec<u8> {
        let bitmap = encode_vec(&BitmapUpdateData {
            rectangles: vec![BitmapData {
                rectangle: InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                width: 1,
                height: 1,
                bits_per_pixel: 32,
                compression_flags: Compression::empty(),
                compressed_data_header: None,
                bitmap_data: &pixel,
            }],
        })
        .unwrap();
        let update = encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::Bitmap,
            compression_flags: None,
            compression_type: None,
            data: &bitmap,
        })
        .unwrap();
        let mut frame = encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), update.len())).unwrap();
        frame.extend_from_slice(&update);
        frame
    }

    fn create(id: u32, name: &str) -> DrdynvcServerPdu {
        DrdynvcServerPdu::Create(CreateRequestPdu::new(id, name.to_owned()))
    }

    fn egfx_data(channel_id: u32, pdu: GfxPdu) -> DrdynvcServerPdu {
        let payload = wrap_uncompressed(&encode_vec(&pdu).unwrap());
        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(channel_id, payload)))
    }

    fn static_message(channel_id: u16, message: DrdynvcServerPdu) -> Vec<u8> {
        server_encode_svc_messages(vec![SvcMessage::from(message)], channel_id, 1_001).unwrap()
    }

    #[derive(Debug)]
    struct TestDvc {
        received: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl_as_any!(TestDvc);

    impl DvcProcessor for TestDvc {
        fn channel_name(&self) -> &str {
            "test"
        }

        fn start(&mut self, _channel_id: u32) -> ironrdp_pdu::PduResult<Vec<DvcMessage>> {
            self.received.lock().unwrap().push(b"start".to_vec());
            Ok(Vec::new())
        }

        fn process(&mut self, _channel_id: u32, payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<DvcMessage>> {
            self.received.lock().unwrap().push(payload.to_vec());
            Ok(Vec::new())
        }
    }

    impl DvcClientProcessor for TestDvc {}
}
