//! Audio level and drive letter persistence for Windows MultiPoint sessions ([MS-RDPADRV]).
//!
//! The session host reports its master volume levels and the drive letters it assigned to redirected USB storage
//! devices, and asks for them back when the client connects again.
//! [`MultipointSettings`] holds those values for the client: seed it from persistent storage before connecting, and
//! save it again whenever [`RdpOutputEvent::MultipointSettingsChanged`] arrives.
//! Reconnects within one client reuse the same settings, so they restore the latest values without the embedder.
//!
//! [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/

use std::sync::{Arc, Mutex, MutexGuard};

use ironrdp_rdpadrv::client::{AudioLevelClient, AudioLevelHandler, DriveLetterClient, DriveLetterHandler};
pub use ironrdp_rdpadrv::pdu::{CacheEntry, DataFlow, SerializedCache, VolumeChange};

use crate::output_channel::OutputEventSender;
use crate::rdp::RdpOutputEvent;

#[derive(Debug, Default)]
struct Settings {
    render: Option<VolumeChange>,
    capture: Option<VolumeChange>,
    drive_letters: Option<SerializedCache>,
}

/// Values a MultiPoint session asked the client to keep across sessions, shared between clones.
#[derive(Debug, Clone, Default)]
pub struct MultipointSettings(Arc<Mutex<Settings>>);

impl MultipointSettings {
    /// Creates settings holding no values, so the client restores nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the stored volume level of `data_flow`.
    pub fn volume(&self, data_flow: DataFlow) -> Option<VolumeChange> {
        let settings = self.lock();
        match data_flow {
            DataFlow::Render => settings.render,
            DataFlow::Capture => settings.capture,
        }
    }

    /// Stores the volume level of `change.data_flow`.
    pub fn set_volume(&self, change: VolumeChange) {
        let mut settings = self.lock();
        match change.data_flow {
            DataFlow::Render => settings.render = Some(change),
            DataFlow::Capture => settings.capture = Some(change),
        }
    }

    /// Returns the stored drive letter mappings.
    ///
    /// Encode them with [`ironrdp_core::encode_vec`] to store them as bytes.
    pub fn drive_letters(&self) -> Option<SerializedCache> {
        self.lock().drive_letters.clone()
    }

    /// Stores drive letter mappings, replacing any stored before.
    pub fn set_drive_letters(&self, cache: SerializedCache) {
        self.lock().drive_letters = Some(cache);
    }

    /// Builds both channel processors, seeded with the stored values, that report changes back to these settings.
    pub(crate) fn processors(&self, output_event_sender: &OutputEventSender) -> (AudioLevelClient, DriveLetterClient) {
        let recorder = || {
            Box::new(Recorder {
                settings: self.clone(),
                output_event_sender: output_event_sender.clone(),
            })
        };

        let audio_level = DataFlow::ALL
            .into_iter()
            .filter_map(|data_flow| self.volume(data_flow))
            .fold(AudioLevelClient::new(recorder()), AudioLevelClient::with_volume);
        let drive_letter = match self.drive_letters() {
            Some(cache) => DriveLetterClient::new(recorder()).with_cache(cache),
            None => DriveLetterClient::new(recorder()),
        };
        (audio_level, drive_letter)
    }

    fn lock(&self) -> MutexGuard<'_, Settings> {
        self.0.lock().expect("not poisoned")
    }
}

/// Stores each value the session reports and tells the embedder to save the settings.
///
/// The handlers run synchronously inside the session loop, so the event is deferred and delivered with backpressure:
/// a dropped event would leave the embedder's saved copy stale.
struct Recorder {
    settings: MultipointSettings,
    output_event_sender: OutputEventSender,
}

impl AudioLevelHandler for Recorder {
    fn volume_changed(&mut self, change: VolumeChange) {
        self.settings.set_volume(change);
        self.output_event_sender
            .defer(RdpOutputEvent::MultipointSettingsChanged);
    }
}

impl DriveLetterHandler for Recorder {
    fn cache_changed(&mut self, cache: &SerializedCache) {
        self.settings.set_drive_letters(cache.clone());
        self.output_event_sender
            .defer(RdpOutputEvent::MultipointSettingsChanged);
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_rdpadrv::pdu::{AudioLevelPdu, DriveLetterPdu};
    use tokio::sync::watch;

    use super::*;

    const CHANNEL_ID: u32 = 1;

    fn encoded(messages: &[ironrdp_dvc::DvcMessage]) -> Vec<Vec<u8>> {
        messages
            .iter()
            .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
            .collect()
    }

    async fn next_deferred_event(
        sender: &OutputEventSender,
        receiver: &mut crate::output_channel::OutputEventReceiver,
    ) -> Option<RdpOutputEvent> {
        let (_close_sender, mut close_receiver) = watch::channel(false);
        assert!(
            sender
                .send_deferred(&mut close_receiver)
                .await
                .expect("output channel is open")
        );
        receiver.recv().await
    }

    // oracle: spec MS-RDPADRV §3.1.3.1, §3.1.5, with the processors rebuilt as a reconnect rebuilds them
    #[tokio::test]
    async fn reported_volume_is_saved_and_restored_by_the_next_connection() {
        let (output_sender, mut output_receiver) = crate::output_channel::output_channel(4);
        let settings = MultipointSettings::new();
        let reported = AudioLevelPdu::VolumeChange(VolumeChange {
            data_flow: DataFlow::Capture,
            volume: 0.25,
            muted: true,
        });

        let (mut audio_level, _) = settings.processors(&output_sender);
        audio_level
            .process(CHANNEL_ID, &encode_vec(&reported).expect("encode"))
            .expect("process");

        assert!(matches!(
            next_deferred_event(&output_sender, &mut output_receiver).await,
            Some(RdpOutputEvent::MultipointSettingsChanged)
        ));
        let (mut audio_level, _) = settings.processors(&output_sender);
        let replies = audio_level
            .process(CHANNEL_ID, &encode_vec(&AudioLevelPdu::Started).expect("encode"))
            .expect("process");
        assert_eq!(encoded(&replies), [encode_vec(&reported).expect("encode")]);
    }

    // oracle: spec MS-RDPADRV §3.1.3.2, §3.1.5, with the processors rebuilt as a reconnect rebuilds them
    #[tokio::test]
    async fn reported_drive_letters_are_saved_and_restored_by_the_next_connection() {
        let (output_sender, mut output_receiver) = crate::output_channel::output_channel(4);
        let settings = MultipointSettings::new();
        let reported = DriveLetterPdu::SerializedCache(SerializedCache {
            entries: vec![CacheEntry::dword("AB".to_owned(), 13)],
        });

        let (_, mut drive_letter) = settings.processors(&output_sender);
        drive_letter
            .process(CHANNEL_ID, &encode_vec(&reported).expect("encode"))
            .expect("process");

        assert!(matches!(
            next_deferred_event(&output_sender, &mut output_receiver).await,
            Some(RdpOutputEvent::MultipointSettingsChanged)
        ));
        let (_, mut drive_letter) = settings.processors(&output_sender);
        let replies = drive_letter
            .process(CHANNEL_ID, &encode_vec(&DriveLetterPdu::Started).expect("encode"))
            .expect("process");
        assert_eq!(encoded(&replies), [encode_vec(&reported).expect("encode")]);
    }
}
