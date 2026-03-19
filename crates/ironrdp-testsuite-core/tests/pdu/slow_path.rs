use ironrdp_core::ReadCursor;
use ironrdp_pdu::slow_path::{self, GraphicsUpdateType};

// --- GraphicsUpdateType parsing ---

#[test]
fn read_graphics_update_type_orders() {
    let buf = 0x0000u16.to_le_bytes();
    let mut cursor = ReadCursor::new(&buf);
    assert_eq!(
        slow_path::read_graphics_update_type(&mut cursor).unwrap(),
        GraphicsUpdateType::Orders,
    );
}

#[test]
fn read_graphics_update_type_bitmap() {
    let buf = 0x0001u16.to_le_bytes();
    let mut cursor = ReadCursor::new(&buf);
    assert_eq!(
        slow_path::read_graphics_update_type(&mut cursor).unwrap(),
        GraphicsUpdateType::Bitmap,
    );
}

#[test]
fn read_graphics_update_type_palette() {
    let buf = 0x0002u16.to_le_bytes();
    let mut cursor = ReadCursor::new(&buf);
    assert_eq!(
        slow_path::read_graphics_update_type(&mut cursor).unwrap(),
        GraphicsUpdateType::Palette,
    );
}

#[test]
fn read_graphics_update_type_synchronize() {
    let buf = 0x0003u16.to_le_bytes();
    let mut cursor = ReadCursor::new(&buf);
    assert_eq!(
        slow_path::read_graphics_update_type(&mut cursor).unwrap(),
        GraphicsUpdateType::Synchronize,
    );
}

#[test]
fn read_graphics_update_type_unknown_value_errors() {
    let buf = 0x00FFu16.to_le_bytes();
    let mut cursor = ReadCursor::new(&buf);
    assert!(slow_path::read_graphics_update_type(&mut cursor).is_err());
}

#[test]
fn read_graphics_update_type_short_buffer_errors() {
    let buf = [0x01]; // only 1 byte, need 2
    let mut cursor = ReadCursor::new(&buf);
    assert!(slow_path::read_graphics_update_type(&mut cursor).is_err());
}

// --- Pointer messageType parsing ---

#[test]
fn decode_pointer_system_hidden() {
    // messageType(u16) + pad(u16) + systemPointerType(u32)
    let buf: [u8; 8] = [
        0x01, 0x00, // messageType = System (0x0001)
        0x00, 0x00, // pad
        0x00, 0x00, 0x00, 0x00, // SYSPTR_NULL
    ];
    let mut cursor = ReadCursor::new(&buf);
    let result = slow_path::decode_slow_path_pointer(&mut cursor).unwrap();
    assert!(matches!(result, ironrdp_pdu::pointer::PointerUpdateData::SetHidden));
}

#[test]
fn decode_pointer_system_default() {
    let buf: [u8; 8] = [
        0x01, 0x00, // messageType = System (0x0001)
        0x00, 0x00, // pad
        0x00, 0x7F, 0x00, 0x00, // SYSPTR_DEFAULT
    ];
    let mut cursor = ReadCursor::new(&buf);
    let result = slow_path::decode_slow_path_pointer(&mut cursor).unwrap();
    assert!(matches!(result, ironrdp_pdu::pointer::PointerUpdateData::SetDefault));
}

#[test]
fn decode_pointer_position() {
    let buf: [u8; 8] = [
        0x03, 0x00, // messageType = Position (0x0003)
        0x00, 0x00, // pad
        0x40, 0x00, // x = 64
        0x80, 0x00, // y = 128
    ];
    let mut cursor = ReadCursor::new(&buf);
    let result = slow_path::decode_slow_path_pointer(&mut cursor).unwrap();
    match result {
        ironrdp_pdu::pointer::PointerUpdateData::SetPosition(pos) => {
            assert_eq!(pos.x, 64);
            assert_eq!(pos.y, 128);
        }
        other => panic!("Expected SetPosition, got: {other:?}"),
    }
}

#[test]
fn decode_pointer_cached() {
    let buf: [u8; 6] = [
        0x07, 0x00, // messageType = Cached (0x0007)
        0x00, 0x00, // pad
        0x05, 0x00, // cacheIndex = 5
    ];
    let mut cursor = ReadCursor::new(&buf);
    let result = slow_path::decode_slow_path_pointer(&mut cursor).unwrap();
    assert!(matches!(result, ironrdp_pdu::pointer::PointerUpdateData::Cached(_)));
}

#[test]
fn decode_pointer_unknown_message_type_errors() {
    let buf: [u8; 4] = [
        0xFF, 0x00, // unknown messageType
        0x00, 0x00, // pad
    ];
    let mut cursor = ReadCursor::new(&buf);
    assert!(slow_path::decode_slow_path_pointer(&mut cursor).is_err());
}

#[test]
fn decode_pointer_short_buffer_errors() {
    let buf: [u8; 2] = [0x01, 0x00]; // only messageType, no pad
    let mut cursor = ReadCursor::new(&buf);
    assert!(slow_path::decode_slow_path_pointer(&mut cursor).is_err());
}

#[test]
fn decode_pointer_system_unknown_type_errors() {
    let buf: [u8; 8] = [
        0x01, 0x00, // messageType = System
        0x00, 0x00, // pad
        0xFF, 0xFF, 0x00, 0x00, // unknown systemPointerType
    ];
    let mut cursor = ReadCursor::new(&buf);
    assert!(slow_path::decode_slow_path_pointer(&mut cursor).is_err());
}
