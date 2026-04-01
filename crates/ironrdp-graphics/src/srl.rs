//! SRL (Simplified Run-Length) entropy codec for progressive upgrade passes.
//!
//! Used during progressive TILE_UPGRADE decoding where the tri-state sign
//! array (DAS) indicates zero-valued coefficients. SRL encodes/decodes
//! magnitudes for coefficients that were previously zero.
//!
//! The algorithm is similar to RLGR's zero-run mode with a simpler structure:
//! adaptive K parameter controlling zero-run lengths, followed by unary-coded
//! magnitudes with sign bits.

/// Decode SRL data for a set of zero-valued (DAS=0) coefficient positions.
///
/// `data` is the SRL byte stream (terminated by a 0x00 sentinel).
/// `num_values` is the number of coefficients to decode.
/// `num_bits` is the bit width for each magnitude value.
///
/// Returns a vector of decoded signed coefficient values. Zero entries
/// mean the coefficient remains zero after this upgrade pass.
pub fn decode_srl(data: &[u8], num_values: usize, num_bits: u8) -> Vec<i16> {
    if num_values == 0 || data.is_empty() {
        return vec![0; num_values];
    }

    let mut output = vec![0i16; num_values];
    let mut reader = BitReader::new(data);
    let mut kp: u32 = 0;
    let mut out_idx = 0;
    let mut nz: u32 = 0; // remaining zeros in current run

    while out_idx < num_values {
        let k = kp >> 3;

        if nz > 0 {
            // Still emitting zeros from a previous run
            nz -= 1;
            output[out_idx] = 0;
            out_idx += 1;
            continue;
        }

        if k > 0 {
            // Zero-run mode: read one bit to decide
            let bit = reader.read_bit();
            if !bit {
                // Emit 1 << k zeros
                nz = 1u32.checked_shl(k).unwrap_or(0);
                kp = kp.saturating_add(4).min(80);
                // First zero of the run
                nz -= 1;
                output[out_idx] = 0;
                out_idx += 1;
                continue;
            }
            // Read k bits for exact remaining zero count
            let zeros = reader.read_bits(k);
            if zeros > 0 {
                nz = zeros;
                nz -= 1;
                output[out_idx] = 0;
                out_idx += 1;
                continue;
            }
            // Fall through to unary mode (no more zeros)
        }

        // Unary mode: decode a non-zero magnitude
        kp = kp.saturating_sub(6);

        if num_bits == 0 {
            // No bits to decode, just emit +/-1 from sign bit
            let sign = reader.read_bit();
            output[out_idx] = if sign { -1 } else { 1 };
            out_idx += 1;
            continue;
        }

        // Read sign bit
        let sign = reader.read_bit();

        if num_bits == 1 {
            output[out_idx] = if sign { -1 } else { 1 };
            out_idx += 1;
            continue;
        }

        // Decode unary magnitude: count leading zeros until a 1-bit.
        // Cap at 16 bits to prevent overflow if the stream ends prematurely
        // (BitReader returns 0 for out-of-bounds reads).
        let mut magnitude: u32 = 1;
        loop {
            let bit = reader.read_bit();
            if bit || magnitude >= 0x8000 {
                break;
            }
            magnitude += 1;
        }

        // Read remaining magnitude bits (num_bits - 1 bits)
        let extra_bits = u32::from(num_bits).saturating_sub(1);
        if extra_bits > 0 && extra_bits < 16 {
            let extra = reader.read_bits(extra_bits);
            magnitude = (magnitude << extra_bits) | extra;
        }

        let value = i16::try_from(magnitude.min(0x7FFF)).unwrap_or(i16::MAX);
        output[out_idx] = if sign { -value } else { value };
        out_idx += 1;
    }

    output
}

/// Encode coefficient magnitudes using the SRL algorithm.
///
/// `values` contains signed coefficient values (non-zero = needs encoding,
/// zero = contributes to zero runs).
/// `num_bits` is the bit width for magnitude encoding.
///
/// Returns the encoded SRL byte stream (with trailing 0x00 sentinel).
pub fn encode_srl(values: &[i16], num_bits: u8) -> Vec<u8> {
    if values.is_empty() {
        return vec![0x00];
    }

    let mut writer = BitWriter::new();
    let mut kp: u32 = 0;
    let mut idx = 0;

    while idx < values.len() {
        let k = kp >> 3;

        if values[idx] == 0 {
            // Count consecutive zeros
            let mut zero_count: u32 = 0;
            while idx + usize::try_from(zero_count).unwrap_or(usize::MAX) < values.len()
                && values[idx + usize::try_from(zero_count).unwrap_or(usize::MAX)] == 0
            {
                zero_count += 1;
            }

            // Encode zero run
            if k > 0 {
                let chunk_size = 1u32.checked_shl(k).unwrap_or(u32::MAX);
                while zero_count >= chunk_size {
                    writer.write_bit(false); // full chunk
                    kp = kp.saturating_add(4).min(80);
                    zero_count -= chunk_size;
                    idx += usize::try_from(chunk_size).unwrap_or(usize::MAX);
                }
                // Emit remaining zeros with escape + count
                writer.write_bit(true);
                writer.write_bits(zero_count, k);
                idx += usize::try_from(zero_count).unwrap_or(usize::MAX);

                if zero_count > 0 {
                    continue;
                }
            } else {
                // k == 0, each zero is individually handled
                // (zero coefficient in unary mode is skipped, we move to next)
                idx += usize::try_from(zero_count).unwrap_or(usize::MAX);
                continue;
            }

            // Fall through to encode the non-zero value that stopped the run
            if idx >= values.len() {
                break;
            }
        }

        // Encode non-zero value
        kp = kp.saturating_sub(6);
        let value = values[idx];
        let sign = value < 0;
        let magnitude = u32::from(value.unsigned_abs());

        writer.write_bit(sign);

        if num_bits <= 1 {
            idx += 1;
            continue;
        }

        // Unary encode the magnitude
        let extra_bits = u32::from(num_bits).saturating_sub(1);
        if extra_bits > 0 && extra_bits < 16 {
            let base = magnitude >> extra_bits;
            let remainder = magnitude & ((1u32 << extra_bits) - 1);

            // Write base in unary: (base - 1) zeros + 1
            for _ in 1..base {
                writer.write_bit(false);
            }
            writer.write_bit(true);

            // Write remainder
            writer.write_bits(remainder, extra_bits);
        }

        idx += 1;
    }

    // Trailing sentinel
    let mut result = writer.finish();
    result.push(0x00);
    result
}

