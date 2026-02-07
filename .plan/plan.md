# Port FreeRDP Bulk Compression to IronRDP

**Status**: 🔵 Not Started  
**Created**: 2026-02-06  
**Estimated Duration**: ~5-7 days of focused work

## Overview

Port FreeRDP's bulk compressor support (MPPC, XCRUSH, NCRUSH) from C to Rust within the IronRDP project. This includes the core algorithms, the bulk coordinator that selects the right compressor based on RDP compression level, and comprehensive unit tests ported from FreeRDP's test suite. The implementation starts with MPPC (simplest), then XCRUSH (depends on MPPC), then NCRUSH (independent Huffman-based), and finishes with memory safety refactoring and WebAssembly compatibility.

## Project Strategy

- **Approach**: Incremental — implement and validate each algorithm before moving to the next
- **Total Tasks**: 37
- **Total Phases**: 11
- **Success Metric**: All ported unit tests pass, code is fully memory-safe, builds for wasm32-unknown-unknown
- **Risk Management**: Allow temporary `unsafe` during initial porting to reach working tests faster; refactor to safe Rust afterwards

## Architecture/Design Principles

- New crate: `ironrdp-bulk` in `crates/ironrdp-bulk/`
- `no_std` compatible with `alloc` feature (required for wasm)
- Follow IronRDP naming conventions and style (see STYLE.md)
- Bitstream reader/writer as a standalone module reusable across all three algorithms
- Each algorithm in its own submodule: `mppc`, `xcrush`, `ncrush`
- Bulk coordinator as the top-level public API
- Use `ironrdp-core` for error types and cursor utilities where applicable
- Lookup tables as `static` arrays (const where possible)

## Phases

### Phase 1: Project Setup & Crate Scaffold

**Goal**: Create the `ironrdp-bulk` crate with proper structure, dependencies, and workspace integration

**Duration Estimate**: 0.5 days

**Tasks**:
- [x] **TASK-001**: Create `crates/ironrdp-bulk/` crate with Cargo.toml
  - **Acceptance Criteria**: Crate compiles with `cargo check -p ironrdp-bulk`
  - **Dependencies**: None
  - **Risk**: Low
- [x] **TASK-002**: Set up module structure (lib.rs, submodules)
  - **Acceptance Criteria**: Module tree compiles: `lib.rs`, `mppc/`, `xcrush/`, `ncrush/`, `bulk.rs`
  - **Dependencies**: TASK-001
  - **Risk**: Low
- [x] **TASK-003**: Define shared types and error types
  - **Acceptance Criteria**: `CompressionType`, `CompressionFlags`, error enum defined and usable
  - **Dependencies**: TASK-002
  - **Risk**: Low

**Success Criteria**:
- Crate compiles successfully
- Module structure is in place
- Shared types are defined

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] `cargo check -p ironrdp-bulk` passes
- [ ] No clippy warnings

**Detailed Plan**: See `.plan/phases/phase-001.md`

---

### Phase 2: Bitstream Utilities

**Goal**: Port FreeRDP's `wBitStream` reader/writer to Rust for use by all three algorithms

**Duration Estimate**: 0.5 days

**Tasks**:
- [x] **TASK-004**: Implement `BitStreamReader` for decompression
  - **Acceptance Criteria**: Can read N bits from a byte buffer, handles prefetch/accumulator correctly
  - **Dependencies**: TASK-002
  - **Risk**: Medium (bit-level operations are error-prone)
- [x] **TASK-005**: Implement `BitStreamWriter` for compression
  - **Acceptance Criteria**: Can write N bits to a byte buffer, handles flush correctly
  - **Dependencies**: TASK-002
  - **Risk**: Medium
- [x] **TASK-006**: Unit tests for bitstream utilities
  - **Acceptance Criteria**: Tests cover reading/writing various bit widths, boundary conditions
  - **Dependencies**: TASK-004, TASK-005
  - **Risk**: Low

