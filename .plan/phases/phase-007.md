# Phase 7: NCRUSH Decompression

**Status**: ⏳ Pending  
**Phase Number**: 7 of 11  
**Estimated Duration**: 1 day

## Overview

Port NCRUSH (RDP 6.0) Huffman-based decompression. NCRUSH uses static Huffman tables for encoding literals, copy-offsets, and match lengths, with an LRU offset cache for recently used offsets.

## Dependencies

Requires Phase 1 (types) and Phase 2 (bitstream reader).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-021 | Port NCRUSH constants and Huffman tables | TASK-003 | Low | ⏳ Pending |
| TASK-022 | Implement NCRUSH context struct | TASK-021 | Low | ⏳ Pending |
| TASK-023 | Implement ncrush_decompress | TASK-004, TASK-022 | High | ⏳ Pending |
| TASK-024 | Port NCRUSH decompression test | TASK-023 | Medium | ⏳ Pending |

## Detailed Task Breakdown

### TASK-021: Port NCRUSH constants and Huffman tables

**Tables to port (from ncrush.c)**:
- `HuffTableLEC[8192]` (u16) - Lookup table for Literal/EndOfStream/CopyOffset Huffman decoding
- `HuffTableLOM[512]` (u16) - Lookup table for LengthOfMatch Huffman decoding
- `HuffTableMask[39]` (u32) - Bit masks for Huffman decoding (1, 3, 7, 15, ...)
- `HuffLengthLEC[294]` (u8) - Code lengths for LEC symbols
- `HuffCodeLEC[588]` (u16) - Huffman codes for LEC symbols
- `HuffLengthLOM[32]` (u8) - Code lengths for LOM symbols
- `HuffCodeLOM[32]` (u16) - Huffman codes for LOM symbols
- `CopyOffsetBitsLUT[32]` (u8) - Extra bits for copy offset encoding
- `CopyOffsetBaseLUT[32]` (u32) - Base values for copy offset ranges
- `LOMBitsLUT[30]` (u8) - Extra bits for length encoding
- `LOMBaseLUT[30]` (u16) - Base values for length ranges

### TASK-023: Implement ncrush_decompress

**Algorithm**:
1. Handle flags: PACKET_FLUSHED → reset; PACKET_AT_FRONT → reset history pointer
2. If not PACKET_COMPRESSED, copy raw to history
3. Attach bitstream reader
4. Decode loop:
   a. Read bits, lookup in `HuffTableLEC`:
      - Symbols 0-255: literal byte → write to history
      - Symbol 256: EndOfStream → stop
      - Symbols 257-288: CopyOffset index
        - Read extra bits from `CopyOffsetBitsLUT[index]`
        - Add to `CopyOffsetBaseLUT[index]` to get actual offset
      - Symbols 289-292: OffsetCache reference (use cached offset)
   b. After getting offset, decode LengthOfMatch from `HuffTableLOM`:
      - Read extra bits from `LOMBitsLUT[index]`
      - Add to `LOMBaseLUT[index]` to get actual length
   c. Copy from history at (current - offset) for length bytes
   d. Update offset cache (LRU rotation)

**Offset Cache**: Array of 4 most recently used offsets. When a cached offset is used, it's moved to front (LRU). When a new offset is used, it pushes others back.

### TASK-024: Port NCRUSH decompression test

**Test vectors**:
```
TEST_BELLS_DATA = "for.whom.the.bell.tolls,.the.bell.tolls.for.thee!"

TEST_BELLS_NCRUSH (44 bytes):
  0xfb, 0x1d, 0x7e, 0xe4, 0xda, 0xc7, 0x1d, 0x70,
  0xf8, 0xa1, 0x6b, 0x1f, 0x7d, 0xc0, 0xbe, 0x6b,
  0xef, 0xb5, 0xef, 0x21, 0x87, 0xd0, 0xc5, 0xe1,
  0x85, 0x71, 0xd4, 0x10, 0x16, 0xe7, 0xda, 0xfb,
  0x1d, 0x7e, 0xe4, 0xda, 0x47, 0x1f, 0xb0, 0xef,
  0xbe, 0xbd, 0xff, 0x2f
```

Flags: `PACKET_COMPRESSED | 2` (type 2 = NCRUSH)

## Success Criteria

- [ ] NCRUSH decompression test passes
- [ ] Huffman decoding handles all symbol types
- [ ] Offset cache works correctly
