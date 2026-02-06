# Phase 1: Project Setup & Crate Scaffold

**Status**: ⏳ Pending  
**Phase Number**: 1 of 11  
**Estimated Duration**: 0.5 days

## Overview

Create the `ironrdp-bulk` crate with proper structure, dependencies, and workspace integration. This is the foundation for all subsequent work.

## Dependencies

None - this is the first phase.

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-001 | Create ironrdp-bulk crate with Cargo.toml | None | Low | ⏳ Pending |
| TASK-002 | Set up module structure | TASK-001 | Low | ⏳ Pending |
| TASK-003 | Define shared types and error types | TASK-002 | Low | ⏳ Pending |

## Detailed Task Breakdown

### TASK-001: Create ironrdp-bulk crate with Cargo.toml

**Description**: Create the `crates/ironrdp-bulk/` directory and `Cargo.toml`.

**Acceptance Criteria**:
- [ ] `crates/ironrdp-bulk/Cargo.toml` exists with correct metadata
- [ ] Crate is listed in workspace `Cargo.toml` members
- [ ] `cargo check -p ironrdp-bulk` succeeds

**Implementation Notes**:
- Follow IronRDP naming convention: `ironrdp-bulk`
- Use `edition.workspace = true` and `license.workspace = true`
- Add dependency on `ironrdp-core` (workspace)
- Must be `no_std` compatible: use `#![no_std]` with `extern crate alloc`
- Suggested features: `std` (default), `alloc`

**Cargo.toml template**:
```toml
[package]
name = "ironrdp-bulk"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "Bulk compression algorithms (MPPC, XCRUSH, NCRUSH) for IronRDP"

[dependencies]

[dev-dependencies]

[features]
default = ["std"]
std = []
```

---

### TASK-002: Set up module structure

**Description**: Create the module tree with placeholder files.

**Files to create**:
- `src/lib.rs` - Crate root with module declarations
- `src/mppc/mod.rs` - MPPC module
- `src/xcrush/mod.rs` - XCRUSH module
- `src/ncrush/mod.rs` - NCRUSH module
- `src/bitstream.rs` - Bitstream utilities
- `src/bulk.rs` - Bulk coordinator

---

### TASK-003: Define shared types and error types

**Description**: Define the types shared across all compression algorithms.

**Types to define**:
```rust
/// RDP compression type (low 4 bits of compression flags)
pub enum CompressionType {
    Rdp4 = 0x00,  // MPPC 8K
    Rdp5 = 0x01,  // MPPC 64K
    Rdp6 = 0x02,  // NCRUSH
    Rdp61 = 0x03, // XCRUSH
}

/// Compression flags (high bits)
pub mod flags {
    pub const PACKET_COMPRESSED: u32 = 0x20;
    pub const PACKET_AT_FRONT: u32 = 0x40;
    pub const PACKET_FLUSHED: u32 = 0x80;
    pub const COMPRESSION_TYPE_MASK: u32 = 0x0F;
    
    // Level 1 flags (XCRUSH)
    pub const L1_PACKET_AT_FRONT: u32 = 0x04;
    pub const L1_NO_COMPRESSION: u32 = 0x02;
    pub const L1_COMPRESSED: u32 = 0x01;
    pub const L1_INNER_COMPRESSION: u32 = 0x10;
}
```

## Success Criteria

- [ ] Crate compiles successfully with `cargo check -p ironrdp-bulk`
- [ ] Module structure is in place
- [ ] Shared types are defined

## Phase Execution Strategy

Execute tasks sequentially: TASK-001 → TASK-002 → TASK-003.
