# IronRDP MS-TSGU

[Terminal Services Gateway Server Protocol][MS-TSGU] implementation for IronRDP.

This crate
- implements an MVP state needed to connect through Microsoft RD Gateway,
- only supports the HTTPS protocol with WebSocket (and not the legacy HTTP or HTTP-RPC transports),
- decodes HTTP control packets (`HTTP_SERVICE_MESSAGE`, `HTTP_REAUTH_MESSAGE`, and `HTTP_CLOSE_PACKET`) without performing mid-session reauthentication,
- does not implement reconnection, Detect, or a live UDP/DTLS side channel,
- authenticates the WebSocket upgrade with HTTP Negotiate (Kerberos then NTLM), NTLM, or Basic fallback,
- encodes and decodes RDG-UDP PDUs (`CONNECT_PKT`, `DATA_PKT`, `DISC_PKT`, and correlation info) without opening that side channel, and
- finishes write-side shutdown by closing the outbound WebSocket and waiting for the gateway worker.

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