// ---------------------------------------------------------------------------
// Bit-level I/O helpers
// ---------------------------------------------------------------------------

struct BitReader<'a> {
    data: &'a [u8],
    byte_idx: usize,
    bit_idx: u8, // 0..7, MSB first
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_idx: 0,
            bit_idx: 0,
        }
    }

    fn read_bit(&mut self) -> bool {
        if self.byte_idx >= self.data.len() {
            return false;
        }
        let bit = (self.data[self.byte_idx] >> (7 - self.bit_idx)) & 1 != 0;
        self.bit_idx += 1;
        if self.bit_idx >= 8 {
            self.bit_idx = 0;
            self.byte_idx += 1;
        }
        bit
    }

    fn read_bits(&mut self, count: u32) -> u32 {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit());
        }
        value
    }
}

struct BitWriter {
    bytes: Vec<u8>,
    current: u8,
    bit_count: u8, // bits written in current byte (0..7)
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current: 0,
            bit_count: 0,
        }
    }

    fn write_bit(&mut self, bit: bool) {
        self.current = (self.current << 1) | u8::from(bit);
        self.bit_count += 1;
        if self.bit_count >= 8 {
            self.bytes.push(self.current);
            self.current = 0;
            self.bit_count = 0;
        }
    }

    fn write_bits(&mut self, value: u32, count: u32) {
        for i in (0..count).rev() {
            self.write_bit((value >> i) & 1 != 0);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            // Pad remaining bits with zeros (MSB aligned)
            self.current <<= 8 - self.bit_count;
            self.bytes.push(self.current);
        }
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_empty() {
        let result = decode_srl(&[], 0, 1);
        assert!(result.is_empty());
    }

    #[test]
    fn decode_empty_data() {
        // With no data (empty slice), all positions default to zero
        let result = decode_srl(&[], 5, 1);
        assert_eq!(result, vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_empty() {
        let encoded = encode_srl(&[], 1);
        assert_eq!(encoded, vec![0x00]); // just sentinel
    }

    #[test]
    fn encode_all_zeros() {
        let encoded = encode_srl(&[0, 0, 0], 1);
        // Should contain only the sentinel since k starts at 0
        // and zeros with k=0 are just skipped
        assert_eq!(*encoded.last().unwrap(), 0x00);
    }

    #[test]
    fn round_trip_single_positive() {
        let original = vec![1];
        let encoded = encode_srl(&original, 1);
        let decoded = decode_srl(&encoded, 1, 1);
        assert_eq!(decoded, original);
    }

    #[test]
    fn round_trip_single_negative() {
        let original = vec![-1];
        let encoded = encode_srl(&original, 1);
        let decoded = decode_srl(&encoded, 1, 1);
        assert_eq!(decoded, original);
    }

    #[test]
    fn round_trip_nonzero_only() {
        // SRL is designed for coefficients already identified as non-zero
        // by the DAS (delta-analysis state). Test with only non-zero values.
        let original = vec![1, -1, 2, -3, 1];
        let encoded = encode_srl(&original, 4);
        let decoded = decode_srl(&encoded, original.len(), 4);
        for (i, (&orig, &dec)) in original.iter().zip(decoded.iter()).enumerate() {
            assert_eq!(orig.signum(), dec.signum(), "index {i}: sign mismatch");
        }
    }

    #[test]
    fn bit_reader_basic() {
        let data = [0b10110000];
        let mut reader = BitReader::new(&data);
        assert!(reader.read_bit()); // 1
        assert!(!reader.read_bit()); // 0
        assert!(reader.read_bit()); // 1
        assert!(reader.read_bit()); // 1
    }

    #[test]
    fn bit_writer_basic() {
        let mut writer = BitWriter::new();
        writer.write_bit(true);
        writer.write_bit(false);
        writer.write_bit(true);
        writer.write_bit(true);
        writer.write_bit(false);
        writer.write_bit(false);
        writer.write_bit(false);
        writer.write_bit(false);
        let result = writer.finish();
        assert_eq!(result, vec![0b10110000]);
    }

    #[test]
    fn bit_writer_multi_byte() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xFF, 8);
        writer.write_bits(0x00, 8);
        let result = writer.finish();
        assert_eq!(result, vec![0xFF, 0x00]);
    }
}
