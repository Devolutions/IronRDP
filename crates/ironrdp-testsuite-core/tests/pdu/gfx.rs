use ironrdp_core::ReadCursor;
use ironrdp_pdu::{decode, decode_cursor, encode_vec, PduEncode};
use ironrdp_testsuite_core::gfx::*;
use ironrdp_testsuite_core::graphics_messages::*;

#[test]
fn from_buffer_correctly_parses_server_pdu() {
    let buffer = HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.as_ref();

    assert_eq!(*HEADER_WITH_WIRE_TO_SURFACE_1, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_server_pdu() {
    let buffer = encode_vec(&*HEADER_WITH_WIRE_TO_SURFACE_1).unwrap();

    assert_eq!(buffer, HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_pdu() {
    assert_eq!(
        HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.len(),
        HEADER_WITH_WIRE_TO_SURFACE_1.size()
    );
}

#[test]
fn from_buffer_correctly_parses_client_pdu() {
    let buffer = HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.as_ref();

    assert_eq!(*HEADER_WITH_FRAME_ACKNOWLEDGE, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_pdu() {
    let buffer = encode_vec(&*HEADER_WITH_FRAME_ACKNOWLEDGE).unwrap();

    assert_eq!(buffer, HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_pdu() {
    assert_eq!(
        HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.len(),
        HEADER_WITH_FRAME_ACKNOWLEDGE.size()
    );
}

#[test]
fn from_buffer_correctly_parses_wire_to_surface_1_pdu() {
    let buffer = WIRE_TO_SURFACE_1_BUFFER.as_ref();

    assert_eq!(*WIRE_TO_SURFACE_1, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_wire_to_surface_1_pdu() {
    let buffer = encode_vec(&*WIRE_TO_SURFACE_1).unwrap();

    assert_eq!(buffer, WIRE_TO_SURFACE_1_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_wire_to_surface_1_pdu() {
    assert_eq!(WIRE_TO_SURFACE_1_BUFFER.len(), WIRE_TO_SURFACE_1.size());
}

#[test]
fn from_buffer_correctly_parses_wire_to_surface_2_pdu() {
    let buffer = WIRE_TO_SURFACE_2_BUFFER.as_ref();

    assert_eq!(*WIRE_TO_SURFACE_2, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_wire_to_surface_2_pdu() {
    let buffer = encode_vec(&*WIRE_TO_SURFACE_2).unwrap();

    assert_eq!(buffer, WIRE_TO_SURFACE_2_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_wire_to_surface_2_pdu() {
    assert_eq!(WIRE_TO_SURFACE_2_BUFFER.len(), WIRE_TO_SURFACE_2.size());
}

#[test]
fn from_buffer_correctly_parses_delete_encoding_context_pdu() {
    let buffer = DELETE_ENCODING_CONTEXT_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*DELETE_ENCODING_CONTEXT, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_delete_encoding_context_pdu() {
    let buffer = encode_vec(&*DELETE_ENCODING_CONTEXT).unwrap();

    assert_eq!(buffer, DELETE_ENCODING_CONTEXT_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_delete_encoding_context_pdu() {
    assert_eq!(DELETE_ENCODING_CONTEXT_BUFFER.len(), DELETE_ENCODING_CONTEXT.size());
}

#[test]
fn from_buffer_correctly_parses_solid_fill_pdu() {
    let buffer = SOLID_FILL_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*SOLID_FILL, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_solid_fill_pdu() {
    let buffer = encode_vec(&*SOLID_FILL).unwrap();
    assert_eq!(buffer, SOLID_FILL_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_solid_fill_pdu() {
    assert_eq!(SOLID_FILL_BUFFER.len(), SOLID_FILL.size());
}

#[test]
fn from_buffer_correctly_parses_surface_to_surface_pdu() {
    let buffer = SURFACE_TO_SURFACE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*SURFACE_TO_SURFACE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_surface_to_surface_pdu() {
    let buffer = encode_vec(&*SURFACE_TO_SURFACE).unwrap();

    assert_eq!(buffer, SURFACE_TO_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_surface_to_surface_pdu() {
    assert_eq!(SURFACE_TO_SURFACE_BUFFER.len(), SURFACE_TO_SURFACE.size());
}

#[test]
fn from_buffer_correctly_parses_surface_to_cache_pdu() {
    let buffer = SURFACE_TO_CACHE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*SURFACE_TO_CACHE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_surface_to_cache_pdu() {
    let buffer = encode_vec(&*SURFACE_TO_CACHE).unwrap();

    assert_eq!(buffer, SURFACE_TO_CACHE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_surface_to_cache_pdu() {
    assert_eq!(SURFACE_TO_CACHE_BUFFER.len(), SURFACE_TO_CACHE.size());
}

#[test]
fn from_buffer_correctly_parses_cache_to_surface_pdu() {
    let buffer = CACHE_TO_SURFACE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*CACHE_TO_SURFACE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_cache_to_surface_pdu() {
    let buffer = encode_vec(&*CACHE_TO_SURFACE).unwrap();

    assert_eq!(buffer, CACHE_TO_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_cache_to_surface_pdu() {
    assert_eq!(CACHE_TO_SURFACE_BUFFER.len(), CACHE_TO_SURFACE.size());
}

#[test]
fn from_buffer_correctly_parses_create_surface_pdu() {
    let buffer = CREATE_SURFACE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*CREATE_SURFACE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_create_surface_pdu() {
    let buffer = encode_vec(&*CREATE_SURFACE).unwrap();

    assert_eq!(buffer, CREATE_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_create_surface_pdu() {
    assert_eq!(CREATE_SURFACE_BUFFER.len(), CREATE_SURFACE.size());
}

#[test]
fn from_buffer_correctly_parses_delete_surface_pdu() {
    let buffer = DELETE_SURFACE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*DELETE_SURFACE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_delete_surface_pdu() {
    let buffer = encode_vec(&*DELETE_SURFACE).unwrap();

    assert_eq!(buffer, DELETE_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_delete_surface_pdu() {
    assert_eq!(DELETE_SURFACE_BUFFER.len(), DELETE_SURFACE.size());
}

#[test]
fn from_buffer_correctly_parses_reset_graphics() {
    let buffer = RESET_GRAPHICS_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*RESET_GRAPHICS, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_reset_graphics() {
    let buffer = encode_vec(&*RESET_GRAPHICS).unwrap();

    assert_eq!(buffer, RESET_GRAPHICS_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_reset_graphics() {
    assert_eq!(RESET_GRAPHICS_BUFFER.len(), RESET_GRAPHICS.size());
}

#[test]
fn from_buffer_correctly_parses_map_surface_to_output_pdu() {
    let buffer = MAP_SURFACE_TO_OUTPUT_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*MAP_SURFACE_TO_OUTPUT, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_map_surface_to_output_pdu() {
    let buffer = encode_vec(&*MAP_SURFACE_TO_OUTPUT).unwrap();

    assert_eq!(buffer, MAP_SURFACE_TO_OUTPUT_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_map_surface_to_output_pdu() {
    assert_eq!(MAP_SURFACE_TO_OUTPUT_BUFFER.len(), MAP_SURFACE_TO_OUTPUT.size());
}

#[test]
fn from_buffer_correctly_parses_evict_cache_entry_pdu() {
    let buffer = EVICT_CACHE_ENTRY_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*EVICT_CACHE_ENTRY, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_evict_cache_entry_pdu() {
    let buffer = encode_vec(&*EVICT_CACHE_ENTRY).unwrap();

    assert_eq!(buffer, EVICT_CACHE_ENTRY_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_evict_cache_entry_pdu() {
    assert_eq!(EVICT_CACHE_ENTRY_BUFFER.len(), EVICT_CACHE_ENTRY.size());
}

#[test]
fn from_buffer_correctly_parses_start_frame_pdu() {
    let buffer = START_FRAME_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*START_FRAME, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_start_frame_pdu() {
    let buffer = encode_vec(&*START_FRAME).unwrap();

    assert_eq!(buffer, START_FRAME_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_start_frame_pdu() {
    assert_eq!(START_FRAME_BUFFER.len(), START_FRAME.size());
}

#[test]
fn from_buffer_correctly_parses_end_frame_pdu() {
    let buffer = END_FRAME_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*END_FRAME, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_end_frame_pdu() {
    let buffer = encode_vec(&*END_FRAME).unwrap();

    assert_eq!(buffer, END_FRAME_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_end_frame_pdu() {
    assert_eq!(END_FRAME_BUFFER.len(), END_FRAME.size());
}

#[test]
fn from_buffer_correctly_parses_capabilities_confirm_pdu() {
    let buffer = CAPABILITIES_CONFIRM_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*CAPABILITIES_CONFIRM, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_capabilities_confirm_pdu() {
    let buffer = encode_vec(&*CAPABILITIES_CONFIRM).unwrap();

    assert_eq!(buffer, CAPABILITIES_CONFIRM_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_capabilities_confirm_pdu() {
    assert_eq!(CAPABILITIES_CONFIRM_BUFFER.len(), CAPABILITIES_CONFIRM.size());
}

#[test]
fn from_buffer_correctly_parses_capabilities_advertise_pdu() {
    let buffer = CAPABILITIES_ADVERTISE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*CAPABILITIES_ADVERTISE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_capabilities_advertise_pdu() {
    let buffer = encode_vec(&*CAPABILITIES_ADVERTISE).unwrap();

    assert_eq!(buffer, CAPABILITIES_ADVERTISE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_capabilities_advertise_pdu() {
    assert_eq!(CAPABILITIES_ADVERTISE_BUFFER.len(), CAPABILITIES_ADVERTISE.size());
}

#[test]
fn from_buffer_correctly_parses_frame_acknowledge_pdu() {
    let buffer = FRAME_ACKNOWLEDGE_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*FRAME_ACKNOWLEDGE, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_frame_acknowledge_pdu() {
    let buffer = encode_vec(&*FRAME_ACKNOWLEDGE).unwrap();

    assert_eq!(buffer, FRAME_ACKNOWLEDGE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_frame_acknowledge_pdu() {
    assert_eq!(FRAME_ACKNOWLEDGE_BUFFER.len(), FRAME_ACKNOWLEDGE.size());
}

#[test]
fn from_buffer_correctly_parses_cache_import_reply() {
    let buffer = CACHE_IMPORT_REPLY_BUFFER.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*CACHE_IMPORT_REPLY, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_cache_import_reply() {
    let buffer = encode_vec(&*CACHE_IMPORT_REPLY).unwrap();

    assert_eq!(buffer, CACHE_IMPORT_REPLY_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_cache_import_reply() {
    assert_eq!(CACHE_IMPORT_REPLY_BUFFER.len(), CACHE_IMPORT_REPLY.size());
}

#[test]
fn from_buffer_consume_correctly_parses_incorrect_len_avc_444_message() {
    let buffer = AVC_444_MESSAGE_INCORRECT_LEN.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*AVC_444_BITMAP, decode_cursor(&mut cursor).unwrap());
    assert!(!cursor.is_empty());
}

#[test]
fn from_buffer_consume_correctly_parses_avc_444_message() {
    let buffer = AVC_444_MESSAGE_CORRECT_LEN.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*AVC_444_BITMAP, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_consume_correctly_serializes_avc_444_message() {
    let buffer = encode_vec(&*AVC_444_BITMAP).unwrap();
    let expected = AVC_444_MESSAGE_CORRECT_LEN.as_ref();

    assert_eq!(expected, buffer.as_slice());
}
