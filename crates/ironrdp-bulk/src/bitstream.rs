//! Bitstream reader and writer utilities for compression algorithms.
//!
//! Ported from FreeRDP's `winpr/include/winpr/bitstream.h`.
//! The reader uses a 32-bit accumulator with a 32-bit prefetch lookahead,
//! loaded in big-endian order. The writer uses a 32-bit accumulator that
//! flushes in big-endian order.

/// Reads bits from a byte buffer using a 32-bit accumulator with prefetch.
///
/// This is a faithful port of FreeRDP's `wBitStream` read operations.
/// The accumulator holds 32 bits loaded big-endian from the buffer.
/// Bits are consumed from the most significant bit (MSB) first.
pub(crate) struct BitStreamReader<'a> {
    buffer: &'a [u8],
    /// Byte offset of the current 4-byte window in the buffer.
    byte_position: usize,
    /// Total number of bits consumed so far.
    bits_consumed: usize,
    /// Number of bits consumed within the current 4-byte accumulator window.
    offset: u32,
    /// Current 32-bit accumulator (big-endian loaded).
    accumulator: u32,
    /// Prefetched next 32-bit word (big-endian loaded).
    prefetch: u32,
    /// Total number of bits available in the buffer.
    total_bits: usize,
}

impl<'a> BitStreamReader<'a> {
    /// Creates a new BitStreamReader attached to the given byte buffer.
    ///
    /// Immediately fetches the first 4 bytes into the accumulator
    /// and prefetches the next 4 bytes.
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut reader = Self {
            buffer: data,
            byte_position: 0,
            bits_consumed: 0,
            offset: 0,
            accumulator: 0,
            prefetch: 0,
            total_bits: data.len().saturating_mul(8),
        };
        reader.fetch();
        reader
    }

    /// Returns the current accumulator value.
    ///
    /// The top bits of the accumulator contain the next bits to be consumed.
    /// This is used by algorithms like MPPC that inspect bit patterns
    /// directly before deciding how many bits to shift.
    #[inline]
    pub(crate) fn accumulator(&self) -> u32 {
        self.accumulator
    }

    /// Reads `nbits` bits from the stream and returns them.
    ///
    /// The returned value contains the bits right-aligned (in the lowest `nbits` bits).
    ///
    /// # Panics
    ///
    /// Panics if `nbits` is 0 or greater than 32.
    #[inline]
    pub(crate) fn read_bits(&mut self, nbits: u32) -> u32 {
        debug_assert!(nbits > 0 && nbits <= 32, "nbits must be 1..=32, got {nbits}");
        let value = self.peek_bits(nbits);
        self.shift(nbits);
        value
    }

    /// Peeks at the top `nbits` bits of the accumulator without consuming them.
    ///
    /// The returned value contains the bits right-aligned.
    #[inline]
    pub(crate) fn peek_bits(&self, nbits: u32) -> u32 {
        if nbits == 32 {
            self.accumulator
        } else {
            self.accumulator >> (32 - nbits)
        }
    }

    /// Returns the number of bits remaining in the stream.
    #[inline]
    pub(crate) fn remaining_bits(&self) -> usize {
        self.total_bits.saturating_sub(self.bits_consumed)
    }

    /// Returns the total number of bits consumed so far.
    #[inline]
    pub(crate) fn bits_consumed(&self) -> usize {
        self.bits_consumed
    }

    /// Advances the stream by `nbits` bits.
    ///
    /// This is the Rust equivalent of FreeRDP's `BitStream_Shift`.
    /// It shifts the accumulator left and fills from the prefetch buffer.
    /// When crossing a 32-bit boundary, it advances the byte pointer
    /// and re-prefetches.
    #[expect(clippy::as_conversions, reason = "nbits (u32 ≤ 31) always fits in usize")]
    pub(crate) fn shift(&mut self, nbits: u32) {
        if nbits == 0 {
            return;
        }

        debug_assert!(nbits < 32, "use shift32() for shifting 32 bits");

        self.accumulator <<= nbits;
        self.bits_consumed += nbits as usize;
        self.offset += nbits;

        if self.offset < 32 {
            // Still within the same 4-byte window.
            // Fill lower bits of accumulator from top of prefetch.
            let mask = (1u32 << nbits) - 1;
            self.accumulator |= (self.prefetch >> (32 - nbits)) & mask;
            self.prefetch <<= nbits;
        } else {
            // Crossed 32-bit boundary.
            // First fill from remaining prefetch bits.
            let mask = (1u32 << nbits) - 1;
            self.accumulator |= (self.prefetch >> (32 - nbits)) & mask;
            self.prefetch <<= nbits;

            self.offset -= 32;
            self.byte_position += 4;
            self.do_prefetch();

            if self.offset > 0 {
                let mask = (1u32 << self.offset) - 1;
                self.accumulator |= (self.prefetch >> (32 - self.offset)) & mask;
                self.prefetch <<= self.offset;
            }
        }
    }

    /// Shifts 32 bits by performing two 16-bit shifts.
    ///
    /// Equivalent to FreeRDP's `BitStream_Shift32`.
    pub(crate) fn shift32(&mut self) {
        self.shift(16);
        self.shift(16);
    }

    /// Loads the accumulator with 4 bytes from the current position (big-endian)
    /// and prefetches the next 4 bytes.
    fn fetch(&mut self) {
        self.accumulator = 0;
        let pos = self.byte_position;
        let cap = self.buffer.len();

        if pos < cap {
            self.accumulator |= u32::from(self.buffer[pos]) << 24;
        }
        if pos + 1 < cap {
            self.accumulator |= u32::from(self.buffer[pos + 1]) << 16;
        }
        if pos + 2 < cap {
            self.accumulator |= u32::from(self.buffer[pos + 2]) << 8;
        }
        if pos + 3 < cap {
            self.accumulator |= u32::from(self.buffer[pos + 3]);
        }

        self.do_prefetch();
    }

    /// Prefetches 4 bytes starting at `byte_position + 4` (big-endian).
    fn do_prefetch(&mut self) {
        self.prefetch = 0;
        let pos = self.byte_position + 4;
        let cap = self.buffer.len();

        if pos < cap {
            self.prefetch |= u32::from(self.buffer[pos]) << 24;
        }
        if pos + 1 < cap {
            self.prefetch |= u32::from(self.buffer[pos + 1]) << 16;
        }
        if pos + 2 < cap {
            self.prefetch |= u32::from(self.buffer[pos + 2]) << 8;
        }
        if pos + 3 < cap {
            self.prefetch |= u32::from(self.buffer[pos + 3]);
        }
    }
}

