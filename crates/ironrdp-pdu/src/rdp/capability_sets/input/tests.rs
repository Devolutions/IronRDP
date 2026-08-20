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
