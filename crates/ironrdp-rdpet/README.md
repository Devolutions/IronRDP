# ironrdp-rdpet

`ironrdp-rdpet` implements the RDP Telemetry Virtual Channel Extension defined by [MS-RDPET].
Over this dynamic virtual channel, a client reports how long it took to reach a fully functional connection: when credentials were prompted for and supplied, when the graphics pipeline channel opened, and when the first graphics message arrived.

The crate provides the `RDP_TELEMETRY_PDU` codec, a client processor that sends a caller-supplied report on the open channel, and a server processor that hands received reports to a handler.
It performs no I/O and reads no clock: the integrating client measures the milestones and decides when to send the single report per connection.

[MS-RDPET]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/
