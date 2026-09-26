# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations: Windows (`WinClipboard`) and Linux desktops over X11 and Wayland (`LinuxClipboard`, text and images).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

## Linux

The `data_control` module is a clipboard client for Wayland compositors that offer `ext-data-control-v1` or `wlr-data-control-unstable-v1`.
It supports clipboard change notifications and delayed rendering.
