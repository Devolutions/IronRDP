# IronRDP Daemon

Reusable persistent RDP-session support for local RPC hosts. It owns the daemon lifecycle,
retained framebuffer, input and screenshot handling, session log buffer, NOW endpoint, and durable
NOW operation manager. The local RPC schema and transport are provided directly by
[`ironrdp-rpc`](https://docs.rs/ironrdp-rpc).

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
