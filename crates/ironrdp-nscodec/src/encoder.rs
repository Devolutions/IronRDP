//! MS-RDPNSC encoder.
//!
//! Implements the codec defined in MS-RDPNSC §3.1.5:
//! 1. RGB → YCoCg color-space conversion (lossy on chroma when CLL > 0).
//! 2. Optional 4:2:0 chroma subsampling — **not implemented**. The encoder
//!    always emits `ChromaSubsamplingLevel = 0` (full-resolution chroma).
//! 3. Per-plane RLE compression (custom MS-RDPNSC byte-level RLE).
//! 4. 20-byte frame header + concatenated Y, Co, Cg, A planes.
//!
//! The encoded byte stream is suitable to drop into the `bitmapData` of a
//! `TS_BITMAP_DATA_EX` carried by a `SurfaceBitsPdu` (MS-RDPBCGR §2.2.9.2.1);
//! that PDU plumbing belongs to the consumer (typically `ironrdp-server`).

use ironrdp_graphics::image_processing::PixelFormat;

/// 0xFF in the third byte of a run header signals "long run; read u32 LE next."
const RLE_LONG_ESCAPE: u8 = 0xFF;

/// Encode an in-memory bitmap as an NSCodec frame.
///
/// # Parameters
///
/// - `data` — pixel buffer in `format`, with `stride` bytes per row.
/// - `width`, `height` — dimensions in pixels (both must be non-zero).
/// - `stride` — bytes between the start of consecutive rows. Must be at least
///   `width * format.bytes_per_pixel()`.
/// - `format` — one of the eight 32-bpp `PixelFormat` variants.
/// - `color_loss_level` — must be 1..=7 per MS-RDPNSC. Higher = smaller output
///   but more chroma loss. The value passed here MUST match what was advertised
///   in the `NsCodec` capability set, or the client will decode against the
///   wrong shift and chroma will look wrong. We `debug_assert!` `>= 1`; at
///   CLL=0 the intermediate Co/Cg values overflow `i8` storage and produce
///   garbage chroma — callers should either clamp upstream or arrange for the
///   capability advertisement to never send CLL=0.
///
/// # Panics
///
/// Debug-asserts `color_loss_level >= 1` and `color_loss_level <= 7`. In
/// release builds the function does not panic on bad CLL but the output will
/// be undecodable (CLL=0) or shifted past zero precision (CLL>7).
pub fn encode(
    data: &[u8],
    width: u16,
    height: u16,
    stride: usize,
    format: PixelFormat,
    color_loss_level: u8,
) -> Vec<u8> {
    #![allow(clippy::similar_names)] // y_plane / co_plane / cg_plane / a_plane match the spec naming.

    debug_assert!(color_loss_level >= 1, "MS-RDPNSC CLL must be in 1..=7");
    debug_assert!(color_loss_level <= 7, "MS-RDPNSC CLL must be in 1..=7");

    let w = usize::from(width);
    let h = usize::from(height);
    let pixels = w * h;
    let cll = i32::from(color_loss_level);
    let bpp = usize::from(format.bytes_per_pixel());

    let mut y_plane = Vec::with_capacity(pixels);
    let mut co_plane = Vec::with_capacity(pixels);
    let mut cg_plane = Vec::with_capacity(pixels);
    let mut a_plane = Vec::with_capacity(pixels);

    // Surface Bits clients consume the bitmap data in bottom-up row order
    // (inherited from the legacy compressed bitmap convention in
    // MS-RDPBCGR §2.2.9.1.1.3.1.2.2, which `TS_BITMAP_DATA_EX` also follows).
    // Top-down inputs (e.g. macOS ScreenCaptureKit) need to be flipped here,
    // otherwise each dirty rect is rendered upside-down inside its bounding
    // box.
    for row in (0..h).rev() {
        let row_off = row * stride;
        for col in 0..w {
            let off = row_off + col * bpp;
            let p = &data[off..off + bpp];
            let (r, g, b, _a) = extract_rgba(format, p);
            let (y, co, cg) = rgb_to_ycocg(r, g, b, cll);
            y_plane.push(y);
            co_plane.push(co);
            cg_plane.push(cg);
            // Desktop captures are always opaque; the source `A` byte can be
            // zero on macOS (premultiplied / unused), and NSCodec clients
            // treat the alpha plane as actual blending — alpha=0 makes
            // everything transparent and the canvas renders as black.
            a_plane.push(0xFF);
        }
    }

    let y_rle = rle_encode(&y_plane);
    let co_rle = rle_encode(&co_plane);
    let cg_rle = rle_encode(&cg_plane);
    let a_rle = rle_encode(&a_plane);

    let plane_len = |rle: &[u8]| -> u32 {
        // RLE expansion is bounded by the plane size (worst case is unbounded
        // literals = `pixels` bytes plus a constant), which is at most
        // `u16::MAX * u16::MAX` = ~4.3 GB — comfortably u32. A u32::MAX cap is
        // defensive for the impossible-in-practice overflow case.
        u32::try_from(rle.len()).unwrap_or(u32::MAX)
    };

    let body_len = y_rle.len() + co_rle.len() + cg_rle.len() + a_rle.len();
    let mut out = Vec::with_capacity(20 + body_len);
    // 20-byte fixed header per MS-RDPNSC §2.2.1.x.
    out.extend_from_slice(&plane_len(&y_rle).to_le_bytes());
    out.extend_from_slice(&plane_len(&co_rle).to_le_bytes());
    out.extend_from_slice(&plane_len(&cg_rle).to_le_bytes());
    out.extend_from_slice(&plane_len(&a_rle).to_le_bytes());
    out.push(color_loss_level);
    out.push(0); // ChromaSubsamplingLevel = 0 (no chroma subsampling).
    out.push(0); // Reserved (2 bytes, MUST be 0).
    out.push(0);
    out.extend_from_slice(&y_rle);
    out.extend_from_slice(&co_rle);
    out.extend_from_slice(&cg_rle);
    out.extend_from_slice(&a_rle);

    out
}

