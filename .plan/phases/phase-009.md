# Phase 9: Bulk Compressor Coordinator

**Status**: ⏳ Pending  
**Phase Number**: 9 of 11  
**Estimated Duration**: 0.5 days

## Overview

Port the bulk.c routing layer that selects the right compression algorithm based on RDP compression level flags.

## Dependencies

Requires all three algorithms (Phases 3-8).

## Tasks in This Phase

| ID | Title | Dependencies | Risk | Status |
|---|---|---|---|---|
| TASK-030 | Implement BulkCompressor struct | All algorithms | Low | ⏳ Pending |
| TASK-031 | Implement bulk compress/decompress routing | TASK-030 | Low | ⏳ Pending |
| TASK-032 | Unit tests for bulk coordinator | TASK-031 | Low | ⏳ Pending |

## Detailed Task Breakdown

### TASK-030: BulkCompressor struct

```rust
pub struct BulkCompressor {
    compression_level: CompressionType,
    mppc_send: Option<MppcContext>,
    mppc_recv: Option<MppcContext>,
    xcrush_send: Option<XCrushContext>,
    xcrush_recv: Option<XCrushContext>,
    ncrush_send: Option<NCrushContext>,
    ncrush_recv: Option<NCrushContext>,
    output_buffer: Vec<u8>,
}
```

### TASK-031: Routing logic

**From bulk.c**:
- Extract compression type: `flags & 0x0F`
- Type 0x00: MPPC 8K → mppc with RDP4 level
- Type 0x01: MPPC 64K → mppc with RDP5 level
- Type 0x02: NCRUSH → ncrush
- Type 0x03: XCRUSH → xcrush
- Skip compression if size <= 50 or >= 16384

## Success Criteria

- [ ] Bulk coordinator correctly routes to all algorithms
- [ ] All flag combinations handled
- [ ] Public API is clean and well-documented
