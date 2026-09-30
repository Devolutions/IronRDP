# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations. Windows has a full backend; Linux has a Wayland clipboard client that a backend can be built on.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

## Linux

The `data_control` module is a clipboard client for Wayland compositors that
offer `ext-data-control-v1` or `wlr-data-control-unstable-v1`. It reads and sets
the clipboard without a window and supports delayed rendering, so the data for a
paste can be produced when the paste happens.
