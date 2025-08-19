# IronRDP MS-TSGU

[Terminal Services Gateway Server Protocol][MS-TSGU] implementation for IronRDP.

This crate
- implements an MVP state needed to connect through Microsoft RD Gateway,
- only supports the HTTPS protocol with WebSocket (and not the legacy HTTP, HTTP-RPC or UDP protocols),
- does not implement reconnection/reauthentication, and
- only supports basic auth.

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
