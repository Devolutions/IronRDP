# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-str-v0.1.0)] - 2026-03-18

### <!-- 1 -->Features

- Add ironrdp-str crate with typed wire-aware RDP string primitives ([#1159](https://github.com/Devolutions/IronRDP/issues/1159)) ([a67b1aace7](https://github.com/Devolutions/IronRDP/commit/a67b1aace71d267287d4dc7ea6d4ee88068e6179)) 

  Introduces `ironrdp-str`, a new crate providing lazily-validated string
  types covering all RDP UTF-16LE field shapes:
  
  - `FixedString<N>`: fixed-size fields (e.g. `clientName`, `fileName`),
  zero-padded on encode, trailing-null stripped on decode.
  - `PrefixedString<P, N>`: length-prefixed fields with configurable
  `LengthPrefix` (`CchU16`, `CchU32`, `CbU16`) and `NullTerminatorPolicy`
  (`NullCounted`, `NullUncounted`, `NoNull`) type parameters.
  - `UnframedString`: externally-lengthed fields whose length comes from a
  sibling field in the containing message.
  - `MultiSzString`: `MULTI_SZ` string lists (e.g. `HardwareIds` in
  MS-RDPEUSB §2.2.4.2), with a `u32 cch` prefix counting all null
  terminators including the final sentinel.
  
  **Key design invariant**: wire bytes are stored as `Vec<u16>` and never
  eagerly converted to Rust strings, enabling single-allocation decode and
  zero-cost decode→encode passthrough for proxy use cases. Conversion is
  deferred to explicit `to_native()` / `to_native_lossy()` calls.
  
  Uses `bytemuck` for zero-copy LE→u16 reinterpretation on little-endian
  targets; falls back to per-element byte-swap on big-endian.


