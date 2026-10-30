# IronRDP RDPEPNP

`ironrdp-rdpepnp` implements the client side of the Remote Desktop Protocol: Plug and Play Devices Virtual Channel Extension ([MS-RDPEPNP]).
It provides the PDU codecs of both subprotocols and the client processors for their dynamic channels: `PNPDR`, which announces the redirected devices, and `FileRedirectorChannel`, which the server opens once per file handle to carry that handle's I/O.

The crate performs no device I/O.
An embedder describes the devices to redirect with `DeviceDescription` values and implements `PnpBackend`, which opens a `PnpFile` per CreateFile request; this crate provides no native backend.
`PnpInfoClient` is registered as a pre-registered channel so a reopened `PNPDR` channel announces the devices again, and `PnpInfoClient::file_redirector_listener` creates the listener for the I/O channels.
CreateFile requests for devices the client has not announced are refused with `ERROR_FILE_NOT_FOUND`.

Read, Write, and IOControl requests are answered asynchronously.
Each request hands the backend a completion that sends exactly one reply from any thread, through the caller-supplied `IoUplink` callback, or in the return value of the processing call when the backend completes before it returns.
The uplink receives `UplinkMessages`; call `UplinkMessages::take_if_open` on the thread that runs the DRDYNVC channel right before writing them, so a reply queued before its channel closed never reaches another channel that reuses the ID.
A completion dropped without a result replies with `E_ABORT`, and replies larger than the request allows become `ERROR_INSUFFICIENT_BUFFER`.
The client advertises I/O version 6, so backends can send custom events when the server also supports them.

The specification answers a protocol violation on a `FileRedirectorChannel` instance, such as a duplicate request ID or an unknown function ID, by terminating that channel.
Dynamic channel processors cannot close their channel, so the instance is disabled instead: its file is closed, its pending replies are discarded, and later messages are ignored.

[MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
