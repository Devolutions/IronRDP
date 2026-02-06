# Phase 8: NCRUSH Compression & Tests

**Status**: ⏳ Pending  
**Phase Number**: 8 of 11  
**Estimated Duration**: 1 day

## Overview

Port NCRUSH compression and validate with FreeRDP test vectors. NCRUSH compression uses Huffman coding on LZ77 matches with an offset cache.

## Dependencies

Requires Phase 7 (NCRUSH context, constants) and Phase 2 (bitstream writer).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-025 | Implement NCRUSH hash-chain match finding | TASK-022 | Medium | ⏳ Pending |
| TASK-026 | Implement NCRUSH Huffman encoding | TASK-005, TASK-025 | High | ⏳ Pending |
| TASK-027 | Implement ncrush_compress | TASK-026 | High | ⏳ Pending |
| TASK-028 | Port NCRUSH compression test | TASK-027 | Medium | ⏳ Pending |
| TASK-029 | NCRUSH round-trip validation | TASK-023, TASK-027 | Low | ⏳ Pending |

## Detailed Task Breakdown

### TASK-025: Hash-chain match finding

**Algorithm**:
1. Hash 2 bytes at current position: `hash = (byte[i] | (byte[i+1] << 8))`
2. Look up `HashTable[hash]` for first chain entry
3. Follow `MatchTable` chain (up to 4 candidates)
4. For each candidate, compare bytes to find longest match
5. Select best match (longest length)
6. Update hash table and match table chain

### TASK-026: Huffman encoding

**Encoding functions**:
- Literal byte: write `HuffCodeLEC[byte]` with `HuffLengthLEC[byte]` bits
- EndOfStream: write `HuffCodeLEC[256]` with `HuffLengthLEC[256]` bits
- CopyOffset: find index in CopyOffsetBaseLUT, write `HuffCodeLEC[257+index]` + extra bits
- OffsetCache ref: write `HuffCodeLEC[289+cache_index]`
- LengthOfMatch: find index in LOMBaseLUT, write `HuffCodeLOM[index]` + extra bits

### TASK-027: Full ncrush_compress

1. Handle PACKET_FLUSHED if history overflow
2. Copy source to history
3. For each position:
   a. Find best match via hash chain
   b. Check offset cache for better encoding
   c. Encode literal or match
   d. Update offset cache (LRU)
   e. Update hash chain
4. Write EndOfStream marker
5. Handle window sliding at 32KB

## Success Criteria

- [ ] NCRUSH compression test passes with byte-exact output
- [ ] Round-trip validation passes
