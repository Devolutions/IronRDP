//! ITU-T G.711 A-law and µ-law companding.
//!
//! These are the reference segment-search formulas, so they decode bit-exactly like the Windows
//! "Microsoft ALAW" and "Microsoft MULAW" codecs.

const SIGN_BIT: u8 = 0x80;
const QUANT_MASK: u8 = 0x0F;
const SEG_MASK: u8 = 0x70;
const SEG_SHIFT: u8 = 4;

const ULAW_BIAS: i16 = 0x84;
const ULAW_CLIP: i32 = 8159;

/// The µ-law bias as applied to the 14-bit magnitude.
const ULAW_BIAS_14_BIT: i32 = 0x84 >> 2;

/// Segment end points of the 13-bit A-law magnitude.
const ALAW_SEG_END: [i32; 8] = [0x1F, 0x3F, 0x7F, 0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF];

/// Segment end points of the 14-bit µ-law magnitude.
const ULAW_SEG_END: [i32; 8] = [0x3F, 0x7F, 0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF];

/// Expands one A-law code to a 16-bit linear sample.
pub fn alaw_to_linear(code: u8) -> i16 {
    let code = code ^ 0x55;
    // The largest magnitude is (0xF << 4 | 0x108) << 6 = 32256, so i16 arithmetic does not overflow.
    let mut magnitude = i16::from(code & QUANT_MASK) << 4;
    let segment = (code & SEG_MASK) >> SEG_SHIFT;
    match segment {
        0 => magnitude += 8,
        1 => magnitude += 0x108,
        _ => {
            magnitude += 0x108;
            magnitude <<= segment - 1;
        }
    }
    if code & SIGN_BIT != 0 { magnitude } else { -magnitude }
}

/// Compresses a 16-bit linear sample to one A-law code.
pub fn linear_to_alaw(sample: i16) -> u8 {
    let mut value = i32::from(sample) >> 3;
    let mask = if value < 0 {
        value = -value - 1;
        0x55
    } else {
        0xD5
    };

    let Some(segment) = ALAW_SEG_END.iter().position(|end| value <= *end) else {
        return 0x7F ^ mask;
    };
    let quant = if segment < 2 { value >> 1 } else { value >> segment };
    let code = (segment_bits(segment) << SEG_SHIFT) | (low_byte(quant) & QUANT_MASK);
    code ^ mask
}

/// Expands one µ-law code to a 16-bit linear sample.
pub fn ulaw_to_linear(code: u8) -> i16 {
    let code = !code;
    // The largest magnitude is ((0xF << 3) + 0x84) << 7 = 32256, so i16 arithmetic does not overflow.
    let mut magnitude = (i16::from(code & QUANT_MASK) << 3) + ULAW_BIAS;
    magnitude <<= (code & SEG_MASK) >> SEG_SHIFT;
    if code & SIGN_BIT != 0 {
        ULAW_BIAS - magnitude
    } else {
        magnitude - ULAW_BIAS
    }
}

/// Compresses a 16-bit linear sample to one µ-law code.
pub fn linear_to_ulaw(sample: i16) -> u8 {
    let mut value = i32::from(sample) >> 2;
    let mask = if value < 0 {
        value = -value;
        0x7F
    } else {
        0xFF
    };
    value = value.min(ULAW_CLIP) + ULAW_BIAS_14_BIT;

    let Some(segment) = ULAW_SEG_END.iter().position(|end| value <= *end) else {
        return 0x7F ^ mask;
    };
    let code = (segment_bits(segment) << SEG_SHIFT) | (low_byte(value >> (segment + 1)) & QUANT_MASK);
    code ^ mask
}

fn segment_bits(segment: usize) -> u8 {
    // INVARIANT: segments index an eight-entry table.
    u8::try_from(segment).expect("segment index fits in u8")
}

fn low_byte(value: i32) -> u8 {
    value.to_le_bytes()[0]
}
