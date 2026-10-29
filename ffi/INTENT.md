## Deterministic disposal in .NET bindings

The generated .NET API SHOULD expose an opaque type as `IDisposable` when at least one of these conditions is true:

- The type owns a large allocated resource that is invisible to the garbage collector's cost model.
- The type owns a scarce or externally observable resource, regardless of its size, such as an operating-system handle, socket, named pipe, or file lock.
- The type is a borrowing view, directly or transitively, of a disposable type.
  A view must not prevent deterministic release of the value it retains.
  (NOTE: this should be enforced by the diplomat-tool’s .NET backend eventually.)

The API SHOULD NOT expose a small, value-like opaque type as `IDisposable` unless it is a borrowing view of a disposable type.
The default is no `IDisposable`; it is not added defensively.

## Opaque types with a lifetime

No opaque type with a lifetime touches borrowed data when it is dropped, through its own `Drop` or a field's.
.NET runs finalizers in no deterministic order, so the data it borrows may already be freed by then.

## Concurrency

The generated C# code does not make concurrent calls safe.
It throws `InvalidOperationException` when a call that mutates an object overlaps another call on the same object.
It does not check overlapping read-only calls.
The hand-written layer adds locks only at fragile places, such as `Framed`, which lets one `ReadPdu` loop run while other tasks write.
Outside those places, a consumer makes one call at a time on each native object.
When a consumer disposes `WinCliprdr`, it does so on the thread that created it.

## Copying into caller buffers

A method that copies native bytes into a caller's buffer (`Fill`, `FillData`, `ReadIntoBuf`) writes them at the start of the buffer.
It throws `IronRdpException` when the buffer is shorter than the data, and leaves every byte past the data untouched.

<a id="copying-a-rectangle"></a>
## Copying a rectangle into a caller buffer

`CopyRectInto` rejects the rectangles the Rust client rejects (`pack_desktop_update` in `crates/ironrdp-client/src/rdp.rs`): an inverted one, and one that reaches past the image.
Like that function, it rejects a row offset that overflows instead of letting it wrap.
Where Rust code would panic on an out-of-range slice, such as a stride or buffer too short for the rectangle, it throws `IronRdpException` instead.
It writes nothing when it rejects.

## PDU framing in .NET

`Framed` sizes an active-stage PDU from its first four buffered bytes, because a TPKT header (MS-RDPBCGR 2.2.9.1.1) is four bytes and a fast-path header (MS-RDPBCGR 2.2.9.1.2) at most three.
The cost of finding a PDU therefore does not grow with the bytes buffered behind it.
A connection-sequence read (`ReadByHint`) hands the hint everything buffered, because a hint may need more than a fixed header and those PDUs are few and small.
Every read reports a stream that ends before the PDU is complete, in its header or its body, as `IronRdpLibException` with `EndOfFile`.
`TryReadBufferedPdu` lets a consumer drain the PDUs already buffered before it flushes rendering, and `ReadPduSync` serves a blocking read loop on a dedicated thread.
Both are reads under the one-read-at-a-time rule above.
