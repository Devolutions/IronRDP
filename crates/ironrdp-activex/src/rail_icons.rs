//! RemoteApp window and notification icons ([MS-RDPERP] 2.2.1.2.3 Icon Info (TS_ICON_INFO)
//! and 2.2.1.2.4 Cached Icon Info (TS_CACHED_ICON_INFO)).
//!
//! The server sends icons as device-independent bitmaps: a bottom-up color bitmap
//! (palettized for 1, 4 and 8 bpp, RGB555 for 16 bpp, BGR for 24 bpp and BGRA for
//! 32 bpp) plus a bottom-up monochrome AND mask. They are converted to top-down
//! BGRA here and turned into `HICON`s, which are cached by `(CacheId, CacheEntry)`
//! so later orders can reference them.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp

use std::collections::HashMap;
use std::rc::Rc;

use ironrdp_pdu::window::{CachedIconInfo, IconInfo, IconSource};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
    HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

/// Largest icon edge accepted from the server; Windows icons are at most 256 pixels.
const MAX_ICON_DIMENSION: u16 = 512;

/// Upper bound on cached icons, so a server cannot grow the cache without limit.
const MAX_CACHED_ICONS: usize = 4096;

/// Converts a TS_ICON_INFO bitmap into top-down BGRA pixels with straight alpha.
///
/// Rows of both bitmaps are read with the stride their byte length implies, so
/// 2- and 4-byte row padding are both accepted. A 32 bpp icon keeps its own
/// alpha when any pixel has one; otherwise the AND mask decides transparency, and an
/// icon without a mask is opaque. Color bits or a mask too short for the icon's size
/// reject the icon.
pub(crate) fn icon_to_bgra(icon: &IconInfo) -> Option<Vec<u8>> {
    let width = usize::from(icon.width);
    let height = usize::from(icon.height);
    if !(0 < width && 0 < height && icon.width <= MAX_ICON_DIMENSION && icon.height <= MAX_ICON_DIMENSION) {
        return None;
    }
    let bits_per_pixel = usize::from(icon.bpp);
    let color_stride = row_stride(icon.bits_color.len(), height, (width * bits_per_pixel).div_ceil(8))?;

    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        let source_row = &icon.bits_color[(height - 1 - y) * color_stride..][..color_stride];
        for x in 0..width {
            let [b, g, r, a] = read_pixel(icon, source_row, x)?;
            pixels[(y * width + x) * 4..][..4].copy_from_slice(&[b, g, r, a]);
        }
    }

    let has_alpha = icon.bpp == 32 && pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
    if !has_alpha {
        let mask_stride = if icon.bits_mask.is_empty() {
            None
        } else {
            Some(row_stride(icon.bits_mask.len(), height, width.div_ceil(8))?)
        };
        for y in 0..height {
            let mask_row = mask_stride.map(|stride| &icon.bits_mask[(height - 1 - y) * stride..][..stride]);
            for x in 0..width {
                let transparent = mask_row.is_some_and(|row| row[x / 8] & (0x80 >> (x % 8)) != 0);
                pixels[(y * width + x) * 4 + 3] = if transparent { 0 } else { 0xff };
            }
        }
    }
    Some(pixels)
}

/// Returns the row stride a bottom-up bitmap of `len` bytes and `height` rows uses,
/// provided each row holds at least `minimum` bytes.
fn row_stride(len: usize, height: usize, minimum: usize) -> Option<usize> {
    let stride = len / height;
    (0 < minimum && minimum <= stride).then_some(stride)
}

