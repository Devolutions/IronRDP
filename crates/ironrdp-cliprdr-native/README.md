# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations: Windows (`WinClipboard`), Linux desktops over X11 and Wayland (`LinuxClipboard`, text and images), and macOS (`MacClipboard`, text, images, and files in both directions).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

## macOS

`MacClipboard` implements text, image, and file clipboard exchange in both
directions.

- Local text (`CF_UNICODETEXT`) and images (`CF_DIB`/`CF_DIBV5`, converted
  to/from PNG by `ironrdp-cliprdr-format`) are advertised with delayed
  rendering. The pasteboard is read only when the remote asks for the data.
- Local files are offered with `FileGroupDescriptorW`. Directories are walked
  into relative paths, and remote `FileContents` requests are answered from the
  current list or from the snapshot taken when the remote locked the list.
- Remote text and images are requested when the remote copy is announced and
  written to the pasteboard as plain text and PNG/TIFF.
- A remote file list is published to the macOS pasteboard as local file URLs,
  and the bytes are fetched only when a reader such as Finder opens the
  placeholder.
- Each remote file is a sparse file in a private `$TMPDIR/ironrdp-cliprdr`
  tree, so a multi-gigabyte file does not allocate its full size before the
  paste.
- An `NSFilePresenter` receives Finder's coordinated read. On the first read,
  the backend sends `RANGE` requests through
  `ClipboardMessage::SendFileContentsRequest` and writes the responses into the
  placeholder. Later reads use the completed file.
- `NSPasteboard`, presenter registration, and `MacClipboard` destruction are
  main-thread operations. Coordinated staging deletes run on a dedicated
  cleanup thread. Create the bridge and call `poll` from the embedder's main
  event loop:

  ```rust,ignore
  let mut clipboard = MacClipboard::new(message_proxy)?;
  let client = client.with_cliprdr_backend_factory(clipboard.backend_factory());

  // Each main-loop tick, after routing CLIPRDR events to the backend:
  clipboard.poll();
  if clipboard.needs_poll() {
      // Keep a short timer alive for Finder reads and remote format requests.
  }
  ```

  `poll` drives queued CLIPRDR callbacks, local clipboard changes, format
  requests, file-content requests, and active downloads. Finder's coordinated
  read blocks its own operation queue until the file completes or the transfer
  fails.
- Finder's presenter callback runs on a private operation queue and does not
  wake the UI event loop. Keep polling on a short timer while a remote file
  offer is on the pasteboard; a loop that only reacts to window events can
  leave a read waiting.
- A remote file descriptor must include a size. A file list without one is
  rejected because a sparse placeholder needs the final length.
- A file transfer with no response for 60 seconds fails. On clipboard change
  or normal close, incomplete placeholders are removed and completed
  placeholders are deleted with `NSFileCoordinator` so an active Finder copy
  finishes first. The clipboard is cleared on exit only if it still contains
  content this session published. Normal close waits up to five minutes for
  outstanding coordinated deletes; if that expires, or the process is
  force-terminated, the seven-day startup prune removes the remaining tree.
- Finder may show “Preparing to copy…” while a file read is in progress. The
  transfer is driven by Finder's read, so the dialog is not a byte-level
  progress indicator.
- HTML, custom progress UI, and a File Provider/FSKit/FUSE mount are not
  implemented.

This module is compile- and clippy-checked on macOS. The sparse-placeholder
and coordinated-read file path was live-validated separately in a prototype.
Text, images, and local-to-remote file upload still need a live test in an
embedder.

## Windows

`WinClipboard` runs on the thread that creates it, which must have a message loop (a GUI thread): a hidden window listens for clipboard changes and renders remote formats on demand.
Background hosts report clipboard changes without waiting for an activation message.
Replacing a deferred format request with a newer clipboard event sends a failed response instead of leaving the request pending.
Only formats whose data is a self-contained `HGLOBAL` are exchanged; handle-based formats (`CF_BITMAP`, `CF_ENHMETAFILE`, `CF_METAFILEPICT`, `CF_PALETTE`), local paths and shell items (`CF_HDROP`, `FileNameW`, `Shell IDList Array`) and OLE storage formats are never advertised in either direction.
Windows synthesizes `CF_DIB` from `CF_BITMAP`, so images still go through.
When Windows lists `CF_DIB` before `CF_DIBV5`, only `CF_DIB` is offered to avoid [Windows' corrupting DIB-to-DIBV5 conversion][dib-conversion-bug].
Images supplied as `CF_DIBV5` first remain available with their alpha channel.

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
[dib-conversion-bug]: https://techcommunity.microsoft.com/discussions/windows11/serious-bug-in-windows-clipboard---images-get-corrupted/4466584

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