/// Pull (R, G, B, A) out of a 4-byte pixel in the given format.
#[inline]
fn extract_rgba(fmt: PixelFormat, p: &[u8]) -> (u8, u8, u8, u8) {
    match fmt {
        PixelFormat::ARgb32 | PixelFormat::XRgb32 => (p[1], p[2], p[3], p[0]),
        PixelFormat::ABgr32 | PixelFormat::XBgr32 => (p[3], p[2], p[1], p[0]),
        PixelFormat::BgrA32 | PixelFormat::BgrX32 => (p[2], p[1], p[0], p[3]),
        PixelFormat::RgbA32 | PixelFormat::RgbX32 => (p[0], p[1], p[2], p[3]),
    }
}

/// RGB → (Y, Co, Cg) using the FreeRDP formulation (which Microsoft clients
/// decode against). Y is unsigned 0..=253; Co and Cg are signed values stored
/// in the `u8` bit pattern of their `i8` form.
///
/// Note on Co/Cg storage range: at the advertised CLL=3 typical of real
/// deployments, Co and Cg fit comfortably in `i8`. At CLL<3 they can overflow
/// — see the encoder doc-comment.
#[inline]
fn rgb_to_ycocg(r: u8, g: u8, b: u8, cll: i32) -> (u8, u8, u8) {
    #![allow(clippy::similar_names)] // co / cg / co_raw / cg_raw match the spec.

    let ri = i32::from(r);
    let gi = i32::from(g);
    let bi = i32::from(b);
    // y ∈ [0, 253] for r,g,b ∈ [0, 255] — always fits in u8.
    let y_i32 = (ri >> 2) + (gi >> 1) + (bi >> 2);
    let y = u8::try_from(y_i32.clamp(0, 255)).expect("clamped to [0, 255]");
    // At CLL ≥ 1 (debug-asserted by caller), co and cg ∈ [-128, 127] and fit
    // in i8; storing as u8 preserves the bit pattern.
    let co_raw = (ri - bi) >> cll;
    let cg_raw = (-(ri >> 1) + gi - (bi >> 1)) >> cll;
    let co = i8::try_from(co_raw.clamp(i32::from(i8::MIN), i32::from(i8::MAX)))
        .expect("clamped to i8 range")
        .cast_unsigned();
    let cg = i8::try_from(cg_raw.clamp(i32::from(i8::MIN), i32::from(i8::MAX)))
        .expect("clamped to i8 range")
        .cast_unsigned();
    (y, co, cg)
}

