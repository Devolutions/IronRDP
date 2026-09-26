# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations: Windows (`WinClipboard`) and Linux desktops over X11 and Wayland (`LinuxClipboard`, text and images).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

Linux clipboard sharing starts after the server requests its initial format list.
It polls for local text and image changes and fetches remote content asynchronously.
New local copies supersede outstanding remote responses, and a queued remote copy
can recover when its preceding request times out. File clipboard transfer and HTML
are not supported by the Linux backend.

Run the native text/image and clipboard state regression tests with
`cargo test -p ironrdp-cliprdr-native --lib`.
