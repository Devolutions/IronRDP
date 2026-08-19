# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## Unreleased

### Added

- [`GwConnectTarget::server_port`] forwards the destination resource port in the MS-TSGU channel-create packet (non-3389 RDP and VMConnect 2179).
- [`GwErrorKind::HttpStatus`] and [`GwErrorKind::GatewayCode`] surface HTTP upgrade status and gateway protocol/HRESULT codes (with common MS-TSGU HRESULT labels).
- HTTP multi-leg authentication for the WebSocket upgrade: Negotiate SPNEGO (Kerberos with NTLM fallback via KDC discovery / `SSPI_KDC_URL`), pure NTLM when only NTLM is advertised, Basic fallback; optional `HTTP/<host>` SPN.
- Post-handshake SSPI NTLM extended authentication when the gateway selects it ([MS-TSGU] 3.3.5.3).
- Broader tunnel capability advertisement (consent, service message, idle timeout, reauth).
- Service-message logging and channel-close handling.
- Mid-session reauthentication: `HTTP_REAUTH_MESSAGE` triggers a secondary connection repeating the setup sequence with the reauth tunnel context, then the data path switches over ([MS-TSGU] 4.1.3).
- Mock gateway harness (`#[cfg(test)]`) plus `Encode` impls for server-side packets, covering the full connection sequence and failure injection.
- `GwClient::tunnel_policy` exposes gateway-negotiated device redirection flags (`GwTunnelRedirFlags`) and idle timeout.
- `GwClient::connect_with_options` with `GwConnectOptions::consent_callback` lets products present the gateway consent message and decline the connection.
- Smart-card gateway authentication (feature `smartcard`): `GwConnectTarget::smart_card` authenticates the HTTP Negotiate scheme with Kerberos PKINIT via sspi smart-card identities (emulated or Windows card readers).
- MS-RPCH v2 client driver (not enabled as a fallback yet): IN/OUT channels over raw HTTP/1 with the 100-Continue IN-body gate, RTS CONN setup, DCE/RPC bind with optional NTLM packet integrity, TsProxy create/authorize/create-channel/setup-receive-pipe sequence, receive-pipe data plane, send/receive window flow control, ping scheduling, and administrative tunnel messages. Covered by an in-process mock RPC proxy test.
- Consent messages are logged and auto-accepted until an interactive UI path exists.
- Endpoint parsing defaults bare hosts to port 443 and supports bracketed IPv6.
- Unit tests for core MS-TSGU packet encode/decode layouts and HTTP auth helpers.
- Tunnel-auth optional fields: client SoH blob encode support; response redir flags, idle timeout, and SoH response decode.
- Channel response UDP port and auth cookie accessors (for a future RDG-UDP path).
- `GwClient::connect_with_certificate_validation` applies the same TLS certificate policy as direct RDP.
- Multi-stage handshake/tunnel/channel encode+decode sequence unit test.
- Dual-channel HTTP transport fallback when WebSocket upgrade is unavailable (`RDG_OUT_DATA` + `RDG_IN_DATA`, seed skip, chunked IN body).
- RDG-UDP packet helpers: [`GwUdpOffer`], [`CONNECT_PKT`] encode/decode, correlation info, and connect fragmentation (no live DTLS open yet).
- Session cookie replay across gateway HTTP request legs for load-balanced deployments.
- HTTP CONNECT and SOCKS5 proxy support via `HTTPS_PROXY` / `NO_PROXY`, including proxy URL credentials.

### Fixed

- `HTTP_EXTENDED_AUTH_NONE` is now `0x00` per [MS-TSGU] 2.2.5.3.2 (was incorrectly `0x01`).
- Handshake response fixed-part size now accounts for a 2-byte `extended_auth` field.
- Packet header fixed-part size comment/layout corrected to `u16 + u16 + u32`.

## [[0.0.1](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-mstsgu-v0.0.1)] - 2026-07-10

Initial release.
