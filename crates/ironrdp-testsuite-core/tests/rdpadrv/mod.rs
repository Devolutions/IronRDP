mod audio_level;
mod drive_letter;

use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor;

const CHANNEL_ID: u32 = 5;

/// Encodes the messages a dynamic channel processor returned.
fn encoded(messages: &[ironrdp_dvc::DvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect()
}

/// Opens the channel on `processor`, which a client answers with nothing.
fn started<P: DvcProcessor>(mut processor: P) -> P {
    assert!(processor.start(CHANNEL_ID).expect("start channel").is_empty());
    processor
}

/// Handler that records every value it receives, shared between clones.
#[derive(Clone)]
struct Recorder<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Recorder<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<T> Recorder<T> {
    fn push(&self, value: T) {
        self.0.lock().unwrap().push(value);
    }

    fn take(&self) -> Vec<T> {
        core::mem::take(&mut *self.0.lock().unwrap())
    }
}
