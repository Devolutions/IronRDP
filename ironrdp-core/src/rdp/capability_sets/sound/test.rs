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
    assert_eq!(*SOUND, Sound::from_buffer(SOUND_BUFFER.as_ref()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_sound_capset() {
    let mut buffer = Vec::new();

    SOUND.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, SOUND_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_sound_capset() {
    assert_eq!(SOUND.buffer_length(), SOUND_BUFFER.len());
}
