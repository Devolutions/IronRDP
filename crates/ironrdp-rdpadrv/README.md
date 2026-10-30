# ironrdp-rdpadrv

`ironrdp-rdpadrv` implements the Audio Level and Drive Letter Persistence Virtual Channel Extension defined by [MS-RDPADRV].
Windows MultiPoint session hosts use its two dynamic channels, `WMSAud` and `WMSDL`, to let a client remember the session's master volume levels and USB drive letter assignments, and restore them when it connects again.

The crate provides the message codecs and a client and a server processor for each channel.
The client processors keep the last values the server reported, answer the server's initialization messages with them, and report each change through a handler so the integrating client can store it.
The server processors ask the client for its stored values when the channel opens and report what the client sends through a handler.

The crate performs no I/O and does not touch the local audio devices or drive letters: storing the values across client restarts belongs to the integrating client.

[MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
