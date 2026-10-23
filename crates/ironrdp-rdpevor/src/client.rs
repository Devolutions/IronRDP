//! Client processors for the MS-RDPEVOR control and data channels.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::pdu::{PresentationCommand, PresentationRequest, PresentationResponse, VideoData};
use crate::presenter::VideoPresenter;
use crate::{CONTROL_CHANNEL_NAME, DATA_CHANNEL_NAME};

/// Presenter shared by the control and data channels and the code compositing its pictures.
pub type SharedPresenter = Arc<Mutex<VideoPresenter>>;

fn lock(presenter: &Mutex<VideoPresenter>) -> MutexGuard<'_, VideoPresenter> {
    presenter.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Client processor for `Microsoft::Windows::RDS::Video::Control::v08.01`.
///
/// It answers a Start Presentation request once the presenter has a decoder for it, and
/// forwards Stop Presentation requests.
pub struct VideoControlClient {
    presenter: SharedPresenter,
    channel_id: Option<u32>,
}

impl VideoControlClient {
    pub fn new(presenter: SharedPresenter) -> Self {
        Self {
            presenter,
            channel_id: None,
        }
    }

    /// Presenter that decodes and schedules this channel's video.
    pub fn presenter(&self) -> &SharedPresenter {
        &self.presenter
    }

    /// Encodes the notifications the presenter queued for the server.
    pub fn pending_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        let Some(channel_id) = self.channel_id else {
            return Ok(Vec::new());
        };
        let notifications = lock(&self.presenter).take_notifications();
        if notifications.is_empty() {
            return Ok(Vec::new());
        }
        let messages = notifications
            .into_iter()
            .map(|notification| -> DvcMessage { Box::new(notification) })
            .collect();
        encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|error| encode_err!(error))
    }
}

impl_as_any!(VideoControlClient);

impl DvcProcessor for VideoControlClient {
    fn channel_name(&self) -> &str {
        CONTROL_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        debug!(channel_id, "Video control channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let request: PresentationRequest = match decode(payload) {
            Ok(request) => request,
            Err(error) => {
                warn!(%error, "Ignoring malformed video control message");
                return Ok(Vec::new());
            }
        };
        let mut presenter = lock(&self.presenter);
        match request.command {
            PresentationCommand::START => {
                if presenter.start(&request) {
                    return Ok(vec![Box::new(PresentationResponse {
                        presentation_id: request.presentation_id,
                    })]);
                }
            }
            PresentationCommand::STOP => presenter.stop(request.presentation_id),
            PresentationCommand(command) => warn!(command, "Ignoring an unknown video presentation command"),
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        lock(&self.presenter).reset();
        debug!("Video control channel closed");
    }
}

impl DvcClientProcessor for VideoControlClient {}

/// Client processor for `Microsoft::Windows::RDS::Video::Data::v08.01`.
pub struct VideoDataClient {
    presenter: SharedPresenter,
}

impl VideoDataClient {
    pub fn new(presenter: SharedPresenter) -> Self {
        Self { presenter }
    }
}

impl_as_any!(VideoDataClient);

impl DvcProcessor for VideoDataClient {
    fn channel_name(&self) -> &str {
        DATA_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!(channel_id, "Video data channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode::<VideoData<'_>>(payload) {
            Ok(data) => lock(&self.presenter).push_packet(&data),
            Err(error) => warn!(%error, "Ignoring malformed video data message"),
        }
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for VideoDataClient {}
