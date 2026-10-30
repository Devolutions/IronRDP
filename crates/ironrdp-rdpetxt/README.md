# IronRDP RDPETXT

`ironrdp-rdpetxt` implements the Remote Desktop Protocol: Text Input Virtual Channel Extension ([MS-RDPETXT]).

Windows 11 and Windows Server 2025 hosts use it to connect input methods on the client, such as IMEs, to edit controls of remote applications.
The server sends on `TextInput_ServerToClientDVC` and the client sends on `TextInput_ClientToServerDVC`.

The `pdu` module encodes and decodes every RDPTXT PDU and builds without `std`.
With the default `std` feature, `client` and `server` each provide a processor per channel around one shared state, because a reply never travels on the channel its request arrived on.
The session sends the PDUs that state queues through `pending_messages` on the processor of the sending channel.

`client::TextInputClient` answers the version exchange and the acknowledgements the specification asks of a client, tracks the focused remote edit control, and inserts committed text into it.
It hosts no input method, so it neither composes text nor undoes and replays pending key events after an update collision.
`server::TextInputServer` announces the server version and hands client PDUs to a `TextInputServerHandler`.

The crate performs no I/O.

[MS-RDPETXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
