//! Fits the server's pointer to what a browser draws well as a CSS cursor.
//!
//! A server may send its pointer inside a larger, mostly transparent bitmap:
//! a Linux guest's virtio-gpu cursor is 64x64 with the arrow in its top-left corner.
//! Cutting off the transparent margin keeps the arrow from shrinking when the pointer is scaled down
//! (see the `PointerBitmap` handling in `session.rs`).

use ironrdp::graphics::pointer::DecodedPointer;

/// `pointer` without its fully transparent margin.
///
/// Returns `None` when it has no margin to cut, is fully transparent, or its pixels do not match its size.
/// The hotspot stays inside, though it need not be on a visible pixel.
///
/// Alpha 0 means invisible here because the web client decodes pointers for hardware rendering,
/// where an inverted pixel is drawn opaque.
pub(crate) fn crop_transparent(pointer: &DecodedPointer) -> Option<DecodedPointer> {
    let (width, height) = (usize::from(pointer.width), usize::from(pointer.height));
    let (hot_x, hot_y) = (usize::from(pointer.hotspot_x), usize::from(pointer.hotspot_y));
    let row_len = width.checked_mul(4)?;
    if !(hot_x < width && hot_y < height && pointer.bitmap_data.len() == row_len.checked_mul(height)?) {
        return None;
    }

    let mut visible = pointer
        .bitmap_data
        .chunks_exact(row_len)
        .enumerate()
        .flat_map(|(y, row)| {
            row.chunks_exact(4)
                .enumerate()
                .filter(|(_, pixel)| pixel[3] != 0)
                .map(move |(x, _)| (x, y))
        });
    let (x, y) = visible.next()?;
    let start = (x.min(hot_x), y.min(hot_y));
    let end = (x.max(hot_x), y.max(hot_y));
    let ((left, top), (right, bottom)) = visible.fold((start, end), |((l, t), (r, b)), (x, y)| {
        ((l.min(x), t.min(y)), (r.max(x), b.max(y)))
    });

    // INVARIANT: left <= hot_x <= right < width and top <= hot_y <= bottom < height, all at most u16::MAX.
    // INVARIANT: The bitmap holds width * height pixels.
    #[expect(clippy::arithmetic_side_effects, reason = "per the invariants above")]
    {
        if (left, top, right, bottom) == (0, 0, width - 1, height - 1) {
            return None;
        }
        let bitmap_data = pointer
            .bitmap_data
            .chunks_exact(row_len)
            .skip(top)
            .take(bottom - top + 1)
            .flat_map(|row| &row[left * 4..(right + 1) * 4])
            .copied()
            .collect();
        Some(DecodedPointer {
            width: u16::try_from(right - left + 1).ok()?,
            height: u16::try_from(bottom - top + 1).ok()?,
            hotspot_x: u16::try_from(hot_x - left).ok()?,
            hotspot_y: u16::try_from(hot_y - top).ok()?,
            bitmap_data,
        })
    }
}

/// Largest pointer side a browser draws as a CSS cursor without falling back to its own (see `session.rs`).
const MAX_CURSOR_SIZE: u16 = 32;

/// A pointer's size and hotspot once scaled down to fit [`MAX_CURSOR_SIZE`].
pub(crate) struct Scaled {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) hotspot_x: u16,
    pub(crate) hotspot_y: u16,
}

/// The size and hotspot `pointer` is scaled to, or `None` when it already fits [`MAX_CURSOR_SIZE`].
pub(crate) fn scaled_to_fit(pointer: &DecodedPointer) -> Option<Scaled> {
    // INVARIANT: 0 < scale <= 1.0
    // INVARIANT: pointer.width * scale <= MAX_CURSOR_SIZE
    // INVARIANT: pointer.height * scale <= MAX_CURSOR_SIZE
    let scale = if pointer.width >= pointer.height && pointer.width > MAX_CURSOR_SIZE {
        f64::from(MAX_CURSOR_SIZE) / f64::from(pointer.width)
    } else if pointer.height > MAX_CURSOR_SIZE {
        f64::from(MAX_CURSOR_SIZE) / f64::from(pointer.height)
    } else {
        return None;
    };

    // Per invariants: Following conversions will never saturate.
    // A side is at least 1 pixel, which the resizer requires: a long, thin pointer would otherwise round to 0.
    // INVARIANT: 1 <= width <= MAX_CURSOR_SIZE and 1 <= height <= MAX_CURSOR_SIZE
    Some(Scaled {
        width: f64_to_u16_saturating_cast(f64::from(pointer.width) * scale).max(1),
        height: f64_to_u16_saturating_cast(f64::from(pointer.height) * scale).max(1),
        hotspot_x: f64_to_u16_saturating_cast(f64::from(pointer.hotspot_x) * scale),
        hotspot_y: f64_to_u16_saturating_cast(f64::from(pointer.hotspot_y) * scale),
    })
}

