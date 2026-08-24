use std::sync::LazyLock;

use ironrdp_core::{decode, encode_vec};

use super::*;

const INPUT_BUFFER: [u8; 84] = [
    0x15, 0x00, // inputFlags
    0x00, 0x00, // pad2octetsA
    0x09, 0x04, 0x00, 0x00, // keyboardLayout
    0x04, 0x00, 0x00, 0x00, // keyboardType
    0x00, 0x00, 0x00, 0x00, // keyboardSubType
    0x0c, 0x00, 0x00, 0x00, // keyboardFunctionKey
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // imeFileName
];

static INPUT: LazyLock<Input> = LazyLock::new(|| Input {
    input_flags: InputFlags::SCANCODES | InputFlags::UNICODE | InputFlags::MOUSEX,
    keyboard_layout: 0x409,
    keyboard_type: Some(KeyboardType::IBM_ENHANCED),
    keyboard_subtype: 0,
    keyboard_function_key: 12,
    keyboard_ime_filename: String::new(),
});

#[test]
fn from_buffer_correctly_parses_input_capset() {
    assert_eq!(*INPUT, decode(INPUT_BUFFER.as_ref()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_input_capset() {
    let input = INPUT.clone();

    let buffer = encode_vec(&input).unwrap();

    assert_eq!(buffer, INPUT_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_input_capset() {
    assert_eq!(INPUT_BUFFER.len(), INPUT.size());
}

#[test]
fn keyboard_type_zero_decodes_to_none() {
    let mut buffer = INPUT_BUFFER;
    buffer[8..12].copy_from_slice(&0u32.to_le_bytes());

    let input: Input = decode(buffer.as_ref()).unwrap();
    assert_eq!(input.keyboard_type, None);

    assert_eq!(encode_vec(&input).unwrap(), buffer.as_ref());
}

#[test]
fn keyboard_type_unrecognized_value_round_trips() {
    let mut buffer = INPUT_BUFFER;
    buffer[8..12].copy_from_slice(&0x51u32.to_le_bytes());

    let input: Input = decode(buffer.as_ref()).unwrap();
    assert_eq!(input.keyboard_type, Some(KeyboardType(0x51)));

    assert_eq!(encode_vec(&input).unwrap(), buffer.as_ref());
}

/// The name occupies a fixed 64-byte slot, so an over-long one is truncated
/// rather than allowed to run past it.
///
/// Computing the padding as `IME_FILE_NAME_SIZE - written` underflowed at 32 or
/// more UTF-16 code units, and the resulting count reached `write_padding`.
#[test]
fn over_long_ime_file_name_is_truncated_to_the_field() {
    let mut input = INPUT.clone();
    input.keyboard_ime_filename = "A".repeat(64);

    let buffer = encode_vec(&input).unwrap();

    assert_eq!(buffer.len(), INPUT_BUFFER.len(), "the capability set is fixed width");

    let decoded: Input = decode(buffer.as_slice()).unwrap();
    assert_eq!(
        decoded.keyboard_ime_filename,
        "A".repeat(31),
        "31 code units fit alongside the terminator"
    );
}

/// 64 non-zero bytes decode to a 32-code-unit name, which is one more than the
/// slot can re-encode. Such a name arrives from a peer, so the round trip has to
/// terminate rather than panic.
#[test]
fn ime_file_name_filling_the_field_survives_a_round_trip() {
    let mut buffer = INPUT_BUFFER;
    for pair in buffer[20..84].chunks_exact_mut(2) {
        pair[0] = b'A';
        pair[1] = 0;
    }

    let decoded: Input = decode(buffer.as_ref()).unwrap();
    assert_eq!(decoded.keyboard_ime_filename.chars().count(), 32);

    let re_encoded = encode_vec(&decoded).unwrap();
    assert_eq!(re_encoded.len(), INPUT_BUFFER.len());
}
