# IronRDP CLIPRDR native backends

Native CLIPRDR backend implementations: Windows (`WinClipboard`) and Linux desktops over X11 and Wayland (`LinuxClipboard`, text and images).

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

## Linux

The `data_control` module is a clipboard client for Wayland compositors that offer [`ext-data-control-v1`] or [`wlr-data-control-unstable-v1`].
It reads and sets the clipboard without a window and supports delayed rendering, so the data for a paste can be produced when the paste happens.

[`ext-data-control-v1`]: https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-data-control/ext-data-control-v1.xml
[`wlr-data-control-unstable-v1`]: https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-data-control-unstable-v1.xml