#[expect(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn f64_to_u16_saturating_cast(value: f64) -> u16 {
    value as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `width` x `height` pointer, transparent but for the pixels in `opaque`.
    fn pointer(width: u16, height: u16, hotspot: (u16, u16), opaque: &[(u16, u16)]) -> DecodedPointer {
        let mut bitmap_data = vec![0u8; usize::from(width) * usize::from(height) * 4];
        for &(x, y) in opaque {
            let at = (usize::from(y) * usize::from(width) + usize::from(x)) * 4;
            bitmap_data[at..at + 4].copy_from_slice(&[u8::try_from(x).unwrap(), u8::try_from(y).unwrap(), 0x80, 0xff]);
        }
        DecodedPointer {
            width,
            height,
            hotspot_x: hotspot.0,
            hotspot_y: hotspot.1,
            bitmap_data,
        }
    }

    fn pixel(p: &DecodedPointer, x: u16, y: u16) -> [u8; 4] {
        let at = (usize::from(y) * usize::from(p.width) + usize::from(x)) * 4;
        p.bitmap_data[at..at + 4].try_into().unwrap()
    }

    // oracle: mutant an off-by-one bound, a hotspot not moved with the crop, or pixels from the wrong rows or columns
    #[test]
    fn an_arrow_in_a_large_transparent_bitmap_is_cut_to_its_pixels() {
        let p = crop_transparent(&pointer(64, 64, (2, 3), &[(1, 2), (12, 20)])).unwrap();
        assert_eq!((p.width, p.height), (12, 19));
        assert_eq!((p.hotspot_x, p.hotspot_y), (1, 1));
        assert_eq!(p.bitmap_data.len(), 12 * 19 * 4);
        assert_eq!(pixel(&p, 0, 0), [1, 2, 0x80, 0xff]);
        assert_eq!(pixel(&p, 11, 18), [12, 20, 0x80, 0xff]);
        assert_eq!(pixel(&p, 1, 0), [0; 4]);
    }

    // oracle: mutant the hotspot left out of the cropped area
    #[test]
    fn a_hotspot_off_the_pixels_stays_inside() {
        let p = crop_transparent(&pointer(64, 64, (40, 0), &[(10, 10), (20, 30)])).unwrap();
        assert_eq!((p.width, p.height), (31, 31));
        assert_eq!((p.hotspot_x, p.hotspot_y), (30, 0));
        assert_eq!(pixel(&p, 0, 10), [10, 10, 0x80, 0xff]);
    }

    // oracle: mutant a pointer copied when there is nothing to cut
    #[test]
    fn a_pointer_with_no_margin_is_left_alone() {
        assert!(crop_transparent(&pointer(4, 4, (0, 0), &[(0, 0), (3, 3)])).is_none());
    }

    // oracle: mutant a panic or an empty crop when no pixel is visible
    #[test]
    fn a_fully_transparent_pointer_is_left_alone() {
        assert!(crop_transparent(&pointer(64, 64, (0, 0), &[])).is_none());
    }

    // oracle: mutant a panic on the empty pointer the decoder sends for a zero-size bitmap
    #[test]
    fn a_zero_size_pointer_is_left_alone() {
        assert!(crop_transparent(&DecodedPointer::new_invisible()).is_none());
    }

    // oracle: mutant the bitmap length check removed
    #[test]
    fn a_pointer_that_does_not_match_its_size_is_left_alone() {
        let mut p = pointer(64, 64, (0, 0), &[(1, 1)]);
        p.bitmap_data.pop();
        assert!(crop_transparent(&p).is_none());
    }

    // oracle: mutant the hotspot bounds check removed
    #[test]
    fn a_hotspot_outside_the_bitmap_is_left_alone() {
        assert!(crop_transparent(&pointer(8, 8, (8, 0), &[(1, 1)])).is_none());
    }

    // oracle: mutant the scale applied to only one side, or the hotspot left unscaled
    #[test]
    fn a_large_pointer_is_scaled_to_fit_with_its_hotspot() {
        let s = scaled_to_fit(&pointer(64, 48, (10, 20), &[])).unwrap();
        assert_eq!((s.width, s.height), (32, 24));
        assert_eq!((s.hotspot_x, s.hotspot_y), (5, 10));
    }

    // oracle: mutant a pointer that already fits scaled anyway
    #[test]
    fn a_pointer_that_fits_is_not_scaled() {
        assert!(scaled_to_fit(&pointer(32, 32, (0, 0), &[])).is_none());
    }

    // oracle: mutant a side rounded down to zero pixels, which the resizer refuses, ending the session
    #[test]
    fn a_long_thin_pointer_keeps_at_least_one_pixel_per_side() {
        let s = scaled_to_fit(&pointer(64, 1, (0, 0), &[])).unwrap();
        assert_eq!((s.width, s.height), (32, 1));
        let resizer = resize::new(
            64,
            1,
            usize::from(s.width),
            usize::from(s.height),
            resize::Pixel::RGBA8P,
            resize::Type::Lanczos3,
        );
        assert!(resizer.is_ok(), "the session's resizer must accept the scaled size");
    }
}
