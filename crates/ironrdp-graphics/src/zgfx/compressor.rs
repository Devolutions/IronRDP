//! ZGFX (RDP8) LZ77 compression.
//!
//! Implements the compression side of the ZGFX codec defined in
//! [\[MS-RDPEGFX\] 2.2.1.1.1]. Uses a hash table mapping 3-byte prefixes to
//! history positions for O(1) match candidate lookup against the 2.5 MB
//! sliding window.
//!
//! [\[MS-RDPEGFX\] 2.2.1.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/

use std::{collections::HashMap, io::Write as _};

use bitvec::prelude::*;

use super::{FixedCircularBuffer, HISTORY_SIZE, TOKEN_TABLE, ZgfxError};
const MIN_MATCH_LENGTH: usize = 3;
const MAX_MATCH_LENGTH: usize = 65535;
/// Maximum back-reference distance (last token in MS-RDPEGFX table)
const MAX_MATCH_DISTANCE: usize = 2_097_152;

/// Cap candidates per lookup to bound worst-case search time
const MAX_CANDIDATES: usize = 16;

/// Cap stored positions per prefix to bound memory
const MAX_POSITIONS_PER_PREFIX: usize = 32;

/// Trigger hash table compaction when entry count exceeds this
const MAX_HASH_TABLE_ENTRIES: usize = 50_000;

/// Compaction evicts down to this low watermark, so it runs at most once per
/// `MAX_HASH_TABLE_ENTRIES - COMPACT_TARGET_ENTRIES` inserted prefixes
const COMPACT_TARGET_ENTRIES: usize = MAX_HASH_TABLE_ENTRIES / 2;

