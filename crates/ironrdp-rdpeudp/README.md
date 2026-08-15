# IronRDP RDP-UDP

Wire types for the [MS-RDPEUDP] version 1 handshake: the FEC header, SYN and SYN+ACK payloads, ACK vectors, and the correlation ID payload.

[MS-RDPEUDP] and [MS-RDPEUDP2] use opposite byte orders and number the bits in their diagrams in opposite directions.
Code and tests that touch both documents need to keep that straight.

This crate does not yet implement a connection state machine or the [MS-RDPEUDP2] data transfer that a negotiated version 3 connection uses.
Those land in a later crate.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEUDP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/
[MS-RDPEUDP2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp2/
