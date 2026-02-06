# Phase 6: XCRUSH Compression & Tests

**Status**: ⏳ Pending  
**Phase Number**: 6 of 11  
**Estimated Duration**: 1 day

## Overview

Port XCRUSH compression and validate with FreeRDP test vectors. XCRUSH compression is two-level: Level 1 uses chunk-based matching with a rolling hash, Level 2 uses MPPC on the Level 1 output.

## Dependencies

Requires Phase 4 (MPPC compression for Level 2) and Phase 5 (XCRUSH types).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-017 | Implement XCRUSH chunk computation | TASK-014 | Medium | ⏳ Pending |
| TASK-018 | Implement XCRUSH match finding/optimization | TASK-017 | High | ⏳ Pending |
| TASK-019 | Implement XCRUSH full compression | TASK-011, TASK-018 | High | ⏳ Pending |
| TASK-020 | Port XCRUSH tests | TASK-016, TASK-019 | Medium | ⏳ Pending |

## Detailed Task Breakdown

### TASK-017: XCRUSH chunk computation

**Rolling hash algorithm** (from xcrush.c `xcrush_compute_chunks`):
1. Initialize accumulator = 0
2. For each byte in data (starting from byte 32 or later):
   - Subtract outgoing byte (32 positions back) from accumulator
   - Add incoming byte
   - Rotate accumulator left by 1
3. When `accumulator & 0x7F == 0`, mark a chunk boundary
4. Record chunk signature (hash seed) and size

### TASK-018: Match finding and optimization

**Match finding** (`xcrush_find_all_matches`):
1. For each signature in current data, look up hash in Chunks table
2. If match found, extend bidirectionally
3. Minimum match length: 11 bytes
4. Record all matches

**Match optimization** (`xcrush_optimize_matches`):
1. Sort matches by output position
2. Remove overlapping matches
3. Merge adjacent matches where possible

### TASK-019: Full XCRUSH compression

1. Compute chunks and signatures
2. Find matches against history
3. Optimize matches
4. Encode Level 1: header + match details + literals
5. If L1 output > 50 bytes, apply Level 2 (MPPC)
6. Update history buffer and chunk tables

### TASK-020: Port XCRUSH tests

**Test vectors**:
```
TEST_BELLS_DATA = "for.whom.the.bell.tolls,.the.bell.tolls.for.thee!"

TEST_BELLS_DATA_XCRUSH (49 bytes):
  0x66, 0x6f, 0x72, 0x2e, 0x77, 0x68, 0x6f, 0x6d, 0x2e, 0x74, 0x68, 0x65, 0x2e, 0x62,
  0x65, 0x6c, 0x6c, 0x2e, 0x74, 0x6f, 0x6c, 0x6c, 0x73, 0x2c, 0x2e, 0x74, 0x68, 0x65,
  0x2e, 0x62, 0x65, 0x6c, 0x6c, 0x2e, 0x74, 0x6f, 0x6c, 0x6c, 0x73, 0x2e, 0x66, 0x6f,
  0x72, 0x2e, 0x74, 0x68, 0x65, 0x65, 0x21

TEST_ISLAND_DATA_XCRUSH: compressed island data (~237 bytes)
```

## Success Criteria

- [ ] Both XCRUSH compression tests pass with byte-exact output
- [ ] Level 1 + Level 2 integration works correctly
