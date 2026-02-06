# Phase 3: MPPC Decompression

**Status**: ⏳ Pending  
**Phase Number**: 3 of 11  
**Estimated Duration**: 1 day

## Overview

Port MPPC decompression from FreeRDP supporting both RDP4 (8K history) and RDP5 (64K history) modes. This is the most fundamental algorithm and is also used internally by XCRUSH.

## Dependencies

Requires Phase 1 (types) and Phase 2 (bitstream reader).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-007 | Port MPPC constants and lookup tables | TASK-003 | Low | ⏳ Pending |
| TASK-008 | Implement MPPC context struct | TASK-007 | Low | ⏳ Pending |
| TASK-009 | Implement mppc_decompress | TASK-004, TASK-008 | High | ⏳ Pending |
| TASK-010 | Port MPPC decompression tests | TASK-009 | Medium | ⏳ Pending |

## Detailed Task Breakdown

### TASK-007: Port MPPC constants and lookup tables

**MPPC_MATCH_TABLE**: 256-entry u32 lookup table used for 3-byte hash computation.

**MPPC_MATCH_INDEX macro** (C):
```c
#define MPPC_MATCH_INDEX(_sym1, _sym2, _sym3) \
    ((((MPPC_MATCH_TABLE[_sym3] << 16) + (MPPC_MATCH_TABLE[_sym2] << 8) + \
       MPPC_MATCH_TABLE[_sym1]) & 0x07FFF000) >> 12)
```

Rust equivalent:
```rust
fn mppc_match_index(sym1: u8, sym2: u8, sym3: u8) -> usize {
    let val = (MPPC_MATCH_TABLE[sym3 as usize] << 16)
        .wrapping_add(MPPC_MATCH_TABLE[sym2 as usize] << 8)
        .wrapping_add(MPPC_MATCH_TABLE[sym1 as usize]);
    ((val & 0x07FFF000) >> 12) as usize
}
```

---

### TASK-009: Implement mppc_decompress

**Algorithm (from FreeRDP mppc.c lines 88-433)**:

1. Handle flags: PACKET_FLUSHED → reset context; PACKET_AT_FRONT → reset history pointer
2. If not PACKET_COMPRESSED, copy raw data to history buffer and return
3. Attach bitstream reader to source data
4. Main decode loop (while bits remaining >= 8):
   a. Read 1 bit: if 0 → **literal < 0x80** (read 7 more bits)
   b. If 1, read another bit: if 0 → **literal >= 0x80** (read 7 more bits, OR 0x80)
   c. If 11 → **copy offset + length**:
      - Decode CopyOffset (varies by RDP4/RDP5):
        - **RDP5**: Peek bits to determine range:
          - `11111` prefix → 0-63 (6 bits after prefix, 11 total)
          - `11110` prefix → 64-319 (8 bits after prefix, 13 total)
          - `1110` prefix → 320-2367 (11 bits after prefix, 15 total)
          - Otherwise → 2368+ (16 bits after `110` prefix, 19 total)
        - **RDP4**: Similar but with different bit widths
      - Decode LengthOfMatch: unary-coded prefix determines range
      - Copy from history buffer at (current_offset - copy_offset) with wrapping

**History wrapping**: RDP4 uses mask 0x1FFF (8K), RDP5 uses 0xFFFF (64K).

---

### TASK-010: Port MPPC decompression tests

**Test vectors from FreeRDP**:

```
TEST_MPPC_BELLS = "for.whom.the.bell.tolls,.the.bell.tolls.for.thee!" (47 bytes)

TEST_MPPC_BELLS_RDP4 (33 bytes, compressed):
  0x66, 0x6f, 0x72, 0x2e, 0x77, 0x68, 0x6f, 0x6d,
  0x2e, 0x74, 0x68, 0x65, 0x2e, 0x62, 0x65, 0x6c,
  0x6c, 0x2e, 0x74, 0x6f, 0x6c, 0x6c, 0x73, 0x2c,
  0xf4, 0x37, 0x2e, 0x66, 0xfa, 0x1f, 0x19, 0x94, 0x84

TEST_MPPC_BELLS_RDP5 (34 bytes, compressed):
  0x66, 0x6f, 0x72, 0x2e, 0x77, 0x68, 0x6f, 0x6d,
  0x2e, 0x74, 0x68, 0x65, 0x2e, 0x62, 0x65, 0x6c,
  0x6c, 0x2e, 0x74, 0x6f, 0x6c, 0x6c, 0x73, 0x2c,
  0xfa, 0x1b, 0x97, 0x33, 0x7e, 0x87, 0xe3, 0x32, 0x90, 0x80
```

Tests:
1. Decompress RDP4 bells → verify output matches "for.whom.the.bell..."
2. Decompress RDP5 bells → verify output matches
3. Decompress RDP5 buffer (598-byte binary data) → verify output matches

Flags for decompression:
- RDP4: `PACKET_AT_FRONT | PACKET_COMPRESSED | 0` (type 0)
- RDP5: `PACKET_AT_FRONT | PACKET_COMPRESSED | 1` (type 1)

## Success Criteria

- [ ] All 3 decompression tests pass
- [ ] Both RDP4 and RDP5 modes produce correct output
- [ ] Output matches FreeRDP byte-for-byte
