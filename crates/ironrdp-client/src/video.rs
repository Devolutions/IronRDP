//! Video optimized remoting ([MS-RDPEVOR]) with geometry tracking ([MS-RDPEGT]).
//!
//! The channels are only registered when an H.264 decoder is available; otherwise the client
//! keeps declining them and the server falls back to its regular graphics output.
//!
//! [MS-RDPEVOR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
//! [MS-RDPEGT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/

use core::time::Duration;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch};
use ironrdp_rdpegt::{GeometryClient, GeometryEvent};
use ironrdp_rdpevor::{SharedPresenter, VideoControlClient, VideoDataClient, VideoPresenter};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageOutput, SessionResult};
use tracing::{debug, warn};

use crate::h264::H264DecoderFactory;

/// Video optimized remoting configuration.
#[derive(Clone, Debug)]
pub struct VideoConfig {
    /// Accept the video channels when an H.264 decoder is available
    /// ([`crate::config::ConfigBuilder::with_h264_decoder`]).
    ///
    /// On by default; `videoplaybackmode:i:0` turns it off, as in mstsc.
    pub enabled: bool,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Registers the video control, video data, and geometry channels.
pub(crate) fn attach(
    drdynvc: DrdynvcClient,
    config: &VideoConfig,
    decoder: Option<&H264DecoderFactory>,
) -> DrdynvcClient {
    if !config.enabled {
        debug!("Video optimized remoting is disabled");
        return drdynvc;
    }
    let Some(decoder) = decoder.cloned() else {
        debug!("No H.264 decoder; video optimized remoting is disabled");
        return drdynvc;
    };
    // Accepting the channels makes the server send video here instead of graphics updates, so
    // only accept them when a decoder can actually be created.
    if decoder.create().is_none() {
        warn!("The H.264 decoder is unavailable; video optimized remoting is disabled");
        return drdynvc;
    }
    let epoch = Instant::now();
    let presenter: SharedPresenter = Arc::new(Mutex::new(VideoPresenter::new(
        Box::new(move || decoder.create()),
        Box::new(move || u64::try_from(epoch.elapsed().as_nanos() / 100).unwrap_or(u64::MAX)),
    )));
    // The server may close and reopen these channels during a session, so each opening gets a
    // new processor sharing the presenter.
    let geometry_presenter = Arc::clone(&presenter);
    let control_presenter = Arc::clone(&presenter);
    drdynvc
        .with_dynamic_channel_factory(ironrdp_rdpegt::CHANNEL_NAME, move || {
            let presenter = Arc::clone(&geometry_presenter);
            GeometryClient::new().with_observer(Box::new(move |event| {
                let mut presenter = presenter.lock().unwrap_or_else(PoisonError::into_inner);
                match event {
                    GeometryEvent::Updated { mapping_id, geometry } => {
                        presenter.set_geometry(mapping_id, Some(geometry.clone()))
                    }
                    GeometryEvent::Cleared { mapping_id } => presenter.set_geometry(mapping_id, None),
                }
            }))
        })
        .with_dynamic_channel_factory(ironrdp_rdpevor::CONTROL_CHANNEL_NAME, move || {
            VideoControlClient::new(Arc::clone(&control_presenter))
        })
        .with_dynamic_channel_factory(ironrdp_rdpevor::DATA_CHANNEL_NAME, move || {
            VideoDataClient::new(Arc::clone(&presenter))
        })
}

/// When the next decoded picture is due.
pub(crate) fn deadline(active_stage: &ActiveStage) -> Option<tokio::time::Instant> {
    let control = active_stage.get_dvc::<VideoControlClient>()?;
    let presenter = control
        .processor()
        .presenter()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let wait = presenter.time_to_next_deadline()?;
    Some(tokio::time::Instant::now() + Duration::from_nanos(wait.saturating_mul(100)))
}

/// Composites the pictures due now into `image` and queues the server notifications.
///
/// Notifications are only sent when the iteration has no other non-empty dynamic channel batch;
/// otherwise they stay queued for a later iteration. Returns whether `dvc_batch` now holds them,
/// in which case they must be routed for the control channel rather than for the request that
/// produced the iteration.
pub(crate) fn present(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    outputs: &mut Vec<ActiveStageOutput>,
    dvc_batch: &mut Option<DvcMessageBatch>,
) -> SessionResult<bool> {
    let Some(mut control) = active_stage.get_dvc_mut::<VideoControlClient>() else {
        return Ok(false);
    };
    let channel_id = control.channel_id();
    let updates = control
        .processor()
        .presenter()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .poll(image.width(), image.height());
    let mut installed = false;
    if dvc_batch.as_ref().is_none_or(|batch| batch.messages().is_empty()) {
        match control
            .processor_mut()
            .pending_messages()
            .and_then(|messages| DvcMessageBatch::try_new(channel_id, messages))
        {
            Ok(batch) if !batch.messages().is_empty() => {
                *dvc_batch = Some(batch);
                installed = true;
            }
            Ok(_) => {}
            Err(error) => warn!(%error, "Unable to send a video notification"),
        }
    }
    if updates.is_empty() {
        return Ok(installed);
    }
    let region =
        active_stage.composite_rgba32(image, updates.into_iter().map(|update| (update.region, update.data)))?;
    if let Some(region) = region {
        outputs.push(ActiveStageOutput::GraphicsUpdate(region));
    }
    Ok(installed)
}
