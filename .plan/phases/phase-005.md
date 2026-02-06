# Phase 5: XCRUSH Decompression

**Status**: ⏳ Pending  
**Phase Number**: 5 of 11  
**Estimated Duration**: 1 day

## Overview

Port XCRUSH (RDP 6.1) two-level decompression. XCRUSH is a two-layer compression scheme: Level 1 is chunk-based matching with a large (2MB) history buffer, and Level 2 is MPPC applied to the Level 1 output.

## Dependencies

Requires Phase 1 (types), Phase 3 (MPPC decompression for Level 2).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-014 | Port XCRUSH types and structures | TASK-003 | Low | ⏳ Pending |
| TASK-015 | Implement XCRUSH Level 1 decompression | TASK-014 | High | ⏳ Pending |
| TASK-016 | Implement XCRUSH full decompression | TASK-009, TASK-015 | Medium | ⏳ Pending |

## Detailed Task Breakdown

### TASK-014: Port XCRUSH types and structures

**Key types**:
```rust
struct XCrushMatchInfo {
    match_offset: u32,
    chunk_offset: u32,
    match_length: u32,
}

struct XCrushChunk {
    offset: u32,
    next: u32,
}

struct XCrushSignature {
    seed: u32,
    size: u32,
}

struct Rdp61MatchDetails {
    match_length: u16,
    match_output_offset: u16,
    match_history_offset: u32,
}

struct XCrushContext {
    compressor: bool,
    mppc: MppcContext,
    history_buffer: Vec<u8>,  // 2MB
    history_offset: u32,
    block_buffer: [u8; 16384],
    signatures: Vec<XCrushSignature>,  // up to 1000
    chunks: Vec<XCrushChunk>,          // 65534
    next_chunks: Vec<u16>,             // 65536
    original_matches: Vec<XCrushMatchInfo>,
    optimized_matches: Vec<XCrushMatchInfo>,
    // ...
}
```

### TASK-015: XCRUSH Level 1 decompression

**RDP61 compressed data format** (parsed during L1 decompression):
1. Byte 0: Level 1 compression flags
2. Byte 1: Level 2 compression flags
3. Bytes 2-3: Match count (u16 LE)
4. For each match: 8 bytes (MatchLength:u16 + MatchOutputOffset:u16 + MatchHistoryOffset:u32)
5. Remaining bytes: literal data

**Decompression algorithm**:
- Walk through matches in order
- Copy literal bytes from literal data up to match's output offset
- Copy match bytes from history buffer at match_history_offset
- After all matches, copy remaining literals
- Append output to history buffer

### TASK-016: XCRUSH full decompression

**Flag combinations**:
- `PACKET_COMPRESSED`: outer compression applied
- `L1_COMPRESSED`: Level 1 compression applied
- `L1_INNER_COMPRESSION`: Level 2 (MPPC) also applied
- `L1_NO_COMPRESSION`: passthrough

When L1_INNER_COMPRESSION is set, first decompress via MPPC, then apply L1 decompression.

## Success Criteria

- [ ] XCRUSH decompression handles all flag combinations
- [ ] Level 1 + Level 2 path works correctly
- [ ] History buffer management is correct