**Success Criteria**:
- Bitstream reader and writer work correctly
- All bitstream unit tests pass

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] No clippy warnings

**Detailed Plan**: See `.plan/phases/phase-002.md`

---

### Phase 3: MPPC Decompression

**Goal**: Port MPPC decompression from FreeRDP, supporting both RDP4 (8K) and RDP5 (64K) modes

**Duration Estimate**: 1 day

**Tasks**:
- [x] **TASK-007**: Port MPPC constants and lookup tables
  - **Acceptance Criteria**: `MPPC_MATCH_TABLE` and related constants defined
  - **Dependencies**: TASK-003
  - **Risk**: Low
- [x] **TASK-008**: Implement MPPC context struct
  - **Acceptance Criteria**: `MppcContext` with history buffer, match buffer, compression level
  - **Dependencies**: TASK-007
  - **Risk**: Low
- [x] **TASK-009**: Implement `mppc_decompress` function
  - **Acceptance Criteria**: Decompresses RDP4 and RDP5 MPPC data correctly
  - **Dependencies**: TASK-004, TASK-008
  - **Risk**: High (complex bit manipulation, two encoding variants)
- [x] **TASK-010**: Port MPPC decompression tests
  - **Acceptance Criteria**: `test_mppc_decompress_bells_rdp4`, `test_mppc_decompress_bells_rdp5`, `test_mppc_decompress_buffer_rdp5` all pass
  - **Dependencies**: TASK-009
  - **Risk**: Medium

**Success Criteria**:
- All 3 decompression tests pass
- Both RDP4 and RDP5 modes work

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] Decompression output matches FreeRDP byte-for-byte

**Detailed Plan**: See `.plan/phases/phase-003.md`

---

### Phase 4: MPPC Compression

**Goal**: Port MPPC compression, validate with round-trip and byte-exact tests

**Duration Estimate**: 1 day

**Tasks**:
- [x] **TASK-011**: Implement `mppc_compress` function
  - **Acceptance Criteria**: Compresses data using 3-byte hash matching, literal/copy-offset/length encoding
  - **Dependencies**: TASK-005, TASK-008
  - **Risk**: High (complex algorithm with hash table management)
- [x] **TASK-012**: Port MPPC compression tests
  - **Acceptance Criteria**: `test_mppc_compress_bells_rdp4`, `test_mppc_compress_bells_rdp5`, `test_mppc_compress_island_rdp5`, `test_mppc_compress_buffer_rdp5` all pass
  - **Dependencies**: TASK-011
  - **Risk**: Medium
- [x] **TASK-013**: MPPC round-trip validation
  - **Acceptance Criteria**: compress → decompress round-trip produces identical output for various inputs
  - **Dependencies**: TASK-009, TASK-011
  - **Risk**: Low

**Success Criteria**:
- All 4 compression tests pass with byte-exact output
- Round-trip validation passes

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] Compression output matches FreeRDP byte-for-byte

**Detailed Plan**: See `.plan/phases/phase-004.md`

---

### Phase 5: XCRUSH Decompression

**Goal**: Port XCRUSH (RDP 6.1) two-level decompression

**Duration Estimate**: 1 day

**Tasks**:
- [x] **TASK-014**: Port XCRUSH types and structures
  - **Acceptance Criteria**: `XCrushContext`, `XCrushMatchInfo`, `XCrushChunk`, `XCrushSignature`, `Rdp61MatchDetails`, `Rdp61CompressedData` defined
  - **Dependencies**: TASK-003
  - **Risk**: Low
- [x] **TASK-015**: Implement XCRUSH Level 1 decompression
  - **Acceptance Criteria**: Parses match details and reconstructs data from history + literals
  - **Dependencies**: TASK-014
  - **Risk**: High (complex two-level format)
- [x] **TASK-016**: Implement XCRUSH full decompression (Level 1 + Level 2/MPPC)
  - **Acceptance Criteria**: Full xcrush_decompress handling all flag combinations
  - **Dependencies**: TASK-009, TASK-015
  - **Risk**: Medium

