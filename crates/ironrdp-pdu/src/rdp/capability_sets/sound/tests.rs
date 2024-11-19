use ironrdp_core::{decode, encode_vec};
use lazy_static::lazy_static;

use super::*;

const SOUND_BUFFER: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

lazy_static! {
    pub static ref SOUND: Sound = Sound {
        flags: SoundFlags::BEEPS,
    };
}

#[test]
fn from_buffer_correctly_parses_sound_capset() {
    assert_eq!(*SOUND, decode(SOUND_BUFFER.as_ref()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_sound_capset() {
    let sound = SOUND.clone();

    let buffer = encode_vec(&sound).unwrap();

    assert_eq!(buffer, SOUND_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_sound_capset() {
    assert_eq!(SOUND.size(), SOUND_BUFFER.len());
}
