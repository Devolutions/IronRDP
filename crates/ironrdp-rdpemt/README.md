# IronRDP RDPEMT

Multitransport tunnel implemented as described in [MS-RDPEMT], which carries
dynamic virtual channel traffic over a sideband connection once the main RDP
connection has asked for one via the Initiate Multitransport Request PDU
([MS-RDPBCGR] section 2.2.15).

Sans-I/O: the tunnel state machine consumes decoded PDUs and produces events,
leaving the transport and any encryption to the layer above.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/
[MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