**Success Criteria**:
- XCRUSH decompression works for all flag combinations
- Handles Level 1 only and Level 1 + Level 2 (MPPC) paths

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Integration with MPPC decompression verified

**Detailed Plan**: See `.plan/phases/phase-005.md`

---

### Phase 6: XCRUSH Compression & Tests

**Goal**: Port XCRUSH compression and validate with FreeRDP test vectors

**Duration Estimate**: 1 day

**Tasks**:
- [ ] **TASK-017**: Implement XCRUSH chunk computation (rolling hash)
  - **Acceptance Criteria**: Chunk boundaries computed using 32-byte rolling hash with rotation
  - **Dependencies**: TASK-014
  - **Risk**: Medium
- [ ] **TASK-018**: Implement XCRUSH match finding and optimization
  - **Acceptance Criteria**: Hash-based chunk matching with bidirectional extension and overlap removal
  - **Dependencies**: TASK-017
  - **Risk**: High
- [ ] **TASK-019**: Implement XCRUSH full compression (Level 1 + Level 2/MPPC)
  - **Acceptance Criteria**: Complete xcrush_compress producing valid output
  - **Dependencies**: TASK-011, TASK-018
  - **Risk**: High
- [ ] **TASK-020**: Port XCRUSH tests
  - **Acceptance Criteria**: `test_xcrush_compress_bells`, `test_xcrush_compress_island` pass
  - **Dependencies**: TASK-016, TASK-019
  - **Risk**: Medium

**Success Criteria**:
- Both XCRUSH compression tests pass
- Compression output matches FreeRDP byte-for-byte

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] Compression integrates correctly with MPPC Level 2

**Detailed Plan**: See `.plan/phases/phase-006.md`

---

### Phase 7: NCRUSH Decompression

**Goal**: Port NCRUSH (RDP 6.0) Huffman-based decompression

**Duration Estimate**: 1 day

**Tasks**:
- [ ] **TASK-021**: Port NCRUSH constants and Huffman tables
  - **Acceptance Criteria**: All static tables ported: `HuffTableLEC`, `HuffTableLOM`, `HuffTableMask`, `CopyOffsetBitsLUT`, `CopyOffsetBaseLUT`, `LOMBitsLUT`, `LOMBaseLUT`, Huffman code/length tables
  - **Dependencies**: TASK-003
  - **Risk**: Low (tedious but straightforward)
- [ ] **TASK-022**: Implement NCRUSH context struct
  - **Acceptance Criteria**: `NCrushContext` with history buffer, hash table, match table, offset cache, Huffman tables
  - **Dependencies**: TASK-021
  - **Risk**: Low
- [ ] **TASK-023**: Implement `ncrush_decompress` function
  - **Acceptance Criteria**: Decompresses NCRUSH data using Huffman decoding, offset cache, history buffer
  - **Dependencies**: TASK-004, TASK-022
  - **Risk**: High (Huffman decoding + offset cache + history management)
- [ ] **TASK-024**: Port NCRUSH decompression test
  - **Acceptance Criteria**: `test_ncrush_decompress_bells` passes
  - **Dependencies**: TASK-023
  - **Risk**: Medium

**Success Criteria**:
- NCRUSH decompression test passes
- Output matches FreeRDP byte-for-byte

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] Decompression handles all Huffman symbol types correctly

**Detailed Plan**: See `.plan/phases/phase-007.md`

---

### Phase 8: NCRUSH Compression & Tests

**Goal**: Port NCRUSH compression and validate

**Duration Estimate**: 1 day

**Tasks**:
- [ ] **TASK-025**: Implement NCRUSH hash-chain match finding
  - **Acceptance Criteria**: 2-byte hash lookup with chain traversal, finds best match
  - **Dependencies**: TASK-022
  - **Risk**: Medium
