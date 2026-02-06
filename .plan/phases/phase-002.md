# Phase 2: Bitstream Utilities

**Status**: ⏳ Pending  
**Phase Number**: 2 of 11  
**Estimated Duration**: 0.5 days

## Overview

Port FreeRDP's `wBitStream` reader/writer to Rust. These utilities are used by all three compression algorithms for bit-level encoding and decoding.

## Dependencies

Requires Phase 1 completion (module structure in place).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-004 | Implement BitStreamReader | TASK-002 | Medium | ⏳ Pending |
| TASK-005 | Implement BitStreamWriter | TASK-002 | Medium | ⏳ Pending |
| TASK-006 | Unit tests for bitstream utilities | TASK-004, TASK-005 | Low | ⏳ Pending |

## Detailed Task Breakdown

### TASK-004: Implement BitStreamReader

**Description**: Port the read side of FreeRDP's `wBitStream`.

**FreeRDP Reference** (`winpr/include/winpr/bitstream.h`):
The bitstream reader uses a 32-bit `accumulator` loaded in big-endian order from the buffer, with a 32-bit `prefetch` lookahead. The `offset` tracks bits consumed within the current 32-bit word. When `offset >= 32`, the reader advances `pointer` by 4 bytes and refetches.

**Key operations to port**:
- `BitStream_Attach(bs, buffer, capacity)` → `BitStreamReader::new(data: &[u8])`
- `BitStream_Fetch(bs)` → loads 4 bytes big-endian into accumulator, prefetches next 4
- `BitStream_Shift(bs, nbits)` → shifts accumulator left by nbits, fills from prefetch
- Reading N bits: peek at top N bits of accumulator, then shift

**Rust API**:
```rust
pub struct BitStreamReader<'a> {
    buffer: &'a [u8],
    position: usize,     // byte position for next fetch
    offset: u32,         // bits consumed in current accumulator
    accumulator: u32,
    prefetch: u32,
    length: usize,       // total bits available
}

impl<'a> BitStreamReader<'a> {
    pub fn new(data: &'a [u8]) -> Self;
    pub fn read_bits(&mut self, nbits: u32) -> u32;
    pub fn peek_bits(&self, nbits: u32) -> u32;
    pub fn remaining_bits(&self) -> usize;
}
```

---

### TASK-005: Implement BitStreamWriter

**Description**: Port the write side of FreeRDP's `wBitStream`.

**FreeRDP Reference**:
The writer accumulates bits in a 32-bit accumulator starting from bit 31 (MSB). `offset` tracks bits written. When `offset >= 32`, the accumulator is flushed big-endian to the buffer and pointer advances by 4.

**Key operations to port**:
- `BitStream_Write_Bits(bs, bits, nbits)` → writes nbits from value into accumulator
- `BitStream_Flush(bs)` → writes remaining accumulator bytes to buffer

**Rust API**:
```rust
pub struct BitStreamWriter<'a> {
    buffer: &'a mut [u8],
    position: usize,     // byte position for next flush
    offset: u32,         // bits written in current accumulator
    accumulator: u32,
    capacity: usize,     // total capacity in bits
}

impl<'a> BitStreamWriter<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self;
    pub fn write_bits(&mut self, value: u32, nbits: u32);
    pub fn flush(&mut self);
    pub fn position_bits(&self) -> usize;
    pub fn byte_length(&self) -> usize;  // bytes written including partial
}
```

---

### TASK-006: Unit tests for bitstream utilities

**Tests to implement**:
1. Read 1 bit at a time from known buffer
2. Read 8 bits = one byte
3. Read across 32-bit boundary (accumulator reload)
4. Write various bit widths and verify buffer contents
5. Round-trip: write bits → read back → compare

## Success Criteria

- [ ] BitStreamReader correctly reads bits from byte buffers
- [ ] BitStreamWriter correctly writes bits to byte buffers
- [ ] All unit tests pass

## Phase Execution Strategy

TASK-004 and TASK-005 can be done in parallel. TASK-006 depends on both.
