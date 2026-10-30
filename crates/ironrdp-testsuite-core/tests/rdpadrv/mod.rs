mod audio_level;
mod drive_letter;

use ironrdp_core::encode_vec;

const CHANNEL_ID: u32 = 5;

/// Encodes the messages a dynamic channel processor returned.
fn encoded(messages: &[ironrdp_dvc::DvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect()
}
