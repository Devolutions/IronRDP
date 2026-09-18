# IronRDP FFI

[Diplomat]-based FFI for IronRDP.

## Diplomat pin (temporary)

Pinned to `irvingoujAtDevolution/diplomat@ab68f7a4` for
[rust-diplomat/diplomat#1250](https://github.com/rust-diplomat/diplomat/pull/1250)
(`Result`/`Option` of `Box<[u8]>` → .NET `RustVec`).

IronRDP still needs a full diplomat **0.7 → 0.16** catch-up before this pin
builds cleanly. After #1250 merges, publish a Devolutions diplomat tag and
finish the migration (same pattern as IronVNC).

Currently, only the .NET target is officially supported.

## How to build

- Install required tools: `cargo xtask ffi install`
  - For .NET, note that `dotnet` is also a requirement that you will need to install on your own.

- Build the shared library: `cargo xtask ffi build` (alternatively, in release mode: `cargo xtask ffi build --release`)

- Build the bindings: `cargo xtask ffi bindings`

At this point, you may build and run the examples for .NET:

- `dotnet run --project Devolutions.IronRdp.ConnectExample`
- `dotnet run --project Devolutions.IronRdp.AvaloniaExample`

[Diplomat]: https://github.com/rust-diplomat/diplomat
