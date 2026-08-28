use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_egfx::pdu::GfxPdu;

fn decode(bytes: &[u8]) -> GfxPdu {
    let mut cursor = ReadCursor::new(bytes);
    GfxPdu::decode(&mut cursor).expect("decode Haven fixture")
}

/// Real GDI+ ClearCodec tile from a live winserver2025 session. See
/// `test_data/egfx/haven/README.md` for provenance.
#[test]
fn wts1_64x64_clearcodec_tile() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_64x64_clearcodec_tile.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (128, 192, 192, 256));
    assert_eq!(rect.right - rect.left, 64, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 64, "exclusive height");
}

/// ClearCodec payload carrying an NSCodec-compressed sub-band. See
/// `test_data/egfx/haven/README.md` for why this is still `Codec1Type::ClearCodec`
/// at the `WireToSurface1` layer.
#[test]
fn wts1_576x128_nscodec_subregion() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_576x128_nscodec_subregion.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (64, 64, 640, 192));
    assert_eq!(rect.right - rect.left, 576, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 128, "exclusive height");
}

/// Taskbar-strip update: a wide, short region typical of window-chrome redraws.
#[test]
fn wts1_64x32_taskbar_strip() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_64x32_taskbar_strip.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (0, 768, 64, 800));
    assert_eq!(rect.right - rect.left, 64, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 32, "exclusive height");
}

/// Smallest real-world region observed: an 8x4 micro-tile update.
#[test]
fn wts1_8x4_micro_tile() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_8x4_micro_tile.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (374, 521, 382, 525));
    assert_eq!(rect.right - rect.left, 8, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 4, "exclusive height");
}

/// The `CreateSurface` PDU that establishes the 1280x800 context the four
/// `WireToSurface1` fixtures above were captured against.
#[test]
fn create_surface_1280x800_context() {
    let bytes = include_bytes!("../../test_data/egfx/haven/create_surface_1280x800_context.bin");
    let GfxPdu::CreateSurface(pdu) = decode(bytes) else {
        panic!("expected CreateSurface");
    };
    assert_eq!(pdu.surface_id, 0);
    assert_eq!(pdu.width, 1280);
    assert_eq!(pdu.height, 800);
}
