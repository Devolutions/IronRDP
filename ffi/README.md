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

## Dynamic resize

`ClientConnector.WithDynamicChannelDisplayControl()` requests the Display Control channel ([MS-RDPEDISP]).
The server opens it and sends its capabilities some time after the connection is established, so `ActiveStage.EncodedResize` returns `null` until then.
Read `ActiveStage.DisplayControlState` after each `Process` call: it moves from `Pending` to `Ready` when the capabilities arrive, which is when to send the first resize.
`Unavailable` means the channel was not requested.
`ActiveStage.DisplayControlCapabilities` exposes the server's monitor count and area limits once `Ready`.

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
- `FastPathInputEventIterator.Encode()` returns the Fast-Path Input PDUs for the events of one `InputDatabase.Apply` as a `VecU8`, ready for `Framed.Write`.
  It reads no `ActiveStage` or `DecodedImage`, so call it outside that lock and input no longer waits on frame decoding.
  `ActiveStage.ProcessFastpathInput` sends the same bytes; keep it, under the lock, only when `PointerSoftwareRendering` is on, because it also redraws the pointer in the image.
  `InputDatabase` tracks pressed keys and buttons, so keep calling `Apply` from one thread.

[Diplomat]: https://github.com/rust-diplomat/diplomat
[MS-RDPEDISP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp
