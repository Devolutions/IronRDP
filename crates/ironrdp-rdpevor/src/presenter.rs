//! Video presentation: H.264 decoding, presentation timing, and placement on the desktop.
//!
//! The presenter performs no I/O. Time comes from a caller-supplied monotonic [`Clock`], and the
//! caller composites the returned [`VideoUpdate`]s into its framebuffer.

use std::collections::{BTreeMap, VecDeque};

use ironrdp_egfx::decode::{DecodedFrame, H264Decoder};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_rdpegt::{GeometryRect, MappedGeometry};
use tracing::{debug, info, trace, warn};

use crate::pdu::{ClientNotification, MF_VIDEO_FORMAT_H264, Notification, PresentationRequest, VideoData};

/// Creates an H.264 decoder for a new presentation, or `None` when none is available.
pub type DecoderFactory = Box<dyn FnMut() -> Option<Box<dyn H264Decoder>> + Send>;

/// Monotonic clock, in 100-ns units.
pub type Clock = Box<dyn Fn() -> u64 + Send>;

/// Largest accepted video width or height.
pub const MAX_VIDEO_DIMENSION: u32 = 8192;
/// Largest accepted reassembled sample.
pub const MAX_SAMPLE_SIZE: usize = 16 * 1024 * 1024;
/// Largest number of geometry mappings mirrored by the presenter.
pub const MAX_GEOMETRIES: usize = ironrdp_rdpegt::client::MAX_MAPPINGS;

/// Largest number of decoded pictures waiting for their presentation time.
const MAX_QUEUED_FRAMES: usize = 8;
/// A picture due longer ago than this resynchronizes the presentation clock (500 ms).
const LATE_RESYNC: u64 = 5_000_000;
/// A picture due further ahead than this resynchronizes the presentation clock (2 s).
const AHEAD_RESYNC: u64 = 20_000_000;
/// Shortest interval between two network error notifications (1 s).
const NOTIFICATION_INTERVAL: u64 = 10_000_000;

/// Pixels to composite into the desktop framebuffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoUpdate {
    /// Destination on the desktop.
    pub region: ExclusiveRectangle,
    /// Tightly packed RGBA rows covering `region`.
    pub data: Vec<u8>,
}

struct PendingFrame {
    publish_at: u64,
    frame: DecodedFrame,
}

/// A sample being reassembled from its packets.
struct Assembly {
    sample_number: u32,
    next_index: u16,
    packets: u16,
    flags: u8,
    timestamp: u64,
    duration: u64,
    data: Vec<u8>,
}

struct Presentation {
    id: u8,
    source_width: u32,
    source_height: u32,
    scaled_width: u32,
    scaled_height: u32,
    mapping_id: u64,
    sequence_header: Vec<u8>,
    decoder: Box<dyn H264Decoder>,
    assembly: Option<Assembly>,
    /// Local time and presentation time of the clock origin.
    origin: Option<(u64, u64)>,
    last_timestamp: u64,
    awaiting_key_frame: bool,
    frames: VecDeque<PendingFrame>,
    presented: u64,
    dropped: u64,
}

/// Decodes and schedules the pictures of the active video presentation.
///
/// The protocol allows one presentation at a time ([3.1]).
///
/// [3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
pub struct VideoPresenter {
    decoder_factory: DecoderFactory,
    clock: Clock,
    presentation: Option<Presentation>,
    geometries: BTreeMap<u64, MappedGeometry>,
    notifications: Vec<ClientNotification>,
    last_notification: Option<u64>,
}

impl VideoPresenter {
    pub fn new(decoder_factory: DecoderFactory, clock: Clock) -> Self {
        Self {
            decoder_factory,
            clock,
            presentation: None,
            geometries: BTreeMap::new(),
            notifications: Vec::new(),
            last_notification: None,
        }
    }

    /// Identifier of the active presentation.
    pub fn presentation_id(&self) -> Option<u8> {
        self.presentation.as_ref().map(|presentation| presentation.id)
    }

    /// Number of pictures shown and dropped in the active presentation.
    pub fn statistics(&self) -> (u64, u64) {
        self.presentation
            .as_ref()
            .map_or((0, 0), |presentation| (presentation.presented, presentation.dropped))
    }

