# IronRDP Session

Abstract state machine to drive an RDP session.

## QOI codecs (features `qoi`, `qoiz-zrip`, `qoiz-zstd`)

`qoiz-zstd` (also enabled by `qoiz`) decompresses the QOI stream with [`zstd-safe`], bindings to the reference C implementation.

Enable `qoiz-zrip` to decompress with [`zrip`], a pure Rust zstd implementation, instead (for example for WASM).

Both options accept the same wire format, so either one decodes what the server sends.

Both backends are maintained permanently.
`zstd-safe` is the default and the reference that tests check `zrip` against, so zstd upgrades and security fixes must be applied to both.

`zrip` is a young project with a single maintainer, and it decodes data received from the server.
It forbids `unsafe` code, reducing the risk of memory-safety vulnerabilities.
Prefer `zstd-safe` where a C toolchain is available and the server is not trusted.

This crate is part of the [IronRDP] project.

[`zrip`]: https://crates.io/crates/zrip
[`zstd-safe`]: https://crates.io/crates/zstd-safe

[IronRDP]: https://github.com/Devolutions/IronRDP
