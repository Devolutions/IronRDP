# IronRDP Daemon

Reusable persistent RDP-session support for local RPC hosts. It owns the daemon lifecycle,
retained framebuffer, input and screenshot handling, session log buffer, NOW endpoint, and durable
NOW operation manager. The local RPC schema and transport are provided directly by
[`ironrdp-rpc`](https://docs.rs/ironrdp-rpc).

## Download directory

Set `DaemonOptions::with_rdpdr_download_dir(Some(directory))` to announce the download-only filesystem device `DL`.
The directory must already exist and have an absolute path; on macOS and Linux it must also be UTF-8.
Windows accepts drive-letter paths without reparse points, not UNC or verbatim paths.
The daemon pins the directory at startup; session property `ironrdp_rdpdr:i:0` disables the announcement.
Only new flat files can be written and published without replacement.
The backend attempts to remove unpublished files on close, reset, or disconnect and logs cleanup failures.
On Windows, the startup option cannot be combined with unrestricted drives or smartcard redirection.
Connections exposing `DL` also reject other drive, smartcard, printer, or port redirection.
See the [native backend policy][download-policy] for limits and local-access requirements.

## Windows RDPDR options

`DaemonOptions` can configure fixed redirected volumes (`with_rdpdr_drives`) and WinSCard smartcard
redirection (`with_smartcard`).

Smartcard is also honored from connect/overlay property `ironrdp_smartcard`:

- startup `--smartcard` / `with_smartcard(true)` / overlay `ironrdp_smartcard:i:1` sets the default
- connect-time `ironrdp_smartcard:i:1` can enable smartcard-only without a startup flag
- connect-time `ironrdp_smartcard:i:0` disables smartcard for that session

Connect properties `drivestoredirect` and `redirectdrives` select drive letters with the mstsc
syntax, as in `.rdp` files. With `*` or `DynamicDrives`, the session also announces drives
connected while it runs. Configured volumes use device IDs from `27`, because `1` to `26` belong to
drive letters.
A configured volume named like a drive letter (`C` or `C:`) cannot be combined with redirected drive
letters, because both would show the same name in the session.

Smartcard-only sessions use an empty drive list with device ID `0` reserved for the smartcard device.

[download-policy]: ../ironrdp-rdpdr-native/README.md
