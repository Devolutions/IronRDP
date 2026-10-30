//! Client processors for the `WMSAud` and `WMSDL` channels.
//!
//! Each processor keeps the last values the server reported and gives them back when the server initializes the protocol, as [MS-RDPADRV] 3.1.3 "Initialization" describes.
//! The stored values survive the channel closing, so a processor kept across a reconnect restores them without help.
//!
//! [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/

use alloc::boxed::Box;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, warn};

use crate::pdu::{AudioLevelPdu, DataFlow, DriveLetterPdu, SerializedCache, VolumeChange};
use crate::{AUDIO_LEVEL_CHANNEL_NAME, DRIVE_LETTER_CHANNEL_NAME};

/// Storage for the volume levels a session reports.
///
/// The processor calls it from the session loop, so it must return promptly.
pub trait AudioLevelHandler: Send {
    /// The session's volume level for `change.data_flow` changed.
    ///
    /// Store it so that later connections can restore it with [`AudioLevelClient::with_volume`].
    fn volume_changed(&mut self, change: VolumeChange);
}

/// Client processor for the `WMSAud` audio level channel.
pub struct AudioLevelClient {
    handler: Box<dyn AudioLevelHandler>,
    render: Option<VolumeChange>,
    capture: Option<VolumeChange>,
}

impl AudioLevelClient {
    /// Creates a processor with no stored volume levels.
    pub fn new(handler: Box<dyn AudioLevelHandler>) -> Self {
        Self {
            handler,
            render: None,
            capture: None,
        }
    }

    /// Stores the volume level of `change.data_flow`, such as one loaded from the client's persistent storage.
    #[must_use]
    pub fn with_volume(mut self, change: VolumeChange) -> Self {
        *self.slot(change.data_flow) = Some(change);
        self
    }

    /// Returns the stored volume level of `data_flow`.
    pub fn volume(&self, data_flow: DataFlow) -> Option<VolumeChange> {
        match data_flow {
            DataFlow::Render => self.render,
            DataFlow::Capture => self.capture,
        }
    }

    fn slot(&mut self, data_flow: DataFlow) -> &mut Option<VolumeChange> {
        match data_flow {
            DataFlow::Render => &mut self.render,
            DataFlow::Capture => &mut self.capture,
        }
    }
}

impl_as_any!(AudioLevelClient);

impl DvcProcessor for AudioLevelClient {
    fn channel_name(&self) -> &str {
        AUDIO_LEVEL_CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: AudioLevelPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed audio level message");
                return Ok(Vec::new());
            }
        };
        match pdu {
            // The client sends volume levels only in answer to an initialization message ([MS-RDPADRV] 3.1.3.1, 3.1.5),
            // render before capture as in the example of [MS-RDPADRV] 4.1.
            AudioLevelPdu::Started | AudioLevelPdu::RemoteConnect => {
                debug!(?pdu, "Audio level protocol initialized");
                Ok(DataFlow::ALL
                    .into_iter()
                    .filter_map(|data_flow| self.volume(data_flow))
                    .map(|change| -> DvcMessage { Box::new(AudioLevelPdu::VolumeChange(change)) })
                    .collect())
            }
            AudioLevelPdu::VolumeChange(change) => {
                debug!(?change, "Session volume level changed");
                *self.slot(change.data_flow) = Some(change);
                self.handler.volume_changed(change);
                Ok(Vec::new())
            }
        }
    }
}

impl DvcClientProcessor for AudioLevelClient {}

/// Storage for the drive letter mappings a session reports.
///
/// The processor calls it from the session loop, so it must return promptly.
pub trait DriveLetterHandler: Send {
    /// The session's drive letter mappings changed and replace any stored before.
    ///
    /// Store them so that later connections can restore them with [`DriveLetterClient::with_cache`].
    fn cache_changed(&mut self, cache: &SerializedCache);
}

/// Client processor for the `WMSDL` drive letter channel.
pub struct DriveLetterClient {
    handler: Box<dyn DriveLetterHandler>,
    cache: Option<SerializedCache>,
    initialized: bool,
}

impl DriveLetterClient {
    /// Creates a processor with no stored drive letter mappings.
    pub fn new(handler: Box<dyn DriveLetterHandler>) -> Self {
        Self {
            handler,
            cache: None,
            initialized: false,
        }
    }

    /// Stores drive letter mappings, such as ones loaded from the client's persistent storage.
    #[must_use]
    pub fn with_cache(mut self, cache: SerializedCache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Returns the stored drive letter mappings.
    pub fn cache(&self) -> Option<&SerializedCache> {
        self.cache.as_ref()
    }

    /// Whether the server initialized the protocol on the open channel.
    ///
    /// A client that registers this channel must not redirect USB mass storage devices before then ([MS-RDPADRV] 3.1.3.2 "Drive Letter Protocol Initialization").
    ///
    /// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
    pub fn initialized(&self) -> bool {
        self.initialized
    }
}

impl_as_any!(DriveLetterClient);

impl DvcProcessor for DriveLetterClient {
    fn channel_name(&self) -> &str {
        DRIVE_LETTER_CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.initialized = false;
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: DriveLetterPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed drive letter message");
                return Ok(Vec::new());
            }
        };
        match pdu {
            // The client sends its mappings only in answer to the initialization message ([MS-RDPADRV] 3.1.5).
            DriveLetterPdu::Started => {
                debug!("Drive letter protocol initialized");
                self.initialized = true;
                Ok(self
                    .cache
                    .clone()
                    .map(|cache| -> DvcMessage { Box::new(DriveLetterPdu::SerializedCache(cache)) })
                    .into_iter()
                    .collect())
            }
            DriveLetterPdu::SerializedCache(cache) => {
                debug!(entries = cache.entries.len(), "Session drive letter mappings changed");
                self.handler.cache_changed(&cache);
                self.cache = Some(cache);
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.initialized = false;
    }
}

impl DvcClientProcessor for DriveLetterClient {}
