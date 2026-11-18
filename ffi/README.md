# IronRDP FFI

[Diplomat]-based FFI for IronRDP.

Currently, only the .NET target is officially supported.

See the [FFI capability matrix](CAPABILITIES.md) for the current surface and its coverage relative to the Rust client.

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

## Native audio

Attach native playback and, optionally, microphone capture before starting the connector.
Each connection needs its own `NativeAudio` object.
Native playback also joins RDPDR when it is absent, because Windows servers require that channel before starting playback.
The added RDPDR channel redirects no devices and does not replace an existing RDPDR configuration.
Empty device selectors follow the system default; otherwise supply a device ID or name.
The latency is in milliseconds; zero selects the native default.

```csharp
using var audio = NativeAudio.New(
    playback: true, capture: true, outputDevice: "", inputDevice: "", latencyMs: 0);
connector.WithNativeAudio(audio);
```

In the serialized active-stage loop, call `PollAudio(audio)` before and after processing an incoming PDU and at least every 50 ms while the connection is idle.
The bounded queue holds at least 100 ms at up to 48 kHz and two channels, including after a mono-to-stereo format change, and each poll drains the queued batches.
If the session loop stalls long enough to exhaust that buffer, capture is stopped and its dynamic channel is closed rather than continuing with missing messages.
Write each returned `ResponseFrame` to the same transport, in order, and dispose the output iterator and frames.
Each audio response can contain multiple complete RDP packets.
Device and stream failures are logged through Rust tracing rather than returned as managed status events.
An Open failure describes one capture attempt, not permanent microphone unavailability: the server can reopen the channel and retry.
Opening capture and a FormatChange that changes rate or channels wait up to two seconds for device readiness in the serialized session loop.
This pauses session processing, and cleanup can take longer if a driver blocks.
Stopping audio closes capture before waiting for playback cleanup.
Capture samples received before the client's Open reply is queued are discarded so they cannot overtake the reply.

```csharp
async Task SendAudioOutputs(ActiveStageOutputIterator outputs)
{
    using (outputs)
    {
        while (outputs.Next() is { } output)
        {
            using (output)
            using (var frame = output.ResponseFrame)
            {
                var bytes = new byte[frame.Size];
                frame.Fill(bytes);
                await framed.Write(bytes);
            }
        }
    }
}

await SendAudioOutputs(stage.PollAudio(audio));

// Stop audio without disconnecting the desktop.
await SendAudioOutputs(stage.StopAudio(audio));
```

Serialize `PollAudio`, `StopAudio`, and `Process` on the same `ActiveStage`.
Stopping audio is permanent for that connection; dispose the stage on disconnect to release device streams, and create new audio settings when reconnecting.
Disposing `NativeAudio` releases its handle but does not stop an attached stage.
Microphone capture requires operating-system permission; iOS applications must provide a microphone usage description and request access.
Linux builds require ALSA development headers.
The native FFI library requires `libasound` at runtime to load, even when audio is disabled.
Android audio is unsupported so that the existing Android API 21 baseline remains usable.

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
- These types are `IDisposable`: `ActiveStage`, `ActiveStageOutput`, `ActiveStageOutputIterator`, `ClientConnectorState`, `ClipboardMessage`, `ClipboardSvcMessage`, `ConnectionResult`, `CredsspProcessGenerator`, `DecodedImage`, `DecodedPointer`, `FfiFileContentsResponse`, `FormatDataResponse`, `NativeAudio`, `PenFrameBuilder`, `TouchFrameBuilder`, `VecU8`, `WinCliprdr` and `WriteBuf`.
  Every other type is released by its finalizer, so drop `using` and `Dispose()` on it.
- `ClientConnector` remains finalizer-managed because its `NextPduHint` and `DynState` views borrow from it, so generated .NET code cannot safely expose `Dispose` while those views retain their source.
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
