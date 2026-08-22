# IronRDP MS-TSGU

[Terminal Services Gateway Server Protocol][MS-TSGU] implementation for IronRDP.

This crate
- implements an MVP state needed to connect through Microsoft RD Gateway,
- supports HTTPS WebSocket transport and falls back to the legacy dual-channel HTTP transport,
- provides internal raw RPCH HTTP framing codecs, but no live RPC-over-HTTP gateway transport,
- decodes HTTP control packets (`HTTP_SERVICE_MESSAGE`, `HTTP_REAUTH_MESSAGE`, and `HTTP_CLOSE_PACKET`) without performing mid-session reauthentication,
- exposes decoded tunnel-authorization policy values without enforcing redirection rules or idle timeouts,
- accepts gateway consent messages by default,
- lets applications inspect and accept or decline a consent message synchronously through [`GwClient::connect_with_consent`] or [`GwClient::connect_with_port_and_consent`],
- does not implement reconnection, Detect, or a live UDP/DTLS side channel,
- authenticates gateway setup with HTTP Negotiate (Kerberos then NTLM), NTLM, or Basic fallback,
- can use Kerberos PKINIT with application-supplied UPN `smartcard` credentials for HTTP Negotiate authentication only,
- does not implement `HTTP_EXTENDED_AUTH_SC`, PAA, or a credential-provider UI,
- encodes and decodes RDG-UDP PDUs (`CONNECT_PKT`, `DATA_PKT`, `DISC_PKT`, and correlation info) without opening that side channel,
- encodes and decodes DCE/RPC common-header fragments and reassembles responses, without a live RPC-over-HTTP transport, and
- finishes write-side shutdown by closing the outbound WebSocket or ending the dual HTTP IN request body.

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
