# IronRDP DVC COM Plugin

Loader for native Windows [DVC (Dynamic Virtual Channel)][dvc-overview] client plugin DLLs
(e.g. `webauthn.dll`) that bridges them into IronRDP's DVC infrastructure.

The plugin DLL is loaded via `LoadLibraryW`, its `VirtualChannelGetInstance` export is called
to obtain `IWTSPlugin` COM objects, and a Rust implementation of `IWTSVirtualChannelManager`
bridges data bidirectionally between the plugin's COM callbacks and IronRDP's DVC system.

This crate is **Windows-only** (`#![cfg(windows)]`).

## Architecture

A dedicated COM worker thread owns all COM objects (which are `!Send`). The `DvcComChannel`
structs (which implement `DvcProcessor + Send`) are registered as DVC channels in IronRDP's
`DrdynvcClient` and communicate with the COM thread via `std::sync::mpsc` channels.

Outbound data from the plugin (`IWTSVirtualChannel::Write`) is injected into the active
session loop via the `on_write_dvc` callback, following the same pattern as
`ironrdp-dvc-pipe-proxy`.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[dvc-overview]: https://learn.microsoft.com/en-us/windows/win32/termserv/writing-a-client-dvc-component