/// ZGFX compressor maintaining a 2.5 MB history buffer and prefix hash table.
pub struct Compressor {
    history: History,
    /// 3-byte prefix → absolute stream positions, for O(1) match candidate lookup.
    /// Positions are never rebased; candidates that have fallen out of reach are
    /// skipped at lookup time.
    match_table: HashMap<[u8; 3], Vec<u64>>,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            history: History::new(),
            match_table: HashMap::new(),
        }
    }

    /// Compress `input` into raw ZGFX segment data (without segment headers).
    pub fn compress(&mut self, input: &[u8]) -> Result<Vec<u8>, ZgfxError> {
        let mut bit_writer = BitWriter::new();
        let mut pos = 0;

        while pos < input.len() {
            let best_match = self.find_best_match(input, pos);

            if let Some(m) = best_match {
                if m.length >= MIN_MATCH_LENGTH {
                    Self::encode_match(&mut bit_writer, m.distance, m.length)?;
                    self.add_to_history(&input[pos..pos + m.length]);
                    pos += m.length;
                    continue;
                }
            }

            let byte = input[pos];
            Self::encode_literal(&mut bit_writer, byte)?;
            self.add_to_history(&[byte]);
            pos += 1;
        }

        Ok(bit_writer.finish())
    }

    /// Record `bytes` in the history without searching them for matches, for
    /// data the caller sends in an uncompressed segment instead of passing it
    /// to [`Compressor::compress`].
    ///
    /// MS-RDPEGFX 3.1.9.1.2 requires every output byte, including the bytes of
    /// segments sent uncompressed, to be recorded in the history; the receiver
    /// does so, and skipping them here would make later back-references point
    /// at the wrong bytes. The bytes are not bulk indexed, so the search that
    /// compression would spend on already entropy coded data is skipped. The
    /// last two recorded bytes can still start a match, because the next
    /// append indexes the positions that straddle the boundary.
    pub(crate) fn record_uncompressed(&mut self, bytes: &[u8]) {
        self.history.push(bytes);
    }

    /// Extend the sliding window; the ring overwrites the oldest bytes once full.
    fn add_to_history(&mut self, bytes: &[u8]) {
        let base_pos = self.history.total();
        self.history.push(bytes);

        // For large chunks (match replays), sample every 4th position to keep
        // the hash table manageable without sacrificing much compression ratio
        let step_size = if bytes.len() > 256 { 4 } else { 1 };

        for i in (0..bytes.len().saturating_sub(MIN_MATCH_LENGTH - 1)).step_by(step_size) {
            self.record_position([bytes[i], bytes[i + 1], bytes[i + 2]], base_pos + as_u64(i));
        }

        if self.match_table.len() > MAX_HASH_TABLE_ENTRIES {
            self.compact_hash_table();
        }

        // Index positions spanning the old/new data boundary so matches
        // that straddle the append point can be found
        for offset in [2u64, 1] {
            let Some(pos) = base_pos.checked_sub(offset) else {
                continue;
            };
            let Some(prefix) = self.history.prefix_at(pos) else {
                continue;
            };
            self.record_position(prefix, pos);
        }
    }

    /// Remember that `prefix` occurs at the absolute stream position `pos`, dropping the
    /// prefix's oldest position when it already has `MAX_POSITIONS_PER_PREFIX`.
    ///
    /// INVARIANT: no position is recorded twice. A position is recorded when its third byte
    /// arrives: the main loop covers the positions whose three bytes are all in the new chunk,
    /// and the boundary loop the two positions that straddle the append point, so no position
    /// is in both. Bytes added with `record_uncompressed` are not indexed, apart from the
    /// positions that straddle their end, which the next append indexes.
    fn record_position(&mut self, prefix: [u8; 3], pos: u64) {
        let positions = self.match_table.entry(prefix).or_default();
        debug_assert!(positions.last() != Some(&pos));
        if MAX_POSITIONS_PER_PREFIX <= positions.len() {
            // Evict oldest position to keep recent data preferred
            positions.remove(0);
        }
        positions.push(pos);
    }

    /// Halve stored positions per prefix, then evict whole prefixes down to
    /// `COMPACT_TARGET_ENTRIES` when the table is over the cap.
    ///
    /// INVARIANT: the table holds at most `MAX_HASH_TABLE_ENTRIES` prefixes on
    /// return. Incompressible input yields a near-unique prefix per byte, so
    /// trimming position lists alone never lowers the prefix count; without
    /// evicting whole prefixes the table would stay above the threshold and the
    /// caller would re-run compaction on every literal byte at O(table) cost.
    /// Evicting to a lower watermark amortizes that cost to O(1) per byte.
    fn compact_hash_table(&mut self) {
        for positions in self.match_table.values_mut() {
            if positions.len() > MAX_POSITIONS_PER_PREFIX / 2 {
                let keep_from = positions.len() - (MAX_POSITIONS_PER_PREFIX / 2);
                *positions = positions[keep_from..].to_vec();
            }
        }
        self.match_table.retain(|_, positions| !positions.is_empty());

        if self.match_table.len() <= COMPACT_TARGET_ENTRIES {
            return;
        }

        // Keep the most-recently-seen prefixes; older ones point further back
        // than a fresh match can reach, and distance is capped at
        // MAX_MATCH_DISTANCE regardless. Each history position belongs to a
        // single prefix, so these newest positions are distinct and the cutoff
        // retains exactly COMPACT_TARGET_ENTRIES entries.
        let mut newest: Vec<u64> = self
            .match_table
            .values()
            .map(|positions| positions.last().copied().unwrap_or(0))
            .collect();
        let cutoff_index = newest.len() - COMPACT_TARGET_ENTRIES;
        let cutoff = *newest.select_nth_unstable(cutoff_index).1;
        self.match_table
            .retain(|_, positions| positions.last().is_some_and(|&pos| pos >= cutoff));
    }

    /// Search hash table for the longest match at `input[pos..]`.
    fn find_best_match(&self, input: &[u8], pos: usize) -> Option<Match> {
        let remaining = input.len() - pos;
        if remaining < MIN_MATCH_LENGTH {
            return None;
        }

        let prefix = [input[pos], input[pos + 1], input[pos + 2]];
        let candidates = self.match_table.get(&prefix)?;

        let max_match_len = remaining.min(MAX_MATCH_LENGTH);
        let mut best_match: Option<Match> = None;
        let search_limit = as_u64(self.history.len().min(MAX_MATCH_DISTANCE));
        let total = self.history.total();

        // Most recent candidates first — better locality, often longer matches
        for &hist_pos in candidates.iter().rev().take(MAX_CANDIDATES) {
            // Positions are absolute, so anything older than the reachable
            // window is simply skipped rather than rebased on every append.
            let Some(distance) = total.checked_sub(hist_pos).filter(|d| *d <= search_limit) else {
                continue;
            };
            let distance = usize::try_from(distance).expect("distance is at most MAX_MATCH_DISTANCE");

            // Prefix already matched via hash table; extend from byte 3 onward.
            // INVARIANT: match_len < distance, so the match reads only bytes
            // already in the history.
            let mut match_len = MIN_MATCH_LENGTH;

            while match_len < max_match_len
                && match_len < distance
                && self.history.byte_back(distance - match_len) == input[pos + match_len]
            {
                match_len += 1;
            }

            if best_match.as_ref().is_none_or(|b| match_len > b.length) {
                best_match = Some(Match {
                    distance,
                    length: match_len,
                });
            }

            // Good enough — diminishing returns from longer searches
            if match_len >= 32 {
                break;
            }
        }

        best_match
    }

    /// Select the ZGFX token whose distance range covers `distance`.
    #[expect(
        clippy::as_conversions,
        reason = "distance_base is u32 from TOKEN_TABLE, always fits usize on 32+ bit targets"
    )]
    fn find_match_token(distance: usize) -> MatchToken {
        for token in TOKEN_TABLE.iter().skip(26) {
            if let super::TokenType::Match {
                distance_value_size,
                distance_base,
            } = token.ty
            {
                let max_distance = distance_base as usize + (1 << distance_value_size) - 1;
                if distance <= max_distance {
                    return MatchToken {
                        prefix: token.prefix,
                        distance_value_size,
                        distance_base: distance_base as usize,
                    };
                }
            }
        }

        // Fallback: last token covers the full 2 MB history range
        if let super::TokenType::Match {
            distance_value_size,
            distance_base,
        } = TOKEN_TABLE[39].ty
        {
            MatchToken {
                prefix: TOKEN_TABLE[39].prefix,
                distance_value_size,
                distance_base: distance_base as usize,
            }
        } else {
            unreachable!("TOKEN_TABLE[39] is always a Match variant");
        }
    }

    /// Return the index of the literal token for `byte`, if one exists.
    fn find_literal_token(byte: u8) -> Option<usize> {
        for (i, token) in TOKEN_TABLE.iter().enumerate().take(26).skip(1) {
            if let super::TokenType::Literal { literal_value } = token.ty {
                if literal_value == byte {
                    return Some(i);
                }
            }
        }
        None
    }

    fn encode_literal(writer: &mut BitWriter, byte: u8) -> Result<(), ZgfxError> {
        if let Some(token_idx) = Self::find_literal_token(byte) {
            writer.write_bits_from_slice(TOKEN_TABLE[token_idx].prefix);
        } else {
            // Null literal: "0" prefix + 8-bit value
            writer.write_bit(false);
            writer.write_bits(u32::from(byte), 8);
        }
        Ok(())
    }

    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "distance_value bounded by token table, value fits u32"
    )]
    fn encode_match(writer: &mut BitWriter, distance: usize, length: usize) -> Result<(), ZgfxError> {
        let match_token = Self::find_match_token(distance);

        writer.write_bits_from_slice(match_token.prefix);

        let distance_value = distance - match_token.distance_base;
        writer.write_bits(distance_value as u32, match_token.distance_value_size);

        Self::encode_match_length(writer, length)?;

        Ok(())
    }

    /// Encode match length using the variable-length scheme from the spec.
    ///
    /// Length 3 is a special case (single zero bit). All other lengths use
    /// unary-coded token size: `token_size` one-bits, a zero bit, then
    /// `token_size + 1` value bits, where `length = 2^(token_size+1) + value`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "length bounded by MAX_MATCH_LENGTH (u16), ilog2 result fits u32/usize"
    )]
    fn encode_match_length(writer: &mut BitWriter, length: usize) -> Result<(), ZgfxError> {
        if length == 3 {
            writer.write_bit(false);
        } else {
            let length_token_size = usize::try_from(length.ilog2()).expect("ilog2 of usize fits usize") - 1;
            let base = 1 << (length_token_size + 1);
            let value = length - base;

            for _ in 0..length_token_size {
                writer.write_bit(true);
            }
            writer.write_bit(false);

            writer.write_bits(value as u32, length_token_size + 1);
        }

        Ok(())
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).expect("usize fits in u64")
}