    /// Prepares a presentation for a Start Presentation request.
    ///
    /// Returns whether the client is ready to render it, in which case the server is told to
    /// start streaming. Requests for another subtype, malformed requests, a restart of the active
    /// presentation, and a missing decoder are refused.
    pub fn start(&mut self, request: &PresentationRequest) -> bool {
        if request.video_subtype != MF_VIDEO_FORMAT_H264 {
            warn!(
                presentation_id = request.presentation_id,
                "Ignoring a video presentation that is not H.264"
            );
            return false;
        }
        let dimensions = [
            request.source_width,
            request.source_height,
            request.scaled_width,
            request.scaled_height,
        ];
        if dimensions
            .iter()
            .any(|dimension| *dimension == 0 || MAX_VIDEO_DIMENSION < *dimension)
        {
            warn!(?dimensions, "Ignoring a video presentation with invalid dimensions");
            return false;
        }
        if self.presentation_id() == Some(request.presentation_id) {
            debug!(
                presentation_id = request.presentation_id,
                "Ignoring a restart of the active video presentation"
            );
            return false;
        }
        let Some(decoder) = (self.decoder_factory)() else {
            warn!("No H.264 decoder is available for the video presentation");
            return false;
        };
        if let Some(previous) = self.presentation.take() {
            info!(presentation_id = previous.id, "Replacing the active video presentation");
        }
        info!(
            presentation_id = request.presentation_id,
            source_width = request.source_width,
            source_height = request.source_height,
            scaled_width = request.scaled_width,
            scaled_height = request.scaled_height,
            mapping_id = request.geometry_mapping_id,
            "Video presentation started"
        );
        self.presentation = Some(Presentation {
            id: request.presentation_id,
            source_width: request.source_width,
            source_height: request.source_height,
            scaled_width: request.scaled_width,
            scaled_height: request.scaled_height,
            mapping_id: request.geometry_mapping_id,
            sequence_header: request.extra_data.clone(),
            decoder,
            assembly: None,
            origin: None,
            last_timestamp: 0,
            awaiting_key_frame: true,
            frames: VecDeque::new(),
            presented: 0,
            dropped: 0,
        });
        true
    }

    /// Ends the presentation with this identifier.
    pub fn stop(&mut self, presentation_id: u8) {
        if self.presentation_id() == Some(presentation_id) {
            if let Some(presentation) = self.presentation.take() {
                info!(
                    presentation_id,
                    presented = presentation.presented,
                    dropped = presentation.dropped,
                    "Video presentation stopped"
                );
            }
        } else {
            debug!(presentation_id, "Ignoring a stop for an unknown video presentation");
        }
    }

    /// Ends any presentation, for example when its channel closes.
    pub fn reset(&mut self) {
        if let Some(presentation) = self.presentation.take() {
            info!(presentation_id = presentation.id, "Video presentation ended");
        }
    }

    /// Mirrors a geometry mapping change from the MS-RDPEGT channel. `None` clears the mapping.
    pub fn set_geometry(&mut self, mapping_id: u64, geometry: Option<MappedGeometry>) {
        match geometry {
            Some(geometry) => {
                if self.geometries.len() < MAX_GEOMETRIES || self.geometries.contains_key(&mapping_id) {
                    self.geometries.insert(mapping_id, geometry);
                }
            }
            None => {
                self.geometries.remove(&mapping_id);
            }
        }
    }

    /// Adds one packet of a video sample and decodes the sample once it is complete.
    pub fn push_packet(&mut self, data: &VideoData<'_>) {
        let Some(presentation) = self.presentation.as_mut() else {
            trace!(
                presentation_id = data.presentation_id,
                "Ignoring video data without a presentation"
            );
            return;
        };
        if presentation.id != data.presentation_id {
            trace!(
                presentation_id = data.presentation_id,
                "Ignoring video data for another presentation"
            );
            return;
        }

        let continues = presentation.assembly.as_ref().is_some_and(|assembly| {
            assembly.sample_number == data.sample_number
                && assembly.next_index == data.current_packet_index
                && assembly.packets == data.packets_in_sample
        });
        if data.current_packet_index == 1 {
            if presentation.assembly.take().is_some() {
                self.report_loss("video sample interrupted by a new sample");
                return self.push_packet(data);
            }
            presentation.assembly = Some(Assembly {
                sample_number: data.sample_number,
                next_index: 1,
                packets: data.packets_in_sample,
                flags: data.flags,
                timestamp: data.timestamp,
                duration: data.duration,
                data: Vec::new(),
            });
        } else if !continues {
            presentation.assembly = None;
            self.report_loss("video sample packet missing or out of order");
            return;
        }

        let Some(assembly) = presentation.assembly.as_mut() else {
            return;
        };
        if MAX_SAMPLE_SIZE - assembly.data.len() < data.sample.len() {
            presentation.assembly = None;
            self.report_loss("video sample exceeds the supported size");
            return;
        }
        assembly.data.extend_from_slice(data.sample);
        assembly.flags |= data.flags;
        if data.current_packet_index < data.packets_in_sample {
            assembly.next_index = data.current_packet_index + 1;
            return;
        }
        if let Some(assembly) = presentation.assembly.take() {
            self.decode_sample(assembly);
        }
    }

