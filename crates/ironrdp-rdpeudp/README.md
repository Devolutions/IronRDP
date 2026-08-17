# IronRDP RDP-UDP

Reliable UDP transport implemented as described in [MS-RDPEUDP] and
[MS-RDPEUDP2].

The two documents divide the work. [MS-RDPEUDP] defines the handshake that
opens a connection and negotiates a protocol version; from version 3 onward
that handshake leads into the data transfer defined by [MS-RDPEUDP2], which is
the one this crate implements. Note that the documents take opposite byte
orders, and number the bits in their diagrams in opposite directions.

Sans-I/O: the state machine is driven by datagrams and by a caller-supplied
instant, and returns the datagrams it wants sent. It performs no I/O and reads
no clock, so it can be driven by any runtime.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEUDP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/
[MS-RDPEUDP2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp2/
