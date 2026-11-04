# IronRDP native RDPEI backend

`WindowsInput` injects RDPEI touch and pen events into the current Windows desktop.
It translates contact fields, preserves frame timing, tracks active contacts, and owns synthetic pen devices.
The crate is a Windows-only extra-tier backend; `ironrdp-rdpei` remains portable and handles the wire protocol.

Create one injector on the desktop's input thread, call `initialize` with the negotiated `CsReadyPdu`, then forward `touch`, `pen`, and `dismiss` calls in order.
The asynchronous methods are executor-agnostic and must remain on that thread.
Fallible methods return `WindowsInputError`, grouped by invalid input, invalid state, native API, or clock failure.
The injector is deliberately neither `Send` nor `Sync`, and only one instance can own touch injection in the process.
Do not mix it with another caller of Windows touch-injection APIs.
Coordinates are desktop pixel coordinates; the caller handles desktop selection, permissions, and any coordinate mapping.

Call `reset` when input is suspended or the client disconnects.
It cancels tracked touch contacts, destroys pen devices, and requires initialization before further input.
Dropping the injector performs the same cleanup, logging failures because native cancellation is best-effort.
Explicit reset can block for up to 100 ms while waiting to cancel touch contacts.
Cancellation of an in-progress asynchronous call also requires reset before the caller resumes input.

No companion process, session-control transport, cursor backend, or server runtime is included.