/// MS-RDPNSC RLE.
///
/// A run is introduced by a value byte appearing twice in succession; the
/// third byte is either `runlength - 2` (0..=253, runs of 2..=255) or `0xFF`
/// (long-run escape) followed by a 32-bit LE runlength. A single occurrence
/// of a value is a plain literal.
///
/// **The last 4 bytes of each plane are copied raw, *not* RLE-encoded** —
/// this matches the FreeRDP reference encoder, which Microsoft NSCodec
/// clients are written against. The decoder unconditionally reads the last
/// 4 bytes of compressed plane data as raw output, so emitting RLE there
/// makes the entire frame undecodable. This convention is implementation-
/// derived, not in the MS-RDPNSC text.
fn rle_encode(plane: &[u8]) -> Vec<u8> {
    let n = plane.len();
    if n <= 4 {
        // Plane too small for any RLE — emit raw.
        return plane.to_vec();
    }
    let body_end = n - 4;
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < body_end {
        let v = plane[i];
        let mut run = 1usize;
        while i + run < body_end && plane[i + run] == v {
            run += 1;
            // Run length must fit in u32 for the long-run wire encoding.
            // Cap here to avoid overflow on the eventual `to_le_bytes()`.
            if u32::try_from(run).is_err() {
                break;
            }
        }
        if run == 1 {
            out.push(v);
        } else if run <= 255 {
            out.push(v);
            out.push(v);
            // `run` is in 2..=255 so `run - 2` fits in u8.
            out.push(u8::try_from(run - 2).expect("run <= 255 implies run-2 fits in u8"));
        } else {
            out.push(v);
            out.push(v);
            out.push(RLE_LONG_ESCAPE);
            out.extend_from_slice(&u32::try_from(run).unwrap_or(u32::MAX).to_le_bytes());
        }
        i += run;
    }
    // Last 4 bytes of the plane are copied raw, by spec/convention.
    out.extend_from_slice(&plane[body_end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_short_input_is_raw() {
        // Inputs of <= 4 bytes can't have a raw 4-byte tail AND a body, so
        // the whole thing is emitted as-is.
        assert_eq!(rle_encode(&[7]), vec![7]);
        assert_eq!(rle_encode(&[7, 8]), vec![7, 8]);
        assert_eq!(rle_encode(&[1, 2, 3, 4]), vec![1, 2, 3, 4]);
    }

    #[test]
    fn rle_no_runs_in_body() {
        // 5-byte plane: body is 1 byte (plane[0]); tail is 4 bytes raw.
        // plane[0]=1 is a literal; remaining 4 bytes copied raw.
        assert_eq!(rle_encode(&[1, 2, 2, 2, 2]), vec![1, 2, 2, 2, 2]);
    }

    #[test]
    fn rle_short_run_in_body_with_raw_tail() {
        // 6 bytes of 7 -> body is plane[0..2] = [7, 7] -> short run of 2.
        // Tail: plane[2..6] = [7, 7, 7, 7].
        let plane = vec![7u8; 6];
        assert_eq!(rle_encode(&plane), vec![7, 7, 0, 7, 7, 7, 7]);
    }

    #[test]
    fn rle_long_run_in_body_with_raw_tail() {
        // 1000 bytes of 4 -> body 996 bytes of 4 -> long run, then 4 raw.
        let plane = vec![4u8; 1000];
        let mut want = vec![4, 4, RLE_LONG_ESCAPE];
        want.extend_from_slice(&996u32.to_le_bytes());
        want.extend_from_slice(&[4, 4, 4, 4]);
        assert_eq!(rle_encode(&plane), want);
    }

    #[test]
    fn ycocg_white_is_white() {
        // White (255,255,255) should give Y near 254 and Co/Cg near 0.
        let (y, co, cg) = rgb_to_ycocg(255, 255, 255, 3);
        assert_eq!(y, 253);
        assert_eq!(co, 0);
        assert_eq!(cg, 0);
    }

    #[test]
    fn ycocg_black_is_zero() {
        let (y, co, cg) = rgb_to_ycocg(0, 0, 0, 3);
        assert_eq!(y, 0);
        assert_eq!(co, 0);
        assert_eq!(cg, 0);
    }

    #[test]
    fn encode_emits_expected_header_size() {
        #![allow(clippy::similar_names)] // y_len / co_len / cg_len / a_len mirror the plane naming.

        // 2x2 solid red BgrA32. Each plane will be RLE-encoded; verify the
        // 20-byte header layout and that the total length is header + sum
        // of plane lengths.
        let data = vec![0, 0, 255, 0xFF, 0, 0, 255, 0xFF, 0, 0, 255, 0xFF, 0, 0, 255, 0xFF];
        let out = encode(&data, 2, 2, 8, PixelFormat::BgrA32, 3);
        assert!(out.len() >= 20, "header at minimum");
        let read_u32 = |slice: &[u8]| -> usize {
            usize::try_from(u32::from_le_bytes(slice.try_into().expect("4 bytes")))
                .expect("usize >= u32 on supported targets")
        };
        let y_len = read_u32(&out[0..4]);
        let co_len = read_u32(&out[4..8]);
        let cg_len = read_u32(&out[8..12]);
        let a_len = read_u32(&out[12..16]);
        assert_eq!(out[16], 3, "CLL stored in header");
        assert_eq!(out[17], 0, "ChromaSubsamplingLevel = 0");
        assert_eq!(&out[18..20], &[0, 0], "reserved = 0");
        assert_eq!(out.len(), 20 + y_len + co_len + cg_len + a_len);
    }
}