    /// Local time when the next decoded picture is due.
    pub fn next_deadline(&self) -> Option<u64> {
        self.presentation
            .as_ref()
            .and_then(|presentation| presentation.frames.front())
            .map(|frame| frame.publish_at)
    }

    /// Time until the next decoded picture is due, in 100-ns units.
    pub fn time_to_next_deadline(&self) -> Option<u64> {
        self.next_deadline()
            .map(|deadline| deadline.saturating_sub((self.clock)()))
    }

    /// Returns the pixels to show now on a desktop of the given size.
    ///
    /// When several pictures are due, only the latest is shown. Pictures are only shown while
    /// the presentation's geometry mapping is known, scaled to the tracked rectangle and clipped
    /// to its visible region and to the desktop.
    pub fn poll(&mut self, desktop_width: u16, desktop_height: u16) -> Vec<VideoUpdate> {
        let now = (self.clock)();
        let Some(presentation) = self.presentation.as_mut() else {
            return Vec::new();
        };
        let mut due = None;
        while presentation.frames.front().is_some_and(|frame| frame.publish_at <= now) {
            if due.is_some() {
                presentation.dropped += 1;
            }
            due = presentation.frames.pop_front();
        }
        let Some(due) = due else {
            return Vec::new();
        };
        let Some(geometry) = self.geometries.get(&presentation.mapping_id) else {
            trace!(mapping_id = presentation.mapping_id, "Video geometry is not known yet");
            presentation.dropped += 1;
            return Vec::new();
        };
        presentation.presented += 1;
        compose(presentation, &due.frame, geometry, desktop_width, desktop_height)
    }

    /// Notifications for the server, queued since the last call.
    pub fn take_notifications(&mut self) -> Vec<ClientNotification> {
        core::mem::take(&mut self.notifications)
    }

    fn decode_sample(&mut self, assembly: Assembly) {
        let now = (self.clock)();
        let Some(presentation) = self.presentation.as_mut() else {
            return;
        };
        let key_frame = assembly.flags & VideoData::KEYFRAME != 0;
        if presentation.awaiting_key_frame && !key_frame {
            trace!(
                sample_number = assembly.sample_number,
                "Skipping a video sample until the next key frame"
            );
            return;
        }
        presentation.awaiting_key_frame = false;

        let timestamp = if assembly.flags & VideoData::HAS_TIMESTAMPS != 0 {
            assembly.timestamp
        } else {
            presentation.last_timestamp.saturating_add(assembly.duration)
        };
        presentation.last_timestamp = timestamp;

        let mut data = assembly.data;
        if key_frame && !presentation.sequence_header.is_empty() && !contains_sequence_parameter_set(&data) {
            let mut with_header = Vec::with_capacity(presentation.sequence_header.len() + data.len());
            with_header.extend_from_slice(&presentation.sequence_header);
            with_header.extend_from_slice(&data);
            data = with_header;
        }

        let frame = match presentation.decoder.decode(&data) {
            Ok(frame) => frame,
            Err(error) => {
                debug!(%error, sample_number = assembly.sample_number, "Failed to decode a video sample");
                self.request_key_frame(now);
                return;
            }
        };

        let publish_at = match presentation.origin {
            Some((local, origin)) if origin <= timestamp => local.saturating_add(timestamp - origin),
            _ => now,
        };
        let publish_at = if presentation.origin.is_none()
            || publish_at.saturating_add(LATE_RESYNC) < now
            || now.saturating_add(AHEAD_RESYNC) < publish_at
        {
            presentation.origin = Some((now, timestamp));
            now
        } else {
            publish_at
        };
        if presentation.frames.len() == MAX_QUEUED_FRAMES {
            presentation.frames.pop_front();
            presentation.dropped += 1;
        }
        presentation.frames.push_back(PendingFrame { publish_at, frame });
    }

