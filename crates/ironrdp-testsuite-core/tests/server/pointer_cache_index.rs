use ironrdp_pdu::rdp::capability_sets::{CmdFlags, LargePointerSupportFlags};
use ironrdp_server::{
    ColorPointer, DesktopSize, DisplayUpdate, LargePointer, RGBAPointer, UpdateEncoder, UpdateEncoderCodecs,
};

// The Pointer Capability Set mstsc sends.
const POINTER_CACHE_SIZE: u16 = 21;
const COLOR_POINTER_CACHE_SIZE: u16 = 20;

/// Whether the encoder sends `update` to a client with mstsc's pointer caches.
async fn is_sent(update: DisplayUpdate) -> bool {
    let mut encoder = UpdateEncoder::new(
        DesktopSize { width: 64, height: 64 },
        CmdFlags::empty(),
        UpdateEncoderCodecs::new(),
        16 * 1024 * 1024,
        POINTER_CACHE_SIZE,
        COLOR_POINTER_CACHE_SIZE,
        LargePointerSupportFlags::all(),
    )
    .unwrap();
    let mut fragments = encoder.update(update);
    let first = fragments.next().await;
    first.map(Result::unwrap).is_some()
}

fn rgba_pointer(cache_index: u16) -> DisplayUpdate {
    DisplayUpdate::RGBAPointer(RGBAPointer {
        cache_index,
        width: 32,
        height: 32,
        hot_x: 0,
        hot_y: 0,
        data: vec![0xff; 32 * 32 * 4],
    })
}

fn large_pointer(cache_index: u16) -> DisplayUpdate {
    DisplayUpdate::LargePointer(LargePointer {
        cache_index,
        width: 128,
        height: 128,
        hot_x: 0,
        hot_y: 0,
        data: vec![0xff; 128 * 128 * 4],
    })
}

fn color_pointer(cache_index: u16) -> DisplayUpdate {
    DisplayUpdate::ColorPointer(ColorPointer {
        cache_index,
        width: 32,
        height: 32,
        hot_x: 0,
        hot_y: 0,
        and_mask: vec![0; 4 * 32],
        xor_mask: vec![0xff; 3 * 32 * 32],
    })
}

// oracle: spec MS-RDPBCGR §2.2.9.1.1.4.4 (cacheIndex), §2.2.7.1.5 (pointerCacheSize)
#[tokio::test]
async fn new_and_large_pointers_use_only_the_clients_pointer_cache() {
    assert!(is_sent(rgba_pointer(0)).await);
    assert!(is_sent(rgba_pointer(POINTER_CACHE_SIZE - 1)).await);
    assert!(!is_sent(rgba_pointer(POINTER_CACHE_SIZE)).await);

    assert!(is_sent(large_pointer(POINTER_CACHE_SIZE - 1)).await);
    assert!(!is_sent(large_pointer(POINTER_CACHE_SIZE)).await);
}

// oracle: spec MS-RDPBCGR §2.2.9.1.1.4.4 (cacheIndex), §2.2.7.1.5 (colorPointerCacheSize)
#[tokio::test]
async fn color_pointers_use_only_the_clients_color_pointer_cache() {
    assert!(is_sent(color_pointer(COLOR_POINTER_CACHE_SIZE - 1)).await);
    assert!(!is_sent(color_pointer(COLOR_POINTER_CACHE_SIZE)).await);
}

// oracle: spec MS-RDPBCGR §2.2.9.1.1.4.6 (a cached pointer was stored by a Color or a New Pointer Update)
#[tokio::test]
async fn cached_pointers_use_only_entries_the_client_has() {
    assert!(is_sent(DisplayUpdate::CachedPointer(POINTER_CACHE_SIZE - 1)).await);
    assert!(!is_sent(DisplayUpdate::CachedPointer(POINTER_CACHE_SIZE)).await);
}
