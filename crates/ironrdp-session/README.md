# IronRDP Session

Abstract state machine to drive an RDP session.

## QOI codecs (features `qoi`, `qoiz`)

`qoiz` decompresses the QOI stream with zstd, using [`zrip`], a pure Rust zstd implementation.

Enable `qoiz-zstd` to decompress with [`zstd-safe`] (bindings to the reference C implementation)
instead. It performs slightly better on non-WASM targets, but requires a C compiler to build.

Both options accept the same wire format, so a server using either backend interoperates with a
client using either one.

This crate is part of the [IronRDP] project.

[`zrip`]: https://crates.io/crates/zrip
[`zstd-safe`]: https://crates.io/crates/zstd-safe

[IronRDP]: https://github.com/Devolutions/IronRDP
