use std::sync::LazyLock;

use ironrdp_core::{decode, encode};

use super::*;

const FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER: [u8; 2] = [0x80, 0x08];
const FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER: [u8; 3] = [0x80, 0x81, 0xE7];
const FAST_PATH_UPDATE_PDU_BUFFER: [u8; 19] = [
    0x4, 0x10, 0x0, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x4, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0,
];
const FAST_PATH_UPDATE_PDU_WITH_LONG_LEN_BUFFER: [u8; 19] = [
    0x4, 0xff, 0x0, 0x4, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x4, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0,
];
const FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER: [u8; 3] = [0x80, 0x80, 0x08];

const FAST_PATH_HEADER_WITH_SHORT_LEN_PDU: FastPathHeader = FastPathHeader {
    flags: EncryptionFlags::ENCRYPTED,
    data_length: 6,
    forced_long_length: false,
};
const FAST_PATH_HEADER_WITH_LONG_LEN_PDU: FastPathHeader = FastPathHeader {
    flags: EncryptionFlags::ENCRYPTED,
    data_length: 484,
    forced_long_length: false,
};
const FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU: FastPathHeader = FastPathHeader {
    flags: EncryptionFlags::ENCRYPTED,
    data_length: 5,
    forced_long_length: true,
};

static FAST_PATH_UPDATE_PDU: LazyLock<FastPathUpdatePdu<'static>> = LazyLock::new(|| FastPathUpdatePdu {
    fragmentation: Fragmentation::Single,
    update_code: UpdateCode::SurfaceCommands,
    compression_flags: None,
    compression_type: None,
    data: &FAST_PATH_UPDATE_PDU_BUFFER[3..],
});

#[test]
fn from_buffer_correctly_parses_fast_path_header_with_short_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_SHORT_LEN_PDU,
        decode::<FastPathHeader>(FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_fast_path_header_with_short_length() {
    let expected = FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FAST_PATH_HEADER_WITH_SHORT_LEN_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_header_with_short_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_SHORT_LEN_BUFFER.len(),
        FAST_PATH_HEADER_WITH_SHORT_LEN_PDU.size()
    );
}

#[test]
fn from_buffer_correctly_parses_fast_path_header_with_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_LONG_LEN_PDU,
        decode::<FastPathHeader>(FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_fast_path_header_with_long_length() {
    let expected = FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FAST_PATH_HEADER_WITH_LONG_LEN_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_header_with_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_LONG_LEN_BUFFER.len(),
        FAST_PATH_HEADER_WITH_LONG_LEN_PDU.size()
    );
}

#[test]
fn from_buffer_correctly_parses_fast_path_header_with_forced_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU,
        decode::<FastPathHeader>(FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_fast_path_header_with_forced_long_length() {
    let expected = FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_header_with_forced_long_length() {
    assert_eq!(
        FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_BUFFER.len(),
        FAST_PATH_HEADER_WITH_FORCED_LONG_LEN_PDU.size()
    );
}

#[test]
fn from_buffer_correctly_parses_fast_path_update() {
    assert_eq!(
        *FAST_PATH_UPDATE_PDU,
        decode::<FastPathUpdatePdu<'_>>(FAST_PATH_UPDATE_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn from_buffer_returns_error_on_long_length_for_fast_path_update() {
    assert!(decode::<FastPathUpdatePdu<'_>>(FAST_PATH_UPDATE_PDU_WITH_LONG_LEN_BUFFER.as_ref()).is_err());
}

#[test]
fn to_buffer_correctly_serializes_fast_path_update() {
    let expected = FAST_PATH_UPDATE_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    encode(&*FAST_PATH_UPDATE_PDU, buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_fast_path_update() {
    assert_eq!(FAST_PATH_UPDATE_PDU_BUFFER.len(), FAST_PATH_UPDATE_PDU.size());
}

#[test]
fn buffer_size_boundary_fast_path_update() {
    let fph = FastPathHeader {
        flags: EncryptionFlags::ENCRYPTED,
        data_length: 125,
        forced_long_length: false,
    };
    assert_eq!(fph.size(), 2);
    let fph = FastPathHeader {
        flags: EncryptionFlags::ENCRYPTED,
        data_length: 126,
        forced_long_length: false,
    };
    assert_eq!(fph.size(), 3);
}
