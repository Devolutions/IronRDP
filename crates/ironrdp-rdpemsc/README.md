# IronRDP RDPEMSC

`ironrdp-rdpemsc` implements the Remote Desktop Protocol: Mouse Cursor Virtual Channel Extension ([MS-RDPEMSC]).

Servers that support the `Microsoft::Windows::RDS::MouseCursor` dynamic channel send pointer shape and position updates on it instead of the [MS-RDPBCGR] fast-path and slow-path pointer updates.
The crate provides the channel's PDU codecs, `MouseCursorClient`, and `MouseCursorServer`.

`MouseCursorClient` advertises version 1 capabilities when the channel opens and queues each pointer update until the caller collects it with `take_updates`.
`PointerUpdate::as_pointer_update_data` converts an update to the equivalent `ironrdp-pdu` pointer update, so the caller applies it, and keeps the Pointer Image Cache, with the pointer handling it already uses for [MS-RDPBCGR] updates.

`MouseCursorServer` confirms version 1 capabilities; the server application then sends `MouseCursorPdu::PointerUpdate` messages on the channel.

The crate performs no I/O.
Malformed messages and messages sent in the wrong direction are logged and ignored.

[MS-RDPEMSC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
[MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
