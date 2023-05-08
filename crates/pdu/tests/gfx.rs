use ironrdp_pdu::rdp::vc::dvc::gfx::*;
use ironrdp_pdu::{PduBufferParsing, PduParsing};
use ironrdp_pdu_samples::gfx::*;
use ironrdp_pdu_samples::graphics_messages::*;

#[test]
fn from_buffer_correctly_parses_server_pdu() {
    let mut buffer = HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.as_slice();

    assert_eq!(
        *HEADER_WITH_WIRE_TO_SURFACE_1,
        ServerPdu::from_buffer(&mut buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_pdu() {
    let mut buffer = Vec::with_capacity(HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.len());
    HEADER_WITH_WIRE_TO_SURFACE_1.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_pdu() {
    assert_eq!(
        HEADER_WITH_WIRE_TO_SURFACE_1_BUFFER.len(),
        HEADER_WITH_WIRE_TO_SURFACE_1.buffer_length()
    );
}

#[test]
fn from_buffer_correctly_parses_client_pdu() {
    let buffer = HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.as_slice();

    assert_eq!(*HEADER_WITH_FRAME_ACKNOWLEDGE, ClientPdu::from_buffer(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    HEADER_WITH_FRAME_ACKNOWLEDGE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_pdu() {
    assert_eq!(
        HEADER_WITH_FRAME_ACKNOWLEDGE_BUFFER.len(),
        HEADER_WITH_FRAME_ACKNOWLEDGE.buffer_length()
    );
}

#[test]
fn from_buffer_correctly_parses_wire_to_surface_1_pdu() {
    let mut buffer = WIRE_TO_SURFACE_1_BUFFER.as_ref();

    assert_eq!(*WIRE_TO_SURFACE_1, WireToSurface1Pdu::from_buffer(&mut buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_wire_to_surface_1_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    WIRE_TO_SURFACE_1.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, WIRE_TO_SURFACE_1_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_wire_to_surface_1_pdu() {
    assert_eq!(WIRE_TO_SURFACE_1_BUFFER.len(), WIRE_TO_SURFACE_1.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_wire_to_surface_2_pdu() {
    let mut buffer = WIRE_TO_SURFACE_2_BUFFER.as_ref();

    assert_eq!(*WIRE_TO_SURFACE_2, WireToSurface2Pdu::from_buffer(&mut buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_wire_to_surface_2_pdu() {
    let mut buffer = Vec::with_capacity(WIRE_TO_SURFACE_2_BUFFER.len());
    WIRE_TO_SURFACE_2.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, WIRE_TO_SURFACE_2_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_wire_to_surface_2_pdu() {
    assert_eq!(WIRE_TO_SURFACE_2_BUFFER.len(), WIRE_TO_SURFACE_2.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_delete_encoding_context_pdu() {
    let mut buffer = DELETE_ENCODING_CONTEXT_BUFFER.as_ref();

    assert_eq!(
        *DELETE_ENCODING_CONTEXT,
        DeleteEncodingContextPdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_delete_encoding_context_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    DELETE_ENCODING_CONTEXT.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, DELETE_ENCODING_CONTEXT_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_delete_encoding_context_pdu() {
    assert_eq!(
        DELETE_ENCODING_CONTEXT_BUFFER.len(),
        DELETE_ENCODING_CONTEXT.buffer_length()
    );
}

#[test]
fn from_buffer_correctly_parses_solid_fill_pdu() {
    let mut buffer = SOLID_FILL_BUFFER.as_ref();

    assert_eq!(*SOLID_FILL, SolidFillPdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_solid_fill_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    SOLID_FILL.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, SOLID_FILL_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_solid_fill_pdu() {
    assert_eq!(SOLID_FILL_BUFFER.len(), SOLID_FILL.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_surface_to_surface_pdu() {
    let mut buffer = SURFACE_TO_SURFACE_BUFFER.as_ref();

    assert_eq!(
        *SURFACE_TO_SURFACE,
        SurfaceToSurfacePdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_surface_to_surface_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    SURFACE_TO_SURFACE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, SURFACE_TO_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_surface_to_surface_pdu() {
    assert_eq!(SURFACE_TO_SURFACE_BUFFER.len(), SURFACE_TO_SURFACE.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_surface_to_cache_pdu() {
    let mut buffer = SURFACE_TO_CACHE_BUFFER.as_ref();

    assert_eq!(*SURFACE_TO_CACHE, SurfaceToCachePdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_surface_to_cache_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    SURFACE_TO_CACHE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, SURFACE_TO_CACHE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_surface_to_cache_pdu() {
    assert_eq!(SURFACE_TO_CACHE_BUFFER.len(), SURFACE_TO_CACHE.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_cache_to_surface_pdu() {
    let mut buffer = CACHE_TO_SURFACE_BUFFER.as_ref();

    assert_eq!(*CACHE_TO_SURFACE, CacheToSurfacePdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_cache_to_surface_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    CACHE_TO_SURFACE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, CACHE_TO_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_cache_to_surface_pdu() {
    assert_eq!(CACHE_TO_SURFACE_BUFFER.len(), CACHE_TO_SURFACE.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_create_surface_pdu() {
    let mut buffer = CREATE_SURFACE_BUFFER.as_ref();

    assert_eq!(*CREATE_SURFACE, CreateSurfacePdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_create_surface_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    CREATE_SURFACE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, CREATE_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_create_surface_pdu() {
    assert_eq!(CREATE_SURFACE_BUFFER.len(), CREATE_SURFACE.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_delete_surface_pdu() {
    let mut buffer = DELETE_SURFACE_BUFFER.as_ref();

    assert_eq!(*DELETE_SURFACE, DeleteSurfacePdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_delete_surface_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    DELETE_SURFACE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, DELETE_SURFACE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_delete_surface_pdu() {
    assert_eq!(DELETE_SURFACE_BUFFER.len(), DELETE_SURFACE.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_reset_graphics() {
    let mut buffer = RESET_GRAPHICS_BUFFER.as_ref();

    assert_eq!(*RESET_GRAPHICS, ResetGraphicsPdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_reset_graphics() {
    let mut buffer = Vec::with_capacity(1024);
    RESET_GRAPHICS.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, RESET_GRAPHICS_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_reset_graphics() {
    assert_eq!(RESET_GRAPHICS_BUFFER.len(), RESET_GRAPHICS.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_map_surface_to_output_pdu() {
    let mut buffer = MAP_SURFACE_TO_OUTPUT_BUFFER.as_ref();

    assert_eq!(
        *MAP_SURFACE_TO_OUTPUT,
        MapSurfaceToOutputPdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_map_surface_to_output_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    MAP_SURFACE_TO_OUTPUT.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, MAP_SURFACE_TO_OUTPUT_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_map_surface_to_output_pdu() {
    assert_eq!(
        MAP_SURFACE_TO_OUTPUT_BUFFER.len(),
        MAP_SURFACE_TO_OUTPUT.buffer_length()
    );
}

#[test]
fn from_buffer_correctly_parses_evict_cache_entry_pdu() {
    let mut buffer = EVICT_CACHE_ENTRY_BUFFER.as_ref();

    assert_eq!(
        *EVICT_CACHE_ENTRY,
        EvictCacheEntryPdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_evict_cache_entry_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    EVICT_CACHE_ENTRY.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, EVICT_CACHE_ENTRY_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_evict_cache_entry_pdu() {
    assert_eq!(EVICT_CACHE_ENTRY_BUFFER.len(), EVICT_CACHE_ENTRY.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_start_frame_pdu() {
    let mut buffer = START_FRAME_BUFFER.as_ref();

    assert_eq!(*START_FRAME, StartFramePdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_start_frame_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    START_FRAME.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, START_FRAME_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_start_frame_pdu() {
    assert_eq!(START_FRAME_BUFFER.len(), START_FRAME.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_end_frame_pdu() {
    let mut buffer = END_FRAME_BUFFER.as_ref();

    assert_eq!(*END_FRAME, EndFramePdu::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_end_frame_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    END_FRAME.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, END_FRAME_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_end_frame_pdu() {
    assert_eq!(END_FRAME_BUFFER.len(), END_FRAME.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_capabilities_confirm_pdu() {
    let mut buffer = CAPABILITIES_CONFIRM_BUFFER.as_ref();

    assert_eq!(
        *CAPABILITIES_CONFIRM,
        CapabilitiesConfirmPdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_capabilities_confirm_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    CAPABILITIES_CONFIRM.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, CAPABILITIES_CONFIRM_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_capabilities_confirm_pdu() {
    assert_eq!(CAPABILITIES_CONFIRM_BUFFER.len(), CAPABILITIES_CONFIRM.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_capabilities_advertise_pdu() {
    let mut buffer = CAPABILITIES_ADVERTISE_BUFFER.as_ref();

    assert_eq!(
        *CAPABILITIES_ADVERTISE,
        CapabilitiesAdvertisePdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_capabilities_advertise_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    CAPABILITIES_ADVERTISE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, CAPABILITIES_ADVERTISE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_capabilities_advertise_pdu() {
    assert_eq!(
        CAPABILITIES_ADVERTISE_BUFFER.len(),
        CAPABILITIES_ADVERTISE.buffer_length()
    );
}

#[test]
fn from_buffer_correctly_parses_frame_acknowledge_pdu() {
    let mut buffer = FRAME_ACKNOWLEDGE_BUFFER.as_ref();

    assert_eq!(
        *FRAME_ACKNOWLEDGE,
        FrameAcknowledgePdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_frame_acknowledge_pdu() {
    let mut buffer = Vec::with_capacity(1024);
    FRAME_ACKNOWLEDGE.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, FRAME_ACKNOWLEDGE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_frame_acknowledge_pdu() {
    assert_eq!(FRAME_ACKNOWLEDGE_BUFFER.len(), FRAME_ACKNOWLEDGE.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_cache_import_reply() {
    let mut buffer = CACHE_IMPORT_REPLY_BUFFER.as_ref();

    assert_eq!(
        *CACHE_IMPORT_REPLY,
        CacheImportReplyPdu::from_buffer(&mut buffer).unwrap()
    );
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_cache_import_reply() {
    let mut buffer = Vec::with_capacity(1024);
    CACHE_IMPORT_REPLY.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, CACHE_IMPORT_REPLY_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_cache_import_reply() {
    assert_eq!(CACHE_IMPORT_REPLY_BUFFER.len(), CACHE_IMPORT_REPLY.buffer_length());
}

#[test]
fn from_buffer_consume_correctly_parses_incorrect_len_avc_444_message() {
    let mut buffer = AVC_444_MESSAGE_INCORRECT_LEN.as_ref();
    assert_eq!(
        *AVC_444_BITMAP,
        Avc444BitmapStream::from_buffer_consume(&mut buffer).unwrap()
    );
}
#[test]
fn from_buffer_consume_correctly_parses_avc_444_message() {
    let mut buffer = AVC_444_MESSAGE_CORRECT_LEN.as_ref();
    assert_eq!(
        *AVC_444_BITMAP,
        Avc444BitmapStream::from_buffer_consume(&mut buffer).unwrap()
    );
}

#[test]
fn to_buffer_consume_correctly_serializes_avc_444_message() {
    let expected = AVC_444_MESSAGE_CORRECT_LEN.as_ref();
    let mut buffer = vec![0; expected.len()];

    AVC_444_BITMAP.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}
