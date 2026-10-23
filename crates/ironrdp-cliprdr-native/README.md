# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations: Windows (`WinClipboard`) and Linux desktops over X11 and Wayland (`LinuxClipboard`, text and images).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

## Windows

`WinClipboard` runs on the thread that creates it, which must have a message loop (a GUI thread): a hidden window listens for clipboard changes and renders remote formats on demand.
Only formats whose data is a self-contained `HGLOBAL` are exchanged; handle-based formats (`CF_BITMAP`, `CF_ENHMETAFILE`, `CF_METAFILEPICT`, `CF_PALETTE`), local paths and shell items (`CF_HDROP`, `FileNameW`, `Shell IDList Array`) and OLE storage formats are never advertised in either direction.
Windows synthesizes `CF_DIB` from `CF_BITMAP`, so images still go through.

With file transfer enabled (`WinClipboard::with_file_transfer`; `WinClipboard::new` leaves it off), files are copied in both directions with the [MS-RDPECLIP] file stream protocol:

- Local files (`CF_HDROP`, with directories walked into relative paths, or `FileGroupDescriptorW` and `FileContents` from another application) are offered with `FileGroupDescriptorW`, and the remote's `FileContents` requests are answered from them, including from the snapshot a Lock PDU took.
- Remote files are put on the clipboard as an OLE data object offering `FileGroupDescriptorW` and one `FileContents` stream per file, so that Explorer can paste them.
  Each stream reads its file through `RANGE` requests carrying the `clipDataId` of the lock `Cliprdr` took when the remote advertised the files, so a paste keeps working after the remote clipboard changes, until that lock is released.
  This needs OLE, so it is only enabled when the thread is a single-threaded apartment.

The streams live in the process's multithreaded apartment, so the pasting application reads them on COM worker threads, never through the clipboard thread: a large paste does not stall the window, input or rendering.
Each stream keeps four 1 MiB `RANGE` requests in flight ahead of the reader.
Only fetching the file list, once per paste, waits on the clipboard thread, like rendering a delay-rendered format.
The embedder must route the file stream `ClipboardMessage`s to `Cliprdr` and drive `Cliprdr::drive_timeouts`.

Pure logic tests run with `cargo test -p ironrdp-testsuite-extra cliprdr_native::windows`.
Tests against the real clipboard, with a `CliprdrClient` over `WinClipboard` talking to an in-memory server and Explorer pasting the files, replace the clipboard contents and briefly open an Explorer window, so they only run on request:

```sh
IRONRDP_WIN_CLIPBOARD_LIVE=1 cargo test -p ironrdp-testsuite-extra cliprdr_native::windows_live -- --ignored --test-threads=1
```

[MS-RDPECLIP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/fb9b7e0b-6db4-41c2-b83c-f889c1ee7688

## Linux

The `data_control` module is a clipboard client for Wayland compositors that offer [`ext-data-control-v1`] or [`wlr-data-control-unstable-v1`].
It reads and sets the clipboard without a window and supports delayed rendering, so the data for a paste can be produced when the paste happens.

`LinuxClipboard` is a CLIPRDR backend built on that client.
When data-control is unavailable, including on GNOME, it falls back to X11/XWayland through XFixes and selection ownership.
A desktop without either clipboard service uses the client's stub backend.

Both backends react to selection changes and advertise remote formats without fetching their contents.
Text (`CF_UNICODETEXT`) and images (`CF_DIB`/`CF_DIBV5`, converted to/from PNG by `ironrdp-cliprdr-format`) are requested only when an application pastes.
Local clipboard content is read only when the peer requests it.
File clipboard transfer and HTML are not supported.

Only one Format Data Request is outstanding at a time, because responses do not identify their request.
A paste times out after five seconds, but its outstanding request is retained until the late response is drained.
If the peer never replies, new remote pastes fail until the clipboard channel is reinitialized; this prevents stale content from being mistaken for a newer copy.
Selection changes cancel older paste waiters, and the existing `ironrdp_cliprdr::loop_detector` detects content echoed by clipboard managers.

The clipboard workers bound data transfers and pending pastes.
X11 transfers use INCR for large content and separate requestor windows so late selection responses cannot complete newer reads.

State-machine tests run in the normal CI suite with `cargo test -p ironrdp-testsuite-extra cliprdr_native`.
Three additional integration tests require a dedicated X11 display; the interoperability test also requires `xclip`:

```sh
DISPLAY=:N cargo test -p ironrdp-testsuite-extra cliprdr_native::linux::x11_ -- --ignored --test-threads=1
```

Use a private display such as Xvfb for these tests, because they replace its clipboard selection.

[`ext-data-control-v1`]: https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-data-control/ext-data-control-v1.xml
[`wlr-data-control-unstable-v1`]: https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-data-control-unstable-v1.xml
