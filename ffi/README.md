# IronRDP FFI

[Diplomat]-based FFI for IronRDP.

Currently, only the .NET target is officially supported.

## How to build

- Install required tools: `cargo xtask ffi install`
  - For .NET, note that `dotnet` is also a requirement that you will need to install on your own.

- Build the shared library: `cargo xtask ffi build` (alternatively, in release mode: `cargo xtask ffi build --release`)

- Build the bindings: `cargo xtask ffi bindings`

At this point, you may build and run the examples for .NET:

- `dotnet run --project Devolutions.IronRdp.ConnectExample`
- `dotnet run --project Devolutions.IronRdp.AvaloniaExample`

## Migrating from 2026.10.19

- Properties replace the `GetX()` and `SetX()` methods they duplicated.
- Only these types are `IDisposable`: `ActiveStage`, `ActiveStageOutput`, `ActiveStageOutputIterator`, `ClipboardMessage`, `ClipboardSvcMessage`, `CredsspProcessGenerator`, `DecodedImage`, `DecodedPointer`, `FfiFileContentsResponse`, `FormatDataResponse`, `VecU8`, `WinCliprdr` and `WriteBuf`.
  Every other type is released by its finalizer, so drop `using` and `Dispose()` on it.
- Dispose `WinCliprdr` on the thread that created it, because only that thread can remove its window subclass.
- Dispose a `CredsspProcessGenerator` before the next call on its `CredsspSequence`.
- `DecodedImage.Data` is gone: size a buffer with `DataSize` and copy into it with `FillData`.
  `ResponseFrame`, `WindowingOrders` and `DecodedPointer.Data` return a `VecU8` instead of a `BytesSlice`.
- `WindowingOrders` reports an encoding failure as `IronRdpErrorKind.EncodeError` instead of `Generic`.
- A native object rejects overlapping calls with `InvalidOperationException`.
  Serialize access to `ActiveStage` and `DecodedImage` across the PDU loop, input handlers and clipboard, as the Avalonia example does with one lock.

[Diplomat]: https://github.com/rust-diplomat/diplomat
