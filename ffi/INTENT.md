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
