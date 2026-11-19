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

## Direct dynamic virtual channels

Register channel names with `DirectDvcConfig` and set `ConfigBuilder.DirectDvc`, or attach it with `ClientConnector.WithDynamicChannelDirect`.
Create its `DirectDvcEventQueue` with a positive event capacity and poll `NextEvent()` for `Opened`, `Data` and `Closed` events.
The capacity limits event count, not aggregate payload bytes.
`NextEvent()` returns null when the queue is empty.
Each event carries the channel name, server channel ID and an endpoint ID unique to that channel opening; call `TakeData()` to transfer a data event's owned bytes into `VecU8`.
Dispose the queue, each returned event and every `VecU8` from `TakeData()` when finished to release native memory promptly.
Pass both IDs to `ActiveStage.SendDirectDvcData` or `CloseDirectDvcChannel`; each method checks that the endpoint is still active when encoding the frame.
After a channel closes or reopens, the previous endpoint ID is rejected.
On queue overflow, `NextEvent()` reports an error immediately, even if events remain queued, and continues to fail on later calls.
Overflow drops an application-level event; it does not corrupt RDP framing or DVC reassembly.
Channels configured with the same event sink share a queue, so overflow stops delivery for each of those channels and later inbound events may fail processing.
Because the application event history is incomplete and the queue cannot recover, stop acting on its events and close the session.
Both methods return an encoded frame for the main RDP connection and fail if the channel was moved to a multitransport tunnel.

Use one serialized pipeline for `ActiveStage.Process`, direct send and close encoding, and the resulting transport writes.
Write the responses from `Process`, including the channel create response, before acting on the resulting DVC events in that pipeline.
Write each encoded direct DVC frame before processing another inbound PDU that could close or reopen its channel; endpoint IDs are checked during encoding, not again when the frame is written.

```csharp
var events = DirectDvcEventQueue.New(256);
var directDvc = DirectDvcConfig.New(events.Sink);
directDvc.AddChannel("my-channel");
builder.DirectDvc = directDvc;
```

## Dynamic resize

`ClientConnector.WithDynamicChannelDisplayControl()` requests the Display Control channel ([MS-RDPEDISP]).
The server opens it and sends its capabilities some time after the connection is established, so `ActiveStage.EncodedResize` returns `null` until then.
Read `ActiveStage.DisplayControlState` after each `Process` call: it moves from `Pending` to `Ready` when the capabilities arrive, which is when to send the first resize.
`Unavailable` means the channel was not requested.
`ActiveStage.DisplayControlCapabilities` exposes the server's monitor count and area limits once `Ready`.

## Touch and pen input (MS-RDPEI)

- Attach the channel before connecting: `connector.WithDynamicChannelRdpei(maxTouchContacts, showTouchVisuals)`.
- Feed toolkit touch events to a `TouchFrameBuilder.New(maxTouchContacts)` with `Push(touchId, TouchPhase, x, y)` or `PushWithPressure`, in desktop coordinates.
- Feed pen samples to a `PenFrameBuilder.New()` with `Push(penId, PenPhase, x, y)` or `PushWithAttributes` for buttons, pressure, rotation and tilt; one pen is tracked at a time.
- Once `activeStage.RdpeiInputState.Touch` (or `.Pen`) is true, `activeStage.EncodeRdpeiTouch(builder)` or `EncodeRdpeiPen(builder)` returns the frame to write to the transport, or `null` when nothing is pending.
  It throws while the channel is not ready or the server suspended input, and the frames stay pending.
  One frame carries at most 32767 touch or pen frames, so call again while `HasPendingFrames` is true.
- `activeStage.EncodeRdpeiDismissHovering(contactId)` tells the server to stop tracking a hovering touch contact.
- Call `CancelAll()` when the window loses focus, `Reset()` when the session ends, and dispose the builders with it: a long suspension can leave tens of thousands of frames pending in native memory.

## Migrating from 2026.10.19

- Properties replace the `GetX()` and `SetX()` methods they duplicated.
- Only these types are `IDisposable`: `ActiveStage`, `ActiveStageOutput`, `ActiveStageOutputIterator`, `ClipboardMessage`, `ClipboardSvcMessage`, `CredsspProcessGenerator`, `DecodedImage`, `DecodedPointer`, `FfiFileContentsResponse`, `FormatDataResponse`, `PenFrameBuilder`, `TouchFrameBuilder`, `VecU8`, `WinCliprdr` and `WriteBuf`.
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