- [ ] **TASK-026**: Implement NCRUSH Huffman encoding
  - **Acceptance Criteria**: Encodes literals, copy-offsets, offset-cache refs, and lengths using Huffman tables
  - **Dependencies**: TASK-005, TASK-025
  - **Risk**: High
- [ ] **TASK-027**: Implement `ncrush_compress` function
  - **Acceptance Criteria**: Complete compression with history management, window sliding
  - **Dependencies**: TASK-026
  - **Risk**: High
- [ ] **TASK-028**: Port NCRUSH compression test
  - **Acceptance Criteria**: `test_ncrush_compress_bells` passes
  - **Dependencies**: TASK-027
  - **Risk**: Medium
- [ ] **TASK-029**: NCRUSH round-trip validation
  - **Acceptance Criteria**: compress → decompress round-trip produces identical output
  - **Dependencies**: TASK-023, TASK-027
  - **Risk**: Low

**Success Criteria**:
- NCRUSH compression test passes
- Round-trip validation passes

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] All Huffman encoding/decoding paths exercised

**Detailed Plan**: See `.plan/phases/phase-008.md`

---

### Phase 9: Bulk Compressor Coordinator

**Goal**: Port the bulk.c routing layer that selects the right algorithm based on compression level

**Duration Estimate**: 0.5 days

**Tasks**:
- [ ] **TASK-030**: Implement `BulkCompressor` struct
  - **Acceptance Criteria**: Holds MPPC/XCRUSH/NCRUSH contexts, routes compress/decompress based on compression type
  - **Dependencies**: TASK-009, TASK-011, TASK-016, TASK-019, TASK-023, TASK-027
  - **Risk**: Low
- [ ] **TASK-031**: Implement bulk compress/decompress routing
  - **Acceptance Criteria**: Correctly dispatches to MPPC (8K/64K), XCRUSH, or NCRUSH based on flags
  - **Dependencies**: TASK-030
  - **Risk**: Low
- [ ] **TASK-032**: Unit tests for bulk coordinator
  - **Acceptance Criteria**: Tests verify correct routing for each compression type
  - **Dependencies**: TASK-031
  - **Risk**: Low

**Success Criteria**:
- Bulk coordinator correctly routes to all three algorithms
- All flag combinations handled properly

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] Public API is clean and documented

**Detailed Plan**: See `.plan/phases/phase-009.md`

---

### Phase 10: Memory Safety Refactoring

**Goal**: Remove all `unsafe` code, replace with safe Rust patterns while maintaining performance

**Duration Estimate**: 1 day

**Tasks**:
- [ ] **TASK-033**: Audit all `unsafe` blocks and raw pointer usage
  - **Acceptance Criteria**: Complete list of unsafe usages with safe alternatives identified
  - **Dependencies**: TASK-032
  - **Risk**: Low
- [ ] **TASK-034**: Replace unsafe pointer arithmetic with slice operations and bounds-checked indexing
  - **Acceptance Criteria**: All history buffer operations use safe slice methods
  - **Dependencies**: TASK-033
  - **Risk**: Medium (must maintain performance)
- [ ] **TASK-035**: Ensure all tests still pass after safety refactoring
  - **Acceptance Criteria**: Full test suite passes, no regressions
  - **Dependencies**: TASK-034
  - **Risk**: Medium

**Success Criteria**:
- Zero `unsafe` blocks in the crate (or minimized with clear justification)
- All tests pass
- No significant performance regression

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] Tests passing
- [ ] `#![forbid(unsafe_code)]` enabled (or all remaining unsafe justified)

**Detailed Plan**: See `.plan/phases/phase-010.md`

---

### Phase 11: WebAssembly Compatibility & Final Integration

**Goal**: Ensure crate builds for wasm32-unknown-unknown and integrate with IronRDP

**Duration Estimate**: 0.5 days

**Tasks**:
- [ ] **TASK-036**: Verify wasm32-unknown-unknown build
  - **Acceptance Criteria**: `cargo check --target wasm32-unknown-unknown -p ironrdp-bulk` succeeds
  - **Dependencies**: TASK-035
  - **Risk**: Low (crate is `no_std` + `alloc`)
- [ ] **TASK-037**: Final cleanup, documentation, and code review prep
  - **Acceptance Criteria**: All public items documented, README written, clippy clean, no dead code
  - **Dependencies**: TASK-036
  - **Risk**: Low

**Success Criteria**:
- Crate builds for wasm32
- Code is clean, documented, and ready for review

**Phase Completion Checklist**:
- [ ] All tasks completed
- [ ] wasm build passes
- [ ] Documentation complete
- [ ] Clippy clean

**Detailed Plan**: See `.plan/phases/phase-011.md`

---

## Task Dependencies

```
TASK-001 → TASK-002 → TASK-003
                ↓
        TASK-004, TASK-005 → TASK-006
                ↓
TASK-003 + TASK-004 → TASK-007 → TASK-008 → TASK-009 → TASK-010
                                     ↓
                      TASK-005 + TASK-008 → TASK-011 → TASK-012
                                              ↓
                                    TASK-009 + TASK-011 → TASK-013

TASK-003 → TASK-014 → TASK-015 → TASK-016 (needs TASK-009)
                ↓
        TASK-017 → TASK-018 → TASK-019 (needs TASK-011)
                                 ↓
                        TASK-016 + TASK-019 → TASK-020

TASK-003 → TASK-021 → TASK-022 → TASK-023 (needs TASK-004) → TASK-024
                         ↓
                  TASK-025 → TASK-026 (needs TASK-005) → TASK-027 → TASK-028
                                                           ↓
                                                   TASK-023 + TASK-027 → TASK-029

All algorithms → TASK-030 → TASK-031 → TASK-032
TASK-032 → TASK-033 → TASK-034 → TASK-035
TASK-035 → TASK-036 → TASK-037
```

## Risk Assessment

**High Risk Items**:
- **TASK-009** (mppc_decompress): Complex bit-level decoding with two RDP variants. Mitigation: thorough test coverage, compare output byte-by-byte with FreeRDP.
- **TASK-011** (mppc_compress): 3-byte hash matching and variable-length encoding. Mitigation: byte-exact test vectors from FreeRDP.
- **TASK-018** (xcrush match finding): Bidirectional match extension and overlap removal. Mitigation: incremental testing against FreeRDP output.
- **TASK-023** (ncrush_decompress): Huffman decoding with offset cache and history management. Mitigation: FreeRDP test vectors.
- **TASK-027** (ncrush_compress): Full Huffman encoding with window management. Mitigation: round-trip testing.

**Contingency Plans**:
- If byte-exact compression output doesn't match FreeRDP, verify via round-trip (compress in Rust → decompress in Rust, and cross-test with FreeRDP)
- If performance is insufficient after safety refactoring, profile and consider targeted unsafe with safety proofs

## Technical Debt & Future Work

Items intentionally deferred:
- Integration with `ironrdp-session` (decompression during RDP session)
- Integration with `ironrdp-server` (compression during RDP session)
- Fuzzing targets for all three algorithms
- Benchmarks comparing with FreeRDP C implementation
- `PACKET_COMPR_TYPE_RDP8` support (not implemented in FreeRDP bulk.c either)

## Definition of Done

Project is complete when:
- [ ] All 37 tasks completed
- [ ] All phases marked complete
- [ ] All ported unit tests passing
- [ ] Code is memory-safe (no unnecessary unsafe)
- [ ] Builds for wasm32-unknown-unknown
- [ ] All public items documented
- [ ] Clippy clean with no warnings
- [ ] Planning tracking files cleaned up

---

**Next Steps**: 
1. Review and approve this plan
2. Run `execute.prompt.md` to begin execution
3. Execute repeatedly until completion
