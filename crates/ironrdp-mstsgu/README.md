# IronRDP MS-TSGU

[Terminal Services Gateway Server Protocol][MS-TSGU] implementation for IronRDP.

## Supported path

This crate implements the client side of the common modern RD Gateway path:

- HTTPS transport:
  - duplex **WebSocket** after `RDG_OUT_DATA` upgrade when the gateway returns `101 Switching Protocols`
  - legacy **dual-channel HTTP** (`RDG_OUT_DATA` response body + `RDG_IN_DATA` request body) when the gateway returns `200 OK` without upgrade ([MS-TSGU] 3.3.5.1)
- Endpoint parsing: bare host defaults to port `443`; bracketed IPv6 is supported
- TLS certificate policy via [`GwClient::connect_with_certificate_validation`] (same
  [`ironrdp_tls::CertificateValidation`] / callback surface as direct RDP)
- HTTP authentication (challenge-first):
  - **Negotiate** SPNEGO prefers Kerberos when a KDC is available (`SSPI_KDC_URL`, system config, or DNS), otherwise NTLM
  - **NTLM** scheme when only NTLM is advertised
  - optional `HTTP/<host>` SPN on both paths
  - **Basic** fallback when neither Negotiate nor NTLM is offered
  - **Smart card** ([MS-TSGU] 2.2.5.3.10 SMARTCARD): Negotiate with Kerberos PKINIT via [`GwConnectTarget::smart_card`] (feature `smartcard`; emulated cards and Windows card readers)
- Session cookies are replayed across the WebSocket/dual-HTTP authentication and IN/OUT request legs for load-balanced gateways
- HTTP CONNECT and SOCKS5 proxies are selected from `HTTPS_PROXY` / `https_proxy` and bypassed by `NO_PROXY` / `no_proxy`
- Handshake may advertise and complete **SSPI NTLM extended auth** when the gateway selects it ([MS-TSGU] 3.3.5.3)
- Tunnel sequence: handshake → (optional extended auth) → tunnel create → tunnel auth → channel create → data/keepalive
- Tunnel caps advertised: consent sign, service message, idle timeout, reauth
- Tunnel auth response optional fields (device-redirection flags, idle timeout, SoH response) are parsed and exposed through [`GwClient::tunnel_policy`]
- Channel response may carry UDP port + auth cookie metadata; [`GwUdpOffer`] / [`CONNECT_PKT`] helpers encode the RDG-UDP connect framing (live DTLS side channel not opened yet)
- Consent text is logged and auto-accepted by default; [`GwConnectOptions::consent_callback`] lets products present and decide the prompt
- Service messages are logged; channel-close ends the stream
- Mid-session **reauthentication**: on `HTTP_REAUTH_MESSAGE` a secondary connection repeats the setup sequence (handshake → tunnel create with the reauth tunnel context → tunnel auth → channel create) and the data path switches over ([MS-TSGU] 4.1.3)
- Common gateway HRESULTs are labeled in [`GwErrorKind::GatewayCode`] display text

The target resource **hostname and port** are taken from [`GwConnectTarget`] and sent in the
MS-TSGU channel-create packet (`HTTP_CHANNEL_PACKET`). Callers should pass the real destination
port (for example `3389` for ordinary RDP, or `2179` for Hyper-V VMConnect).

Gateway failures report a stage-oriented context string and, when available, an HTTP status or
gateway protocol/HRESULT code via [`GwErrorKind`].

Product clients may implement `GatewayUsageMethod::Detect` (try direct TCP, then this crate) outside
of `ironrdp-mstsgu`; this crate itself always opens a gateway tunnel.

## Not yet supported

- HTTP-RPC transport is implemented end to end (IN/OUT channels with the 100-Continue gate, RTS CONN setup, DCE/RPC bind with optional NTLM packet integrity, TsProxy tunnel/channel calls, receive-pipe data plane, flow control, ping scheduling, and administrative messages) but not wired as a fallback until validated against a real RD Gateway
- Live RDG-UDP / DTLS side channel open (PDU helpers only; needs MS-RDPEUDP consumer)
- PAA (pluggable authentication and authorization) cookies
- `HTTP_EXTENDED_AUTH_SC` CredSSP blob exchange (smart card here uses HTTP Negotiate PKINIT instead, matching FreeRDP)
- Kerberos without discoverable KDC (falls back to NTLM inside Negotiate)
- Interactive consent UI in product clients (the `consent_callback` surface exists; IronRDP's own clients auto-accept)
- Recoverable reconnect after transport failure

Do not confuse this crate with `ironrdp-rdcleanpath` (Devolutions Gateway / RDCleanPath).

[MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