    fn report_loss(&mut self, reason: &'static str) {
        debug!(reason, "Video sample lost");
        let now = (self.clock)();
        if let Some(presentation) = self.presentation.as_mut() {
            presentation.awaiting_key_frame = true;
        }
        self.request_key_frame(now);
    }

    fn request_key_frame(&mut self, now: u64) {
        let Some(presentation_id) = self.presentation_id() else {
            return;
        };
        if self
            .last_notification
            .is_some_and(|last| now < last.saturating_add(NOTIFICATION_INTERVAL))
        {
            return;
        }
        self.last_notification = Some(now);
        self.notifications.push(ClientNotification {
            presentation_id,
            notification: Notification::NetworkError,
        });
    }
}

impl core::fmt::Debug for VideoPresenter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VideoPresenter")
            .field("presentation_id", &self.presentation_id())
            .field("geometries", &self.geometries.len())
            .finish_non_exhaustive()
    }
}

/// Scales `frame` to the tracked rectangle and clips it to the visible region and the desktop.
fn compose(
    presentation: &Presentation,
    frame: &DecodedFrame,
    geometry: &MappedGeometry,
    desktop_width: u16,
    desktop_height: u16,
) -> Vec<VideoUpdate> {
    let tracked = geometry.desktop_rect();
    let (destination, visible) = if tracked.is_empty() {
        // Without a tracked size, show the picture at its source resolution.
        let destination = GeometryRect {
            left: tracked.left,
            top: tracked.top,
            right: tracked
                .left
                .saturating_add(i32::try_from(presentation.source_width).unwrap_or(i32::MAX)),
            bottom: tracked
                .top
                .saturating_add(i32::try_from(presentation.source_height).unwrap_or(i32::MAX)),
        };
        (destination, vec![destination])
    } else {
        (tracked, geometry.visible_rects())
    };
    let desktop = GeometryRect {
        left: 0,
        top: 0,
        right: i32::from(desktop_width),
        bottom: i32::from(desktop_height),
    };

    // The decoder may pad the picture to whole macroblocks.
    let picture_width = presentation.scaled_width.min(frame.width());
    let picture_height = presentation.scaled_height.min(frame.height());
    let (Ok(destination_width), Ok(destination_height)) =
        (u32::try_from(destination.width()), u32::try_from(destination.height()))
    else {
        return Vec::new();
    };
    if picture_width == 0 || picture_height == 0 || destination_width == 0 || destination_height == 0 {
        return Vec::new();
    }
    let frame_stride = usize::try_from(frame.width()).unwrap_or(0) * 4;
    let pixels = frame.data();

    let mut updates = Vec::new();
    for rect in visible
        .iter()
        .filter_map(|rect| rect.intersect(&destination))
        .filter_map(|rect| rect.intersect(&desktop))
    {
        let (Ok(left), Ok(top), Ok(right), Ok(bottom)) = (
            u16::try_from(rect.left),
            u16::try_from(rect.top),
            u16::try_from(rect.right),
            u16::try_from(rect.bottom),
        ) else {
            continue;
        };
        let width = usize::from(right - left);
        let mut data = Vec::with_capacity(width * usize::from(bottom - top) * 4);
        for y in rect.top..rect.bottom {
            let offset_y = u64::try_from(y - destination.top).unwrap_or(0);
            let source_y = offset_y * u64::from(picture_height) / u64::from(destination_height);
            let row = usize::try_from(source_y).unwrap_or(0) * frame_stride;
            if destination_width == picture_width {
                let start = row + usize::try_from(rect.left - destination.left).unwrap_or(0) * 4;
                match pixels.get(start..start + width * 4) {
                    Some(source) => data.extend_from_slice(source),
                    None => return Vec::new(),
                }
                continue;
            }
            for x in rect.left..rect.right {
                let offset_x = u64::try_from(x - destination.left).unwrap_or(0);
                let source_x = offset_x * u64::from(picture_width) / u64::from(destination_width);
                let start = row + usize::try_from(source_x).unwrap_or(0) * 4;
                match pixels.get(start..start + 4) {
                    Some(source) => data.extend_from_slice(source),
                    None => return Vec::new(),
                }
            }
        }
        updates.push(VideoUpdate {
            region: ExclusiveRectangle {
                left,
                top,
                right,
                bottom,
            },
            data,
        });
    }
    updates
}

/// Returns whether an Annex B H.264 sample carries a sequence parameter set.
fn contains_sequence_parameter_set(data: &[u8]) -> bool {
    data.windows(4)
        .any(|window| window[..3] == [0, 0, 1] && window[3] & 0x1F == 7)
}
