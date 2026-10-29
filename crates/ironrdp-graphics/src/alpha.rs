//! Alpha codec ([MS-RDPEGFX] [2.2.4.3] `ALPHACODEC_BITMAP_STREAM`).
//!
//! The stream carries only per-pixel opacity, either raw (one byte per pixel) or run-length encoded
//! with `CLEARCODEC_ALPHA_RLE_SEGMENT` runs ([2.2.4.3.1]). The color channels of the target come
//! from an earlier command; this codec only replaces their alpha.
//!
//! [`decode`] hands the opacity to the caller run by run instead of materializing it: a twelve-byte
//! stream can describe billions of pixels, and the caller knows which of them actually exist.
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
//! [2.2.4.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/4c14f2a6-a025-476f-aa33-30f1718bd015
//! [2.2.4.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/9f003bdb-02fe-47b8-8d5e-c1422cdb4523

use ironrdp_core::ReadCursor;

/// `alphaSig`: the value every stream starts with ("AL", little-endian).
pub const ALPHA_SIGNATURE: u16 = 0x414C;

const HEADER_SIZE: usize = 2 /* alphaSig */ + 2 /* compressed */;

/// Error type for alpha codec decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlphaDecodeError {
    /// The stream is shorter than its header or than the data it announces.
    Truncated,
    /// `alphaSig` is not [`ALPHA_SIGNATURE`].
    InvalidSignature(u16),
    /// The decoded pixel count differs from the bitmap area, which 2.2.4.3 requires to match.
    PixelCountMismatch { expected: usize, decoded: usize },
}

impl core::fmt::Display for AlphaDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => write!(f, "alpha codec stream is truncated"),
            Self::InvalidSignature(signature) => write!(f, "invalid alpha codec signature {signature:#06x}"),
            Self::PixelCountMismatch { expected, decoded } => {
                write!(f, "alpha codec stream encodes {decoded} pixels, bitmap has {expected}")
            }
        }
    }
}

impl core::error::Error for AlphaDecodeError {}

/// Opacity for a run of consecutive pixels, in row-major order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaRun<'a> {
    /// One opacity byte per pixel.
    Values(&'a [u8]),
    /// `len` pixels sharing one opacity.
    Fill { value: u8, len: usize },
}

impl AlphaRun<'_> {
    /// Number of pixels the run covers.
    pub fn len(&self) -> usize {
        match self {
            Self::Values(values) => values.len(),
            Self::Fill { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Decode an `ALPHACODEC_BITMAP_STREAM` for a `width` x `height` bitmap.
///
/// `apply` receives each run with the row-major index of its first pixel. The stream is validated
/// in full before the first call, so a malformed stream applies nothing.
pub fn decode(
    data: &[u8],
    width: u16,
    height: u16,
    mut apply: impl FnMut(usize, AlphaRun<'_>),
) -> Result<(), AlphaDecodeError> {
    decode_runs(data, width, height, &mut apply)
}

/// The body of [`decode`], kept non-generic so it is compiled once rather than per callback type.
fn decode_runs(
    data: &[u8],
    width: u16,
    height: u16,
    apply: &mut dyn FnMut(usize, AlphaRun<'_>),
) -> Result<(), AlphaDecodeError> {
    let expected = usize::from(width) * usize::from(height);
    let mut src = ReadCursor::new(data);
    if src.len() < HEADER_SIZE {
        return Err(AlphaDecodeError::Truncated);
    }

    let signature = src.read_u16();
    if signature != ALPHA_SIGNATURE {
        return Err(AlphaDecodeError::InvalidSignature(signature));
    }
    let compressed = src.read_u16();

    if compressed == 0 {
        if src.len() < expected {
            return Err(AlphaDecodeError::Truncated);
        }
        if 0 < expected {
            apply(0, AlphaRun::Values(src.read_slice(expected)));
        }
        return Ok(());
    }

    // First pass: check that the runs cover exactly the bitmap, so nothing is applied from a
    // stream that turns out to be malformed.
    let segments = src;
    let mut covered = 0usize;
    let mut pass = segments.clone();
    while covered < expected {
        let (_, run_length) = read_segment(&mut pass)?;
        covered = check_run(covered, run_length, expected)?;
    }
    // Segments past the area may only be empty runs: 2.2.4.3 requires the segments to encode
    // exactly the area, so a run past it is an error rather than something to ignore.
    while !pass.is_empty() {
        let (_, run_length) = read_segment(&mut pass)?;
        check_run(covered, run_length, expected)?;
    }

    let mut pass = segments;
    let mut first_pixel = 0usize;
    while first_pixel < expected {
        let (value, len) = read_segment(&mut pass)?;
        if len > 0 {
            apply(first_pixel, AlphaRun::Fill { value, len });
        }
        first_pixel += len;
    }

    Ok(())
}

/// Advance the covered pixel count by one run, refusing a run past the end of the bitmap.
fn check_run(covered: usize, run_length: usize, expected: usize) -> Result<usize, AlphaDecodeError> {
    let remaining = expected - covered;
    if run_length > remaining {
        return Err(AlphaDecodeError::PixelCountMismatch {
            expected,
            decoded: covered.saturating_add(run_length),
        });
    }
    Ok(covered + run_length)
}

/// Read one `CLEARCODEC_ALPHA_RLE_SEGMENT` (2.2.4.3.1): a value and its run length, whose
/// encoding widens from one to two to four bytes behind the 0xFF and 0xFFFF escapes.
fn read_segment(src: &mut ReadCursor<'_>) -> Result<(u8, usize), AlphaDecodeError> {
    if src.len() < 2 {
        return Err(AlphaDecodeError::Truncated);
    }
    let value = src.read_u8();
    let factor1 = src.read_u8();
    if factor1 < 0xFF {
        return Ok((value, usize::from(factor1)));
    }

    if src.len() < 2 {
        return Err(AlphaDecodeError::Truncated);
    }
    let factor2 = src.read_u16();
    if factor2 < 0xFFFF {
        return Ok((value, usize::from(factor2)));
    }

    if src.len() < 4 {
        return Err(AlphaDecodeError::Truncated);
    }
    let factor3 = src.read_u32();
    // A run longer than the address space cannot fit any bitmap, so saturate and let the
    // pixel-count check reject it.
    Ok((value, usize::try_from(factor3).unwrap_or(usize::MAX)))
}