/// The last `HISTORY_SIZE` bytes of the stream, kept in a fixed ring so that
/// appending never moves existing bytes.
struct History {
    ring: FixedCircularBuffer,
    /// Bytes appended since the compressor was created; the absolute stream
    /// position of the next byte.
    total: u64,
}

impl History {
    fn new() -> Self {
        Self {
            ring: FixedCircularBuffer::new(HISTORY_SIZE),
            total: 0,
        }
    }

    /// Number of valid bytes in the ring, at most `HISTORY_SIZE`.
    fn len(&self) -> usize {
        usize::try_from(self.total.min(as_u64(HISTORY_SIZE))).expect("at most HISTORY_SIZE")
    }

    fn total(&self) -> u64 {
        self.total
    }

    fn push(&mut self, bytes: &[u8]) {
        // The ring keeps only the last HISTORY_SIZE bytes of an input larger than itself.
        self.ring.write_all(bytes).expect("writing to the ring cannot fail");
        self.total += as_u64(bytes.len());
    }

    /// The byte `distance` positions before the end of the stream.
    ///
    /// INVARIANT: `0 < distance <= self.len()`. Both callers establish it
    /// themselves, which is why the check is a `debug_assert!` and not a branch
    /// on every extended byte: `find_best_match` only keeps candidates within
    /// the window and extends while `match_len < distance`, and `prefix_at`
    /// checks the window before it reads. Tests run in debug builds, where a
    /// violation panics, and the round-trip tests would catch a wrong byte.
    fn byte_back(&self, distance: usize) -> u8 {
        debug_assert!(0 < distance && distance <= self.len());
        self.ring.byte_back(distance)
    }