/// Writes bits to a byte buffer using a 32-bit accumulator.
///
/// This is a faithful port of FreeRDP's `wBitStream` write operations.
/// Bits are written from the most significant bit (MSB) first.
/// When the 32-bit accumulator is full, it is flushed to the buffer
/// in big-endian order.
pub(crate) struct BitStreamWriter<'a> {
    buffer: &'a mut [u8],
    /// Byte offset where the next 4-byte flush will write.
    byte_position: usize,
    /// Total number of bits written so far.
    bits_written: usize,
    /// Number of bits written within the current 4-byte accumulator.
    offset: u32,
    /// Current 32-bit accumulator (big-endian, MSB first).
    accumulator: u32,
}

impl<'a> BitStreamWriter<'a> {
    /// Creates a new BitStreamWriter targeting the given byte buffer.
    pub(crate) fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            byte_position: 0,
            bits_written: 0,
            offset: 0,
            accumulator: 0,
        }
    }

    /// Writes `nbits` bits from `value` into the stream.
    ///
    /// The bits are taken from the lowest `nbits` bits of `value`.
    /// They are placed MSB-first into the accumulator. When the
    /// accumulator fills 32 bits, it is flushed to the buffer.
    ///
    /// This is the Rust equivalent of FreeRDP's `BitStream_Write_Bits`.
    #[expect(clippy::as_conversions, reason = "nbits (u32 ≤ 32) always fits in usize")]
    pub(crate) fn write_bits(&mut self, value: u32, nbits: u32) {
        self.bits_written += nbits as usize;
        self.offset += nbits;

        if self.offset < 32 {
            // Fits within the current accumulator.
            // Place bits at position (32 - offset), which is just after
            // the previously written bits.
            self.accumulator |= value << (32 - self.offset);
        } else {
            // Crossed the 32-bit boundary.
            self.offset -= 32;

            // Put the upper (nbits - offset) bits into the current accumulator.
            let mask = (1u32 << (nbits - self.offset)) - 1;
            self.accumulator |= (value >> self.offset) & mask;

            // Flush the full accumulator to the buffer.
            self.do_flush();
            self.accumulator = 0;
            self.byte_position += 4;

            // Put the remaining lower `offset` bits into the new accumulator.
            if self.offset > 0 {
                let mask = (1u32 << self.offset) - 1;
                self.accumulator |= (value & mask) << (32 - self.offset);
            }
        }
    }

    /// Flushes any remaining bits in the accumulator to the output buffer.
    ///
    /// This must be called after all bits have been written to ensure
    /// any partial accumulator contents are written to the buffer.
    ///
    /// This is the Rust equivalent of FreeRDP's `BitStream_Flush`.
    pub(crate) fn flush(&mut self) {
        self.do_flush();
    }

    /// Returns the total number of bits written so far.
    #[inline]
    pub(crate) fn bits_written(&self) -> usize {
        self.bits_written
    }

    /// Returns the number of bytes needed to hold all written bits,
    /// rounding up for any partial byte.
    ///
    /// Equivalent to `(bs->position + 7) / 8` in FreeRDP.
    #[inline]
    pub(crate) fn byte_length(&self) -> usize {
        self.bits_written.div_ceil(8)
    }

    /// Writes the accumulator bytes to the buffer in big-endian order.
    fn do_flush(&mut self) {
        let pos = self.byte_position;
        let cap = self.buffer.len();
        let bytes = self.accumulator.to_be_bytes();

        if pos < cap {
            self.buffer[pos] = bytes[0];
        }
        if pos + 1 < cap {
            self.buffer[pos + 1] = bytes[1];
        }
        if pos + 2 < cap {
            self.buffer[pos + 2] = bytes[2];
        }
        if pos + 3 < cap {
            self.buffer[pos + 3] = bytes[3];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================
    // BitStreamReader tests
    // ========================

    #[test]
    fn reader_read_single_bits() {
        // 0xA5 = 1010_0101
        let data = [0xA5];
        let mut reader = BitStreamReader::new(&data);

        assert_eq!(reader.read_bits(1), 1); // bit 7: 1
        assert_eq!(reader.read_bits(1), 0); // bit 6: 0
        assert_eq!(reader.read_bits(1), 1); // bit 5: 1
        assert_eq!(reader.read_bits(1), 0); // bit 4: 0
        assert_eq!(reader.read_bits(1), 0); // bit 3: 0
        assert_eq!(reader.read_bits(1), 1); // bit 2: 1
        assert_eq!(reader.read_bits(1), 0); // bit 1: 0
        assert_eq!(reader.read_bits(1), 1); // bit 0: 1

        assert_eq!(reader.bits_consumed(), 8);
        assert_eq!(reader.remaining_bits(), 0);
    }

    #[test]
    fn reader_read_8_bits() {
        let data = [0xDE, 0xAD];
        let mut reader = BitStreamReader::new(&data);

        assert_eq!(reader.read_bits(8), 0xDE);
        assert_eq!(reader.read_bits(8), 0xAD);
        assert_eq!(reader.remaining_bits(), 0);
    }

    #[test]
    fn reader_read_16_bits() {
        let data = [0xCA, 0xFE, 0xBA, 0xBE];
        let mut reader = BitStreamReader::new(&data);

        assert_eq!(reader.read_bits(16), 0xCAFE);
        assert_eq!(reader.read_bits(16), 0xBABE);
        assert_eq!(reader.remaining_bits(), 0);
    }

    #[test]
    fn reader_read_mixed_widths() {
        // 0b1100_1010 0b0011_1111 = 0xCA3F
        let data = [0xCA, 0x3F];
        let mut reader = BitStreamReader::new(&data);

        assert_eq!(reader.read_bits(4), 0b1100); // top 4 bits of 0xCA
        assert_eq!(reader.read_bits(4), 0b1010); // bottom 4 bits of 0xCA
        assert_eq!(reader.read_bits(6), 0b001111); // top 6 bits of 0x3F
        assert_eq!(reader.read_bits(2), 0b11); // bottom 2 bits of 0x3F
        assert_eq!(reader.bits_consumed(), 16);
    }

    #[test]
    fn reader_accumulator_boundary_crossing() {
        // 8 bytes = 64 bits, need to cross the 32-bit accumulator boundary
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let mut reader = BitStreamReader::new(&data);

        // Read first 32 bits (full first accumulator window)
        assert_eq!(reader.read_bits(16), 0x0102);
        assert_eq!(reader.read_bits(16), 0x0304);

        // This crosses into the prefetch/second window
        assert_eq!(reader.read_bits(16), 0x0506);
        assert_eq!(reader.read_bits(16), 0x0708);
        assert_eq!(reader.remaining_bits(), 0);
    }

    #[test]
    fn reader_cross_boundary_odd_width() {
        // Read across the 32-bit boundary with an odd-sized read
        let data = [0xFF, 0x00, 0xFF, 0x00, 0xAA, 0xBB, 0xCC, 0xDD];
        let mut reader = BitStreamReader::new(&data);

        // Read 28 bits (within first window)
        assert_eq!(reader.read_bits(28), 0xFF00FF0);
        // Read 8 bits (crosses the 32-bit boundary: 4 bits from first window + 4 from second)
        assert_eq!(reader.read_bits(8), 0x0A);
        // Continue reading from second window
        assert_eq!(reader.read_bits(16), 0xABBC);
    }

    #[test]
    fn reader_peek_does_not_consume() {
        let data = [0xAB, 0xCD];
        let mut reader = BitStreamReader::new(&data);

        assert_eq!(reader.peek_bits(8), 0xAB);
        assert_eq!(reader.peek_bits(8), 0xAB); // same value
        assert_eq!(reader.bits_consumed(), 0);

        assert_eq!(reader.read_bits(8), 0xAB); // now consume
        assert_eq!(reader.peek_bits(8), 0xCD);
    }

    #[test]
    fn reader_accumulator_direct_access() {
        let data = [0xDE, 0xAD, 0xBE, 0xEF];
        let reader = BitStreamReader::new(&data);

        // Accumulator should hold all 4 bytes in big-endian order
        assert_eq!(reader.accumulator(), 0xDEADBEEF);
    }

    #[test]
    fn reader_shift32() {
        let data = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let mut reader = BitStreamReader::new(&data);

        assert_eq!(reader.accumulator(), 0x11223344);
        reader.shift32();
        assert_eq!(reader.accumulator(), 0x55667788);
        assert_eq!(reader.bits_consumed(), 32);
    }

    #[test]
    fn reader_small_buffer() {
        // Buffer smaller than 4 bytes
        let data = [0xAB, 0xCD];
        let reader = BitStreamReader::new(&data);

        // Accumulator pads with zeros for missing bytes
        assert_eq!(reader.accumulator(), 0xABCD0000);
        assert_eq!(reader.remaining_bits(), 16);
    }

    #[test]
    fn reader_empty_buffer() {
        let data: [u8; 0] = [];
        let reader = BitStreamReader::new(&data);

        assert_eq!(reader.accumulator(), 0);
        assert_eq!(reader.remaining_bits(), 0);
    }

    // ========================
    // BitStreamWriter tests
    // ========================

    /// Helper: write bits, flush, and return (byte_length, bits_written) before
    /// releasing the mutable borrow on the buffer.
    fn write_and_flush(buf: &mut [u8], ops: &[(u32, u32)]) -> (usize, usize) {
        let mut writer = BitStreamWriter::new(buf);
        for &(value, nbits) in ops {
            writer.write_bits(value, nbits);
        }
        writer.flush();
        (writer.byte_length(), writer.bits_written())
    }

    #[test]
    fn writer_write_8_bits() {
        let mut buf = [0u8; 4];
        let (byte_len, _) = write_and_flush(&mut buf, &[(0xAB, 8)]);

        assert_eq!(buf[0], 0xAB);
        assert_eq!(byte_len, 1);
    }

    #[test]
    fn writer_write_16_bits() {
        let mut buf = [0u8; 4];
        let (byte_len, _) = write_and_flush(&mut buf, &[(0xCAFE, 16)]);

        assert_eq!(buf[0], 0xCA);
        assert_eq!(buf[1], 0xFE);
        assert_eq!(byte_len, 2);
    }

    #[test]
    fn writer_write_single_bits() {
        let mut buf = [0u8; 4];
        // Write 1010_0101 one bit at a time
        let (_, bits_written) = write_and_flush(
            &mut buf,
            &[(1, 1), (0, 1), (1, 1), (0, 1), (0, 1), (1, 1), (0, 1), (1, 1)],
        );

        assert_eq!(buf[0], 0xA5);
        assert_eq!(bits_written, 8);
    }

    #[test]
    fn writer_write_mixed_widths() {
        let mut buf = [0u8; 4];
        write_and_flush(&mut buf, &[(0b1100, 4), (0b1010, 4)]);

        assert_eq!(buf[0], 0xCA);
    }

    #[test]
    fn writer_accumulator_boundary_crossing() {
        let mut buf = [0u8; 8];
        let (byte_len, _) = write_and_flush(&mut buf, &[(0xDEAD, 16), (0xBEEF, 16), (0xCAFE, 16)]);

        assert_eq!(buf[0], 0xDE);
        assert_eq!(buf[1], 0xAD);
        assert_eq!(buf[2], 0xBE);
        assert_eq!(buf[3], 0xEF);
        assert_eq!(buf[4], 0xCA);
        assert_eq!(buf[5], 0xFE);
        assert_eq!(byte_len, 6);
    }

    #[test]
    fn writer_cross_boundary_odd_width() {
        let mut buf = [0u8; 8];
        let (byte_len, _) = write_and_flush(&mut buf, &[(0x1234567, 28), (0x89A, 12)]);

        // Total 40 bits = 5 bytes
        // Bits: 0001_0010_0011_0100_0101_0110_0111_1000_1001_1010
        assert_eq!(buf[0], 0x12);
        assert_eq!(buf[1], 0x34);
        assert_eq!(buf[2], 0x56);
        assert_eq!(buf[3], 0x78);
        assert_eq!(buf[4], 0x9A);
        assert_eq!(byte_len, 5);
    }

    #[test]
    fn writer_byte_length_partial() {
        let mut buf = [0u8; 4];
        let mut writer = BitStreamWriter::new(&mut buf);

        writer.write_bits(0b101, 3);
        assert_eq!(writer.byte_length(), 1); // 3 bits rounds up to 1 byte
        assert_eq!(writer.bits_written(), 3);
    }

    // ========================
    // Round-trip tests
    // ========================

    #[test]
    fn roundtrip_single_byte() {
        let mut buf = [0u8; 4];
        write_and_flush(&mut buf, &[(0xA5, 8)]);

        let mut reader = BitStreamReader::new(&buf[..1]);
        assert_eq!(reader.read_bits(8), 0xA5);
    }

    #[test]
    fn roundtrip_multiple_values() {
        let mut buf = [0u8; 16];
        let (byte_len, total_bits) =
            write_and_flush(&mut buf, &[(0b110, 3), (0xFF, 8), (0b10101, 5), (0xCAFE, 16)]);

        let mut reader = BitStreamReader::new(&buf[..byte_len]);
        assert_eq!(reader.read_bits(3), 0b110);
        assert_eq!(reader.read_bits(8), 0xFF);
        assert_eq!(reader.read_bits(5), 0b10101);
        assert_eq!(reader.read_bits(16), 0xCAFE);
        assert_eq!(reader.bits_consumed(), total_bits);
    }

    #[test]
    fn roundtrip_across_boundary() {
        let mut buf = [0u8; 16];
        let (byte_len, _) =
            write_and_flush(&mut buf, &[(0x1234, 16), (0x5678, 16), (0x9ABC, 16), (0xDEF0, 16)]);

        let mut reader = BitStreamReader::new(&buf[..byte_len]);
        assert_eq!(reader.read_bits(16), 0x1234);
        assert_eq!(reader.read_bits(16), 0x5678);
        assert_eq!(reader.read_bits(16), 0x9ABC);
        assert_eq!(reader.read_bits(16), 0xDEF0);
    }

    #[test]
    fn roundtrip_many_small_values() {
        let mut buf = [0u8; 16];
        // Write 20 x 3-bit values (60 bits total, crossing boundary)
        let values: Vec<(u32, u32)> = (0..20).map(|i| (i % 8, 3)).collect();
        let (byte_len, _) = write_and_flush(&mut buf, &values);

        let mut reader = BitStreamReader::new(&buf[..byte_len]);
        for i in 0..20u32 {
            assert_eq!(reader.read_bits(3), i % 8);
        }
    }
}
