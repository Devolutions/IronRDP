# RFX decoder optimizations

This note tracks performance work on the RemoteFX (RFX) tile decode path in `ironrdp-graphics`.
Each optimization records what changed and how much it is faster.

## Optimizations

### RLGR bit reader (#2150)

What changed:
the `bitvec`-based reader in `rlgr::decode` is replaced by `BitReader`, which buffers up to 64 bits in a `u64` accumulator.
The decoder now uses `skip_leading` and `read` instead of slice splitting.

Why it is faster:
refilling takes one bounds check for up to eight bytes, and any read of 32 bits or fewer is served from the accumulator.
Run lengths of zeros and ones come from `leading_zeros` on the whole word instead of a per-bit scan.

Measured result:
RLGR1 and RLGR3 entropy decoding run 1.97x and 2.17x faster, and single-tile decode is 1.6x to 3.7x faster.

## Rejected approaches

TBD. Record ideas that were tried and did not help, with the measured effect.

## Running the benchmarks

```shell
# Native target
cargo xtask bench rfx-decode
# WASM target
cargo xtask bench rfx-decode-wasm --simd
```
