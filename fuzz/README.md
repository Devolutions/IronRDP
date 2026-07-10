# IronRDP fuzzing

## Difference between fuzzing and property testing

`ironrdp` correctness is validated in various ways. Two of these are fuzzing and property testing.
Both of these methods involve feeding random inputs to the API in order to check if the program is
behaving as expected or not.

However,

- Fuzzing is well suited for black-box-like testing.
  Inputs are typically guided by instrumentalizing the code (coverage…) rather than manually informed.

- Property testing requires the developer to describe the interesting inputs and properties to test.

- When fuzzing, some properties are tested as well, but those are typically simple (absence of crash, round-trip…).

- In contrast, property testing is well suited when testing more complex properties.

- With fuzzing, we are actively trying to show that something is (unexpectedly) broken.

- With property testing, we are actively trying to show that the properties are holding (as expected).

## Targets

### `pdu_decoding`

Feeds random inputs to PDU decoding code.

### `bitmap_stream`

Feeds random inputs to the RDP6 bitmap decoder.

### `rle_decompression`

Feeds random inputs to the interleaved Run-Length Encoding (RLE) bitmap decoder.

### `bulk_mppc`

Feeds random inputs to the MPPC bulk decompressor (`ironrdp-bulk`). The first
input byte selects between RDP4 mode (low bit clear, 8 KB history) and RDP5
mode (low bit set, 64 KB history); remaining bytes are the compressed payload.

### `bulk_ncrush`

Feeds random inputs to the NCRUSH bulk decompressor (RDP6.0, `ironrdp-bulk`).

### `bulk_xcrush`

Feeds random inputs to the XCRUSH bulk decompressor (RDP6.1, `ironrdp-bulk`).
XCRUSH has the largest sliding-window history of the bulk family (2 MB).

### `bulk_round_trip`

Compresses arbitrary input via `BulkCompressor::compress`, then decompresses
the result via `BulkCompressor::decompress`, and asserts byte-equality with
the original input. First input byte selects the algorithm (RDP4 / RDP5 /
RDP6 / RDP6.1). Catches asymmetric bugs where the compressor produces output
the decompressor cannot consume, or the decompressor produces output that
does not equal the original input.

## Building crates with the `arbitrary` feature

Several crates expose an optional `arbitrary` feature that enables
[`arbitrary::Arbitrary`](https://docs.rs/arbitrary) implementations on their
PDU types. This is the foundation for structure-aware fuzzing harnesses that
generate valid-looking inputs rather than raw bytes.

To verify the feature compiles cleanly for a single crate:

```shell
cargo check -p ironrdp-pdu --features arbitrary
```

The feature is also compatible with the `no_std + alloc` build path:

```shell
cargo check -p ironrdp-pdu --no-default-features --features arbitrary,alloc
```

A handful of PDU types do not implement `Arbitrary`. They fall into two categories:

- **Types with non-derivable fields** (e.g., `StaticChannelSet` keyed by `TypeId`).
  These are either skipped via `#[arbitrary(default)]` on the containing struct's
  field, or hand-rolled with a placeholder.
- **Error types that are not part of the wire-protocol surface** (e.g.,
  `ServerLicenseError`). Most error enums fall here: they are constructed locally
  rather than decoded from the wire, so the fuzzer has no reason to generate them.
  The exception is wire-protocol error PDUs such as `DisconnectProviderUltimatum`
  in `mcs.rs`, which do implement `Arbitrary` because they are decoded from the wire.

Inline source comments mark each skip with the rationale.