fn read_pixel(icon: &IconInfo, row: &[u8], x: usize) -> Option<[u8; 4]> {
    let palette = |index: usize| -> [u8; 4] {
        icon.color_table
            .get(index * 4..index * 4 + 3)
            .map_or([0, 0, 0, 0], |entry| [entry[0], entry[1], entry[2], 0])
    };
    Some(match icon.bpp {
        1 => palette(usize::from((row[x / 8] >> (7 - x % 8)) & 0x01)),
        4 => palette(usize::from(
            (row[x / 2] >> if x.is_multiple_of(2) { 4 } else { 0 }) & 0x0f,
        )),
        8 => palette(usize::from(row[x])),
        16 => {
            let value = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]);
            let expand = |component: u16| -> u8 {
                // Masked to five bits, so the value fits in a byte.
                let component = (component & 0x1f) as u8;
                (component << 3) | (component >> 2)
            };
            [expand(value), expand(value >> 5), expand(value >> 10), 0]
        }
        24 => [row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 0],
        32 => [row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]],
        _ => return None,
    })
}

/// Builds a top-down monochrome AND mask with 2-byte row alignment from BGRA alpha.
pub(crate) fn mask_from_alpha(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let stride = width.div_ceil(16) * 2;
    let mut mask = vec![0u8; stride * height];
    for y in 0..height {
        for x in 0..width {
            if pixels[(y * width + x) * 4 + 3] == 0 {
                mask[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    mask
}

/// An icon handle destroyed when the last reference goes away.
#[derive(Debug)]
pub(crate) struct OwnedIcon(HICON);

impl OwnedIcon {
    pub(crate) fn handle(&self) -> HICON {
        self.0
    }

    /// Creates an `HICON` from a TS_ICON_INFO.
    pub(crate) fn from_icon_info(icon: &IconInfo) -> Option<Rc<Self>> {
        let pixels = icon_to_bgra(icon)?;
        let width = i32::from(icon.width);
        let height = i32::from(icon.height);
        let mask = mask_from_alpha(usize::from(icon.width), usize::from(icon.height), &pixels);

        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).ok()?,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = core::ptr::null_mut();
        // SAFETY: `bitmap_info` describes a 32 bpp top-down DIB and outlives the call.
        let color = unsafe { CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
        if bits.is_null() {
            delete_bitmap(color);
            return None;
        }
        // SAFETY: the DIB section holds exactly `width * height * 4` bytes, the length of `pixels`.
        unsafe { core::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len()) };
        // SAFETY: `mask` holds `height` rows of 2-byte-aligned monochrome bits.
        let mask_bitmap = unsafe { CreateBitmap(width, height, 1, 1, Some(mask.as_ptr().cast())) };
        if mask_bitmap.is_invalid() {
            delete_bitmap(color);
            return None;
        }

        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bitmap,
            hbmColor: color,
        };
        // SAFETY: both bitmaps are valid; CreateIconIndirect copies them.
        let icon = unsafe { CreateIconIndirect(&info) };
        delete_bitmap(color);
        delete_bitmap(mask_bitmap);
        match icon {
            Ok(icon) => Some(Rc::new(Self(icon))),
            Err(error) => {
                tracing::debug!(?error, "Unable to create a RAIL icon");
                None
            }
        }
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateIconIndirect and is destroyed once.
        if let Err(error) = unsafe { DestroyIcon(self.0) } {
            tracing::debug!(?error, "Unable to destroy a RAIL icon");
        }
    }
}

fn delete_bitmap(bitmap: HBITMAP) {
    // SAFETY: the bitmap is owned by the caller and no longer selected into a DC.
    let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
}

/// The client's RAIL icon cache ([MS-RDPERP] 3.2.1.2 Window Icon Cache).
///
/// [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp
#[derive(Debug, Default)]
pub(crate) struct RailIconCache {
    icons: HashMap<(u8, u16), Rc<OwnedIcon>>,
}

impl RailIconCache {
    /// Resolves an icon order to an icon handle, storing inline icons the server asks to cache.
    pub(crate) fn resolve(&mut self, source: &IconSource) -> Option<Rc<OwnedIcon>> {
        match source {
            IconSource::Icon(icon) => {
                let handle = OwnedIcon::from_icon_info(icon)?;
                if icon.cache_entry != IconInfo::NO_CACHE_ENTRY && icon.cache_id != IconInfo::NO_CACHE_ID {
                    let key = (icon.cache_id, icon.cache_entry);
                    if self.icons.len() < MAX_CACHED_ICONS || self.icons.contains_key(&key) {
                        self.icons.insert(key, Rc::clone(&handle));
                    } else {
                        tracing::debug!(?key, "RAIL icon cache is full");
                    }
                }
                Some(handle)
            }
            IconSource::Cached(CachedIconInfo { cache_entry, cache_id }) => {
                let icon = self.icons.get(&(*cache_id, *cache_entry)).cloned();
                if icon.is_none() {
                    tracing::debug!(cache_id, cache_entry, "RAIL order references an uncached icon");
                }
                icon
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.icons.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(
        bpp: u8,
        width: u16,
        height: u16,
        bits_color: Vec<u8>,
        bits_mask: Vec<u8>,
        color_table: Vec<u8>,
    ) -> IconInfo {
        IconInfo {
            cache_entry: IconInfo::NO_CACHE_ENTRY,
            cache_id: IconInfo::NO_CACHE_ID,
            bpp,
            width,
            height,
            bits_mask,
            color_table,
            bits_color,
        }
    }

    // oracle: spec MS-RDPERP §2.2.1.2.3 bottom-up DIB rows, palette entries are RGBQUAD, AND bit set = transparent
    #[test]
    fn palettized_icon_is_flipped_and_masked() {
        // 1 bpp, 2x2, 4-byte rows (DIB alignment). Bottom row first on the wire.
        let bits_color = vec![
            0b0100_0000,
            0,
            0,
            0, // bottom row: black, white
            0b1000_0000,
            0,
            0,
            0, // top row: white, black
        ];
        let bits_mask = vec![
            0b0000_0000,
            0,
            0,
            0, // bottom row: opaque
            0b0100_0000,
            0,
            0,
            0, // top row: second pixel transparent
        ];
        let color_table = vec![0, 0, 0, 0, 0xff, 0xff, 0xff, 0];

        let pixels = icon_to_bgra(&icon(1, 2, 2, bits_color, bits_mask, color_table)).unwrap();

        assert_eq!(
            pixels,
            [
                0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, // top row
                0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, // bottom row
            ]
        );
    }

    // oracle: mutant component order or bit replication in the RGB555 to 8-bit expansion
    #[test]
    fn rgb555_icon_expands_each_component() {
        // One pixel: red = 0x1f, green = 0, blue = 0x10, rows padded to 2 bytes.
        let value: u16 = (0x1f << 10) | 0x10;
        let pixels = icon_to_bgra(&icon(16, 1, 1, value.to_le_bytes().to_vec(), vec![0, 0], Vec::new())).unwrap();

        assert_eq!(pixels, [0x84, 0, 0xff, 0xff]);
    }

    // oracle: reference Win32 CreateIconIndirect uses a 32 bpp color bitmap's alpha and ignores the mask
    #[test]
    fn alpha_icon_ignores_the_mask() {
        let pixels = icon_to_bgra(&icon(32, 1, 1, vec![1, 2, 3, 0x80], vec![0x80, 0], Vec::new())).unwrap();

        assert_eq!(pixels, [1, 2, 3, 0x80]);
    }

    // oracle: spec MS-RDPERP §2.2.1.2.3 CbBitsColor covers Height rows of Width pixels at Bpp
    #[test]
    fn truncated_color_bits_are_rejected() {
        assert!(icon_to_bgra(&icon(24, 2, 2, vec![0; 6], Vec::new(), Vec::new())).is_none());
    }

    // oracle: reference Win32 CreateBitmap monochrome rows are WORD aligned, set AND bit = transparent
    #[test]
    fn mask_marks_transparent_pixels_with_word_aligned_rows() {
        let pixels = [0, 0, 0, 0, 0, 0, 0, 0xff, 0, 0, 0, 0xff, 0, 0, 0, 0];

        assert_eq!(mask_from_alpha(2, 2, &pixels), [0x80, 0, 0x40, 0]);
    }

    // oracle: spec MS-RDPERP §2.2.1.2.3 4 bpp packs two palette indices per byte, high nibble first
    #[test]
    fn four_bpp_icon_reads_both_nibbles() {
        // 2x1, one 2-byte row: indices 1 then 2.
        let color_table = vec![0, 0, 0, 0, 10, 20, 30, 0, 40, 50, 60, 0];
        let pixels = icon_to_bgra(&icon(4, 2, 1, vec![0x12, 0], vec![0, 0], color_table)).unwrap();

        assert_eq!(pixels, [10, 20, 30, 0xff, 40, 50, 60, 0xff]);
    }

    // oracle: spec MS-RDPERP §2.2.1.2.3 8 bpp is one palette index per byte, bottom-up rows
    #[test]
    fn eight_bpp_icon_indexes_the_palette_per_byte() {
        // 1x2, rows padded to 2 bytes: bottom row index 0, top row index 1.
        let color_table = vec![1, 2, 3, 0, 4, 5, 6, 0];
        let pixels = icon_to_bgra(&icon(8, 1, 2, vec![0, 0, 1, 0], vec![0, 0, 0, 0], color_table)).unwrap();

        assert_eq!(pixels, [4, 5, 6, 0xff, 1, 2, 3, 0xff]);
    }

    // oracle: mutant an index past the color table must not read out of bounds (no document decides its color; black is this client's choice)
    #[test]
    fn palette_indices_past_the_table_are_black() {
        let pixels = icon_to_bgra(&icon(8, 1, 1, vec![5, 0], vec![0, 0], vec![1, 2, 3, 0])).unwrap();

        assert_eq!(pixels, [0, 0, 0, 0xff]);
    }

    // oracle: spec MS-RDPERP §2.2.1.2.3 24 bpp pixels are BGR triplets in DWORD-aligned rows
    #[test]
    fn twenty_four_bpp_icon_reads_bgr_triplets() {
        // 1x1, row padded to 4 bytes.
        let pixels = icon_to_bgra(&icon(24, 1, 1, vec![7, 8, 9, 0], vec![0, 0], Vec::new())).unwrap();

        assert_eq!(pixels, [7, 8, 9, 0xff]);
    }

    // oracle: spec MS-RDPERP §2.2.1.2.3 CbBitsMask covers Height rows of the 1 bpp mask
    #[test]
    fn truncated_masks_are_rejected() {
        // 2x2 needs at least one byte per mask row.
        assert!(icon_to_bgra(&icon(24, 2, 2, vec![0; 16], vec![0], Vec::new())).is_none());
    }

    // oracle: mutant an icon without an AND mask must not read the mask (opaque is this client's choice)
    #[test]
    fn icons_without_a_mask_are_opaque() {
        let pixels = icon_to_bgra(&icon(24, 1, 1, vec![7, 8, 9, 0], Vec::new(), Vec::new())).unwrap();

        assert_eq!(pixels, [7, 8, 9, 0xff]);
    }

    fn arbitrary_icon() -> impl proptest::strategy::Strategy<Value = IconInfo> {
        use proptest::prelude::*;

        (
            prop::sample::select(vec![1u8, 4, 8, 16, 24, 32]),
            0u16..40,
            0u16..40,
            prop::collection::vec(any::<u8>(), 0..2048),
            prop::collection::vec(any::<u8>(), 0..512),
            prop::collection::vec(any::<u8>(), 0..1100),
        )
            .prop_map(|(bpp, width, height, bits_color, bits_mask, color_table)| {
                icon(bpp, width, height, bits_color, bits_mask, color_table)
            })
    }

    proptest::proptest! {
        // oracle: property server-controlled TS_ICON_INFO never panics the decoder and an
        // accepted icon has exactly Width x Height BGRA pixels
        #[test]
        fn malformed_icons_never_panic(icon in arbitrary_icon()) {
            if let Some(pixels) = icon_to_bgra(&icon) {
                proptest::prop_assert_eq!(pixels.len(), usize::from(icon.width) * usize::from(icon.height) * 4);
            }
        }
    }
}
