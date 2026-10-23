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
Writes acknowledge bounded local buffering; flush waits for the signed TS Proxy send operation and reports its failure.
The stream applies TS Proxy redirection restrictions before starting RDP device channels.

The live stream reserves a 1 GiB IN body.
It opens a successor IN channel before the default one runs out, accepts IN_R1 and IN_R2 replacement responses and follows OUT_R1 and OUT_R2 recycling when the proxy requests it, with the same HTTP authentication and certificate policy as the original channels.
Only RPC PDUs count against the flow-control windows; RTS PDUs do not.
Cookies set on any channel response are sent on later channel requests so a load balancer keeps the virtual connection on one proxy.
The IN channel response is watched for the whole session; an `RPC Error` status ends the stream with the gateway's error code.
Unexpected RTS sequences, a rejected replacement channel or more than 1 MiB of queued target data end the stream with an error, including while waiting for an RPC response.
It negotiates TS Proxy administrative messaging and keeps one `TsProxyMakeTunnelCall` pending to receive service, consent and reauthentication messages.
Consent messages are accepted and service messages are logged.
A reauthentication message is answered on a separate authenticated virtual connection that sends the reauthentication tunnel context and repeats user and resource authorization without opening another data channel.
The original data path stays open.
A failed message call disables messaging without ending the session.
It closes the TS Proxy channel, cancels the pending message call and closes the tunnel on graceful shutdown; an I/O or integrity error poisons the session, while dropping the stream closes its sockets without a close handshake.
There is no automatic HTTP/WebSocket-to-RPCH fallback.
A session through one Windows RD Gateway has been tested: it negotiated messaging, reached the remote desktop and disconnected cleanly.
The same gateway completed six IN_R2 replacements with a test build using a 128 KiB IN body, and separate connections verified cancellation of the pending message call during graceful shutdown.
A dedicated TCP transfer through the same gateway verified 1.25 GiB across OUT recycling and repeated server-requested reauthentication, with real consent and service message delivery.
Load-balanced affinity and deployments with different HTTP authentication policies remain unverified.

[MS-RPCH]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpch/c0f4c9c5-1a61-4d10-b8e2-005378d1d212

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