    /// The three bytes starting at absolute position `pos`, or `None` when they
    /// are not all in the ring: fewer than three bytes follow `pos`, or `pos`
    /// has fallen out of the window.
    fn prefix_at(&self, pos: u64) -> Option<[u8; 3]> {
        let distance = usize::try_from(self.total.checked_sub(pos)?).ok()?;
        if !(MIN_MATCH_LENGTH..=self.len()).contains(&distance) {
            return None;
        }
        Some([
            self.byte_back(distance),
            self.byte_back(distance - 1),
            self.byte_back(distance - 2),
        ])
    }
}

#[derive(Debug, Clone, Copy)]
struct Match {
    distance: usize,
    length: usize,
}

struct MatchToken {
    prefix: &'static BitSlice<u8, Msb0>,
    distance_value_size: usize,
    distance_base: usize,
}

/// MSB-first bit writer for ZGFX token encoding.
struct BitWriter {
    bytes: Vec<u8>,
    current_byte: u8,
    bits_in_current: usize,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current_byte: 0,
            bits_in_current: 0,
        }
    }

    fn write_bit(&mut self, bit: bool) {
        if bit {
            self.current_byte |= 1 << (7 - self.bits_in_current);
        }
        self.bits_in_current += 1;

        if self.bits_in_current == 8 {
            self.bytes.push(self.current_byte);
            self.current_byte = 0;
            self.bits_in_current = 0;
        }
    }

    fn write_bits(&mut self, value: u32, num_bits: usize) {
        for i in (0..num_bits).rev() {
            self.write_bit((value >> i) & 1 == 1);
        }
    }

    fn write_bits_from_slice(&mut self, bits: &BitSlice<u8, Msb0>) {
        for bit in bits {
            self.write_bit(*bit);
        }
    }

    /// Finalize: append partial byte (if any) and the ZGFX-required
    /// trailing byte indicating unused bits in the final data byte.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "unused_bits is 0..=7, always fits u8"
    )]
    fn finish(mut self) -> Vec<u8> {
        let unused_bits = if self.bits_in_current == 0 {
            0
        } else {
            8 - self.bits_in_current
        };

        if self.bits_in_current > 0 {
            self.bytes.push(self.current_byte);
        }

        self.bytes.push(unused_bits as u8);

        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_empty() {
        let mut compressor = Compressor::new();
        let compressed = compressor.compress(&[]).unwrap();

        assert_eq!(compressed.len(), 1);
        assert_eq!(compressed[0], 0);
    }

    #[test]
    fn compress_single_byte() {
        let mut compressor = Compressor::new();
        let compressed = compressor.compress(&[0x42]).unwrap();

        // Null literal: "0" + 8 bits + padding = 9 bits = 2 bytes + padding byte
        assert!(compressed.len() >= 2);
    }

    #[test]
    fn compress_round_trip() {
        use super::super::Decompressor;

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();

        let data = b"Hello, ZGFX compression! This is a test.";
        let compressed = compressor.compress(data).unwrap();

        let mut output = Vec::new();
        decompressor.decompress_segment(&compressed, &mut output).unwrap();

        assert_eq!(&output, data);
    }

    #[test]
    fn compress_repetitive_data() {
        use super::super::Decompressor;

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();

        let data = b"AAAAAAAAAABBBBBBBBBBCCCCCCCCCC";
        let compressed = compressor.compress(data).unwrap();

        let mut output = Vec::new();
        decompressor.decompress_segment(&compressed, &mut output).unwrap();

        assert_eq!(&output, data);
    }

    #[test]
    fn compress_large_patterned_data() {
        use super::super::Decompressor;

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();

        let mut data = Vec::new();
        for i in 0..1000 {
            data.extend_from_slice(b"Pattern");
            data.push(u8::try_from(i % 256).unwrap());
        }

        let compressed = compressor.compress(&data).unwrap();

        let mut output = Vec::new();
        decompressor.decompress_segment(&compressed, &mut output).unwrap();

        assert_eq!(output, data);
    }

    #[test]
    fn compress_high_entropy_round_trips_and_bounds_table() {
        use super::super::Decompressor;

        // A near-unique 3-byte prefix per byte is the compactor's worst case:
        // trimming per-prefix position lists frees nothing, so without evicting
        // whole prefixes the table grows past MAX_HASH_TABLE_ENTRIES and
        // compaction re-runs on every literal byte at O(table) cost. A
        // deterministic LCG makes the stream exceed the entry cap reproducibly.
        let mut state: u32 = 0x1234_5678;
        let data: Vec<u8> = core::iter::repeat_with(|| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            u8::try_from(state >> 24).unwrap()
        })
        .take(100_000)
        .collect();

        let mut compressor = Compressor::new();
        let compressed = compressor.compress(&data).unwrap();

        let mut decompressor = Decompressor::new();
        let mut output = Vec::new();
        decompressor.decompress_segment(&compressed, &mut output).unwrap();
        assert_eq!(output, data);

        assert!(
            compressor.match_table.len() <= MAX_HASH_TABLE_ENTRIES,
            "hash table must stay bounded, got {} entries",
            compressor.match_table.len()
        );
    }

    #[test]
    fn compress_round_trips_across_history_wrap() {
        use super::super::Decompressor;

        // More than twice HISTORY_SIZE through one compressor, in segments that
        // repeat earlier content, so matches reach back across the ring wrap
        // point and the decompressor's history has to agree byte for byte.
        let mut state: u32 = 0x0BAD_F00D;
        let mut next_byte = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            u8::try_from(state >> 24).unwrap()
        };
        let blocks: Vec<Vec<u8>> =
            core::iter::repeat_with(|| core::iter::repeat_with(&mut next_byte).take(4096).collect())
                .take(64)
                .collect();

        let mut compressor = Compressor::new();
        let mut decompressor = Decompressor::new();
        let mut total = 0;
        let mut segment_index = 0usize;
        while total < 2 * HISTORY_SIZE + 100_000 {
            let mut segment = Vec::with_capacity(60_000);
            for k in 0..14 {
                let block = &blocks[(segment_index * 7 + k * 5) % blocks.len()];
                segment.extend_from_slice(block);
                segment.push(next_byte());
            }
            segment_index += 1;

            let compressed = compressor.compress(&segment).unwrap();
            let mut output = Vec::new();
            decompressor.decompress_segment(&compressed, &mut output).unwrap();
            assert_eq!(output, segment, "segment {segment_index} after {total} bytes");
            total += segment.len();
        }
        assert!(compressor.history.total() > u64::try_from(2 * HISTORY_SIZE).unwrap());
    }

    #[test]
    fn a_prefix_keeps_only_its_newest_positions() {
        let mut compressor = Compressor::new();
        let recorded = MAX_POSITIONS_PER_PREFIX + 8;
        for pos in 0..recorded {
            compressor.record_position(*b"abc", as_u64(pos));
        }

        let expected: Vec<u64> = (8..recorded).map(as_u64).collect();
        assert_eq!(compressor.match_table[b"abc"], expected);
    }

    #[test]
    fn positions_are_recorded_inside_a_chunk_and_across_the_append_point() {
        let mut compressor = Compressor::new();
        compressor.add_to_history(b"abcd");
        compressor.add_to_history(b"ef");

        // The first chunk holds "abc" at 0 and "bcd" at 1. The second chunk completes "cde" at 2 and
        // "def" at 3, which start before the append point.
        let table = &compressor.match_table;
        assert_eq!(table[b"abc"], [0]);
        assert_eq!(table[b"bcd"], [1]);
        assert_eq!(table[b"cde"], [2]);
        assert_eq!(table[b"def"], [3]);
        assert_eq!(table.len(), 4);
    }

    #[test]
    fn history_ring_keeps_the_last_bytes_in_order() {
        let mut history = History::new();
        let chunk: Vec<u8> = (0..=255u8).cycle().take(HISTORY_SIZE - 10).collect();
        history.push(&chunk);
        history.push(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        assert_eq!(history.len(), HISTORY_SIZE);
        assert_eq!(history.byte_back(1), 15);
        assert_eq!(history.byte_back(15), 1);
        assert_eq!(history.byte_back(16), *chunk.last().unwrap());
        assert_eq!(history.prefix_at(history.total() - 3), Some([13, 14, 15]));
        assert_eq!(
            history.prefix_at(history.total() - 2),
            None,
            "fewer than three bytes follow the position"
        );
        let oldest = history.total() - as_u64(HISTORY_SIZE);
        assert_eq!(
            history.prefix_at(oldest),
            Some([
                history.byte_back(HISTORY_SIZE),
                history.byte_back(HISTORY_SIZE - 1),
                history.byte_back(HISTORY_SIZE - 2),
            ]),
            "the oldest position still in the ring is reachable"
        );
        assert_eq!(
            history.prefix_at(oldest - 1),
            None,
            "one position older has left the ring"
        );

        // A push larger than the ring keeps only its tail.
        let big: Vec<u8> = (0..HISTORY_SIZE + 7).map(|i| u8::try_from(i % 251).unwrap()).collect();
        history.push(&big);
        assert_eq!(history.len(), HISTORY_SIZE);
        assert_eq!(history.byte_back(1), *big.last().unwrap());
        assert_eq!(history.byte_back(HISTORY_SIZE), big[7]);
    }

    #[test]
    fn bit_writer_basic() {
        let mut writer = BitWriter::new();

        writer.write_bit(true);
        writer.write_bit(false);
        writer.write_bit(true);
        writer.write_bits(0b101, 3);

        let result = writer.finish();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 0b10110100);
        assert_eq!(result[1], 2);
    }

    #[test]
    fn encode_literal_with_token() {
        let mut writer = BitWriter::new();

        Compressor::encode_literal(&mut writer, 0x00).unwrap();

        let result = writer.finish();
        assert!(!result.is_empty());
    }

    #[test]
    fn encode_literal_null_prefix() {
        let mut writer = BitWriter::new();

        Compressor::encode_literal(&mut writer, 0x42).unwrap();

        let result = writer.finish();
        assert_eq!(result.len(), 3);
        assert_eq!(result[2], 7);
    }
}
