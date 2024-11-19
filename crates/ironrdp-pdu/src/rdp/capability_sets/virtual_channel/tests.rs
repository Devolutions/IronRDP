use ironrdp_core::{decode, encode_vec};
use lazy_static::lazy_static;

use super::*;

const VIRTUAL_CHANNEL_INCOMPLETE_BUFFER: [u8; 4] = [
    0x01, 0x00, 0x00, 0x00, // flags
];

const VIRTUAL_CHANNEL_BUFFER: [u8; 8] = [
    0x00, 0x00, 0x00, 0x00, // flags
    0x40, 0x06, 0x00, 0x00, // chunk size
];

lazy_static! {
    pub static ref VIRTUAL_CHANNEL_INCOMPLETE: VirtualChannel = VirtualChannel {
        flags: VirtualChannelFlags::COMPRESSION_SERVER_TO_CLIENT,
        chunk_size: None,
    };
    pub static ref VIRTUAL_CHANNEL: VirtualChannel = VirtualChannel {
        flags: VirtualChannelFlags::NO_COMPRESSION,
        chunk_size: Some(1600),
    };
}

#[test]
fn from_buffer_correctly_parses_virtual_channel_incomplete_capset() {
    assert_eq!(
        *VIRTUAL_CHANNEL_INCOMPLETE,
        decode(VIRTUAL_CHANNEL_INCOMPLETE_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_virtual_channel_capset() {
    assert_eq!(*VIRTUAL_CHANNEL, decode(VIRTUAL_CHANNEL_BUFFER.as_ref()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_virtual_channel_incomplete_capset() {
    let c = VIRTUAL_CHANNEL_INCOMPLETE.clone();

    let buffer = encode_vec(&c).unwrap();

    assert_eq!(buffer, VIRTUAL_CHANNEL_INCOMPLETE_BUFFER.as_ref());
}

#[test]
fn to_buffer_correctly_serializes_virtual_channel_capset() {
    let c = VIRTUAL_CHANNEL.clone();

    let buffer = encode_vec(&c).unwrap();

    assert_eq!(buffer, VIRTUAL_CHANNEL_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_virtual_channel_incomplete_capset() {
    assert_eq!(
        VIRTUAL_CHANNEL_INCOMPLETE_BUFFER.len(),
        VIRTUAL_CHANNEL_INCOMPLETE.size()
    );
}

#[test]
fn buffer_length_is_correct_for_virtual_channel_capset() {
    assert_eq!(VIRTUAL_CHANNEL_BUFFER.len(), VIRTUAL_CHANNEL.size());
}
