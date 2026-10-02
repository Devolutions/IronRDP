# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations: Windows (`WinClipboard`) and Linux desktops over X11 and Wayland (`LinuxClipboard`, text and images).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

## Linux

`LinuxClipboard` uses the shared `data_control` client for Wayland compositors offering `ext-data-control-v1` or `wlr-data-control-unstable-v1`.
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
