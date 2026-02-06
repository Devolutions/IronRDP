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
    pub(crate) fn shift(&mut self, nbits: u32) {
        if nbits == 0 {
            return;
        }

        debug_assert!(nbits < 32, "use shift32() for shifting 32 bits");

        self.accumulator <<= nbits;
        self.bits_consumed += usize::from(nbits);
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
