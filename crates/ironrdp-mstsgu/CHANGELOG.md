# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- HTTP multi-leg authentication for the WebSocket upgrade: portable Negotiate SPNEGO (Kerberos with NTLM fallback via KDC discovery / `SSPI_KDC_URL`), pure NTLM when only NTLM is advertised, and Basic fallback.
- Native Windows SSPI for HTTP Negotiate/NTLM when the native TLS transport supplies endpoint binding.
- RDG-UDP PDU encode/decode helpers (`CONNECT_PKT`, `DATA_PKT`, `DISC_PKT`, and `UDP_CORRELATION_INFO`) without opening a live DTLS side channel.
- Encode and decode for HTTP control packets (`HTTP_SERVICE_MESSAGE`, `HTTP_REAUTH_MESSAGE`, and `HTTP_CLOSE_PACKET`) without performing mid-session reauthentication.
- DCE/RPC common-header fragment codecs and response reassembly, without a live RPC-over-HTTP transport.
- NTLM-authenticated DCE/RPC bind, bind_ack, and rpc_auth_3 association codecs without a live RPC-over-HTTP transport or authenticated request/response traffic.
- Decode and expose tunnel-authorization policy fields (`redirFlags`, `idleTimeout`, and `SoHResponse`) without enforcing redirection restrictions or idle timeouts.
- Decode gateway consent messages and let callers accept or decline them before tunnel authorization.

## [[0.0.1](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-mstsgu-v0.0.1)] - 2026-07-10

Initial release.
