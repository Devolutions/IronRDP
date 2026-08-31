# IronRDP MS-TSGU

[Terminal Services Gateway Server Protocol][MS-TSGU] implementation for IronRDP.

This crate
- implements an MVP state needed to connect through Microsoft RD Gateway,
- supports HTTPS WebSocket transport and falls back to the legacy dual-channel HTTP transport,
- honors `HTTPS_PROXY`/`https_proxy` and `NO_PROXY`/`no_proxy` for outbound HTTP(S) CONNECT and SOCKS5 gateway proxies,
- provides raw RPCH HTTP framing, v2 setup and flow-control codecs, and a crate-internal transport-independent session engine, but no live RPC-over-HTTP gateway transport,
- processes `HTTP_REAUTH_MESSAGE` by opening a short-lived reauthentication transport that retains the active data transport,
- exposes decoded tunnel-authorization policy values without enforcing redirection rules or idle timeouts,
- accepts gateway consent messages by default,
- lets applications inspect and accept or decline a consent message synchronously through [`GwClient::connect_with_consent`] or [`GwClient::connect_with_port_and_consent`],
- does not implement reconnection, Detect, or a live UDP/DTLS side channel,
- authenticates gateway setup with HTTP Negotiate (Kerberos then NTLM), NTLM, Basic fallback, or MS-TSGU `SSPI_NTLM` extended authentication when negotiated,
- can use Kerberos PKINIT with application-supplied UPN `smartcard` credentials for HTTP Negotiate authentication only,
- does not implement `HTTP_EXTENDED_AUTH_SC` or PAA exchanges without a credential-provider UI,
- encodes and decodes RDG-UDP PDUs (`CONNECT_PKT`, `DATA_PKT`, `DISC_PKT`, and correlation info) without opening that side channel,
- encodes and decodes DCE/RPC fragments, including the NTLM bind/bind_ack/rpc_auth_3 association exchange, caller-owned packet-integrity trailer framing, and response reassembly, but no live RPC-over-HTTP transport or authentication-provider request/response processing, and
- finishes write-side shutdown by closing the outbound WebSocket or ending the dual HTTP IN request body.

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
