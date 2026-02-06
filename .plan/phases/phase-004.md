# Phase 4: MPPC Compression

**Status**: ⏳ Pending  
**Phase Number**: 4 of 11  
**Estimated Duration**: 1 day

## Overview

Port MPPC compression from FreeRDP. The compressor uses LZ77 with a 3-byte hash for match finding, encoding literals and copy-offset/length pairs into a bitstream.

## Dependencies

Requires Phase 2 (bitstream writer) and Phase 3 (MPPC context, constants).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-011 | Implement mppc_compress | TASK-005, TASK-008 | High | ⏳ Pending |
| TASK-012 | Port MPPC compression tests | TASK-011 | Medium | ⏳ Pending |
| TASK-013 | MPPC round-trip validation | TASK-009, TASK-011 | Low | ⏳ Pending |

## Detailed Task Breakdown

### TASK-011: Implement mppc_compress

**Algorithm (from FreeRDP mppc.c lines 435-782)**:

1. Check if history has room for source data. If not, set PACKET_FLUSHED and reset.
2. Copy source data into history buffer at current offset.
3. Initialize bitstream writer on output buffer.
4. For each position in the newly added data:
   a. Compute 3-byte hash: `MPPC_MATCH_INDEX(sym1, sym2, sym3)`
   b. Look up `MatchBuffer[hash_index]` for previous occurrence
   c. If match found and within valid range:
      - Extend match to find longest match
      - Encode CopyOffset (varies by RDP4/RDP5)
      - Encode LengthOfMatch
   d. If no match or match too short:
      - Encode literal byte
   e. Update `MatchBuffer[hash_index]` with current position
5. Flush bitstream
6. If compressed size >= source size, fall back to PACKET_FLUSHED (send uncompressed)

**CopyOffset encoding (RDP5)**:
- 0-63: `11111` + 6 bits (11 total)
- 64-319: `11110` + 8 bits (13 total)
- 320-2367: `1110` + 11 bits (15 total)
- 2368+: `110` + 16 bits (19 total)

**CopyOffset encoding (RDP4)**:
- 0-63: `1111` + 6 bits (10 total)
- 64-319: `1110` + 8 bits (12 total)
- 320-8191: `110` + 13 bits (16 total)

**LengthOfMatch encoding**:
- 3: `0`
- 4-7: `10` + 2 bits
- 8-15: `110` + 3 bits
- 16-31: `1110` + 4 bits
- 32-63: `11110` + 5 bits
- 64-127: `111110` + 6 bits
- 128-255: `1111110` + 7 bits
- 256-511: `11111110` + 8 bits
- 512-1023: `111111110` + 9 bits
- 1024-2047: `1111111110` + 10 bits
- 2048-4095: `11111111110` + 11 bits
- 4096-8191: `111111111110` + 12 bits
- 8192-16383: `1111111111110` + 13 bits
- 16384-32767: `11111111111110` + 14 bits
- 32768-65535: `111111111111110` + 15 bits

---

### TASK-012: Port MPPC compression tests

**Test vectors**: Compress known input, compare output byte-for-byte with FreeRDP's compressed output. This validates that the Rust implementation produces identical bitstreams.

---

### TASK-013: MPPC round-trip validation

Compress with Rust, decompress with Rust. Verify original data recovered.

## Success Criteria

- [ ] All 4 compression tests pass with byte-exact output
- [ ] Round-trip validation passes for various input sizes
