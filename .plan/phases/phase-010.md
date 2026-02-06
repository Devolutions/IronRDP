# Phase 10: Memory Safety Refactoring

**Status**: ⏳ Pending  
**Phase Number**: 10 of 11  
**Estimated Duration**: 1 day

## Overview

Remove all `unsafe` code and replace with safe Rust patterns. The initial implementation may use unsafe for faster porting; this phase converts everything to idiomatic, safe Rust.

## Dependencies

Requires Phase 9 (all algorithms working with tests passing).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-033 | Audit all unsafe blocks | TASK-032 | Low | ⏳ Pending |
| TASK-034 | Replace unsafe with safe patterns | TASK-033 | Medium | ⏳ Pending |
| TASK-035 | Verify tests pass after refactoring | TASK-034 | Medium | ⏳ Pending |

## Detailed Task Breakdown

### TASK-034: Safe Rust patterns to use

**History buffer operations**:
- Use `&history[start..end]` slices instead of pointer arithmetic
- Use `.copy_within()` for overlapping copies within history buffer
- Use wrapping index: `index & mask` with bounds-checked array access

**Bitstream operations**:
- All buffer reads through `.get()` with error handling
- No raw pointer dereference for big-endian loads — use `u32::from_be_bytes()`

**Hash table operations**:
- Use standard array indexing with bounds from hash mask
- No unchecked indexing

**Common patterns**:
```rust
// Instead of: unsafe { *ptr.add(offset) }
// Use: buffer[offset]  or  buffer.get(offset).ok_or(Error)?

// Instead of: unsafe { ptr::copy(src, dst, len) }
// Use: buffer.copy_within(src_range, dst_start)

// Instead of: unsafe { *(ptr as *const u32) }
// Use: u32::from_be_bytes(buffer[i..i+4].try_into().unwrap())
```

## Success Criteria

- [ ] Zero `unsafe` blocks (or clearly justified minimal unsafe)
- [ ] All tests pass unchanged
- [ ] No performance regression on test data
