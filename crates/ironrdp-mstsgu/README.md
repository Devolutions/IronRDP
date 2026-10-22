# IronRDP MS-TSGU

[Terminal Services Gateway Server Protocol][MS-TSGU] implementation for IronRDP.

This crate
- implements an MVP state needed to connect through Microsoft RD Gateway,
- supports HTTPS WebSocket transport and falls back to the legacy dual-channel HTTP transport,
- honors `HTTPS_PROXY`/`https_proxy` and `NO_PROXY`/`no_proxy` for outbound HTTP(S) CONNECT and SOCKS5 gateway proxies,
- exposes an explicit, bounded HTTPS RPC-over-HTTP v2 channel through `RpchClient`, with caller-owned RPC authentication and packet integrity,
- processes `HTTP_REAUTH_MESSAGE` by opening a short-lived reauthentication transport that retains the active data transport,
- exposes decoded tunnel-authorization policy values without enforcing redirection rules or idle timeouts,
- accepts gateway consent messages by default,
- lets applications inspect and accept or decline a consent message synchronously through [`GwClient::connect_with_consent`] or [`GwClient::connect_with_port_and_consent`],
- does not implement reconnection, Detect, or a live UDP/DTLS side channel,
- authenticates gateway setup with HTTP Negotiate (Kerberos then NTLM), NTLM, Basic fallback, or MS-TSGU `SSPI_NTLM` extended authentication when negotiated,
- can use Kerberos PKINIT with application-supplied UPN `smartcard` credentials for HTTP Negotiate authentication only,
- does not implement `HTTP_EXTENDED_AUTH_SC` or PAA exchanges without a credential-provider UI,
- encodes and decodes RDG-UDP PDUs (`CONNECT_PKT`, `DATA_PKT`, `DISC_PKT`, and correlation info) without opening that side channel,
- encodes and decodes DCE/RPC fragments, including the NTLM bind/bind_ack/rpc_auth_3 association exchange, caller-owned packet-integrity trailer framing, and response reassembly, and
- finishes write-side shutdown by closing the outbound WebSocket or ending the dual HTTP IN request body.

## Bounded RPC-over-HTTP channel

`RpchClient::connect` opens HTTPS `RPC_IN_DATA` and `RPC_OUT_DATA` channels and completes the MS-RPCH v2 CONN sequence.
Enable the crate's `rustls` or `native-tls` feature to select a TLS backend.
Pass the RPC proxy backend authority, usually `gateway-host:3388`, separately from the destination RDP server.
The channel shares `GwConnectTarget` credentials, outbound proxy settings and certificate-policy semantics with `GwClient`; its default certificate policy is strict.
It preserves caller-supplied RPC authentication trailers, handles flow-control acknowledgements and peer pings, and exposes the negotiated timeout for caller-driven idle pings.

TLS and HTTP authentication do not establish RPC packet integrity.
`RpchClient` is a low-level channel: callers must authenticate the RPC association, sign requests, verify responses and perform the TS Proxy calls required by [MS-TSGU] 3.2.3.
`send_fragment` rejects request fragments without a packet-integrity trailer but cannot verify that the caller's security provider produced it.
`receive_fragment` returns unverified bytes, not trusted RPC responses or an RDP byte stream.
`RpcNtlmAuth` supplies NTLM token exchange and fragment-signing primitives, but the caller must run the bind/authentication sequence and maintain independent send and receive sequence numbers.

`RpchStream::connect_with_port_and_certificate_validation` provides the explicit authenticated path used by the client's `TransportKind::GatewayRpch` selection.
It performs the NTLM RPC association exchange, signs outgoing TS Proxy requests, verifies incoming responses, opens the TS Proxy tunnel and exposes it as an asynchronous RDP byte stream.
Its packet-integrity state is owned by the session and fail-closed; TLS certificate validation and HTTP channel authentication remain separate gateway security layers.
The stream applies TS Proxy redirection restrictions before starting RDP device channels.

The live stream uses a 128 KiB IN body for setup and traffic, and returns errors on exhausted send credit, unsupported RTS sequences, HTTP reauthentication, consent messages or channel lifetime/recycling transitions.
It closes the TS Proxy context and tunnel on graceful shutdown; an I/O or integrity error poisons the session, while dropping the stream closes its sockets without a close handshake.
The HTTP channel setup does not retain load-balancer cookies or monitor IN-channel error responses after setup.
There is no automatic HTTP/WebSocket-to-RPCH fallback, and interoperability with a real RD Gateway has not been verified.

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
