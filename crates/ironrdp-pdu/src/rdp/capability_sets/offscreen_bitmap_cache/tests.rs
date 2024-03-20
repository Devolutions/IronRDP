use lazy_static::lazy_static;

use super::*;
use crate::{decode, encode_vec};

const OFFSCREEN_BITMAP_CACHE_BUFFER: [u8; 8] = [
    0x01, 0x00, 0x00, 0x00, // offscreenSupportLevel
    0x00, 0x1e, // offscreenCacheSize
    0x64, 0x00, // offscreenCacheEntries
];

lazy_static! {
    pub static ref OFFSCREEN_BITMAP_CACHE: OffscreenBitmapCache = OffscreenBitmapCache {
        is_supported: true,
        cache_size: 7680,
        cache_entries: 100,
    };
}

#[test]
fn from_buffer_correctly_parses_offscreen_bitmap_cache_capset() {
    assert_eq!(
        *OFFSCREEN_BITMAP_CACHE,
        decode(OFFSCREEN_BITMAP_CACHE_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_offscreen_bitmap_cache_capset() {
    let off = OFFSCREEN_BITMAP_CACHE.clone();

    let buffer = encode_vec(&off).unwrap();

    assert_eq!(buffer, OFFSCREEN_BITMAP_CACHE_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_offscreen_bitmap_cache_capset() {
    assert_eq!(OFFSCREEN_BITMAP_CACHE_BUFFER.len(), OFFSCREEN_BITMAP_CACHE.size());
}
