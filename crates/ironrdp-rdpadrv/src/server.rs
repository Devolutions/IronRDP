//! Server processors for the `WMSAud` and `WMSDL` channels.
//!
//! Each processor initializes the protocol when the channel opens and hands the values the client restores to its handler.
//! Reporting later changes to the client is up to the caller, through [`AudioLevelServer::volume_changed`] and [`DriveLetterServer::cache_changed`].

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, warn};

use crate::pdu::{AudioLevelPdu, DriveLetterPdu, SerializedCache, VolumeChange};
use crate::{AUDIO_LEVEL_CHANNEL_NAME, DRIVE_LETTER_CHANNEL_NAME};

/// Session side of the volume levels a client restores.
pub trait AudioLevelServerHandler: Send {
    /// The client asked to set the session's volume level for `change.data_flow`.
    fn volume_requested(&mut self, change: VolumeChange);
}

/// Server processor for the `WMSAud` audio level channel.
pub struct AudioLevelServer {
    handler: Box<dyn AudioLevelServerHandler>,
    reconnected: bool,
}

impl AudioLevelServer {
    /// Creates a processor for a new session, which opens the protocol with [`AudioLevelPdu::Started`].
    pub fn new(handler: Box<dyn AudioLevelServerHandler>) -> Self {
        Self {
            handler,
            reconnected: false,
        }
    }

    /// Opens the protocol with [`AudioLevelPdu::RemoteConnect`] instead, for a client reconnecting to an existing session.
    #[must_use]
    pub fn reconnected(mut self) -> Self {
        self.reconnected = true;
        self
    }

    /// Builds the message reporting a session volume level change to the client.
    pub fn volume_changed(change: VolumeChange) -> DvcMessage {
        Box::new(AudioLevelPdu::VolumeChange(change))
    }
}

impl_as_any!(AudioLevelServer);

impl DvcProcessor for AudioLevelServer {
    fn channel_name(&self) -> &str {
        AUDIO_LEVEL_CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let pdu = if self.reconnected {
            AudioLevelPdu::RemoteConnect
        } else {
            AudioLevelPdu::Started
        };
        Ok(vec![Box::new(pdu)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode(payload) {
            Ok(AudioLevelPdu::VolumeChange(change)) => {
                debug!(?change, "Client restored a volume level");
                self.handler.volume_requested(change);
            }
            Ok(pdu) => warn!(
                ?pdu,
                "Ignoring server-originated audio level message received from client"
            ),
            Err(error) => warn!(%error, "Ignoring malformed audio level message"),
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for AudioLevelServer {}

/// Session side of the drive letter mappings a client restores.
pub trait DriveLetterServerHandler: Send {
    /// The client restored the drive letter mappings it stored from an earlier session.
    fn cache_restored(&mut self, cache: SerializedCache);
}

/// Server processor for the `WMSDL` drive letter channel.
pub struct DriveLetterServer {
    handler: Box<dyn DriveLetterServerHandler>,
}

impl DriveLetterServer {
    /// Creates a processor that opens the protocol with [`DriveLetterPdu::Started`].
    pub fn new(handler: Box<dyn DriveLetterServerHandler>) -> Self {
        Self { handler }
    }

    /// Builds the message reporting the session's drive letter mappings to the client.
    pub fn cache_changed(cache: SerializedCache) -> DvcMessage {
        Box::new(DriveLetterPdu::SerializedCache(cache))
    }
}

impl_as_any!(DriveLetterServer);

impl DvcProcessor for DriveLetterServer {
    fn channel_name(&self) -> &str {
        DRIVE_LETTER_CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(vec![Box::new(DriveLetterPdu::Started)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode(payload) {
            Ok(DriveLetterPdu::SerializedCache(cache)) => {
                debug!(entries = cache.entries.len(), "Client restored drive letter mappings");
                self.handler.cache_restored(cache);
            }
            Ok(DriveLetterPdu::Started) => {
                warn!("Ignoring server-originated drive letter message received from client")
            }
            Err(error) => warn!(%error, "Ignoring malformed drive letter message"),
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for DriveLetterServer {}
