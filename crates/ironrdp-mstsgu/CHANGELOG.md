# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## [[0.0.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-mstsgu-v0.0.1...ironrdp-mstsgu-v0.0.2)] - 2026-09-01

### <!-- 0 -->Security

- Add RPC integrity framing ([#1839](https://github.com/Devolutions/IronRDP/issues/1839)) ([db6436eead](https://github.com/Devolutions/IronRDP/commit/db6436eead9aa6a028d3e6a7317919b3f61dec84)) 

  Provide structural packet-integrity DCE/RPC framing with explicit
  caller-owned signed regions and verifier validation.
  
  Reject unsupported authentication levels and clear terminal response
  state after failed decoding. Verifier construction, sequence ownership,
  and RPCH transport remain with a future security context.

### <!-- 1 -->Features

- Add Negotiate and NTLM HTTP gateway auth ([#1715](https://github.com/Devolutions/IronRDP/issues/1715)) ([476b897129](https://github.com/Devolutions/IronRDP/commit/476b897129634c7f39d5679a80265258d506c1d3)) 

  Corporate RD Gateways typically challenge with Negotiate or NTLM rather
  than Basic, so the WebSocket-only client could not complete the upgrade.
  
  The first upgrade request omits Authorization. On 401, WWW-Authenticate
  is parsed as an HTTP challenge list and Negotiate is preferred (Kerberos
  then NTLM inside SPNEGO via sspi network_client), then NTLM, then HTTP
  Basic. SSPI and KDC resolution run on spawn_blocking. A 101 may complete
  SSPI from a final GSS token when present. connect and connect_with_port
  signatures are unchanged.
  
  sspi 0.21 is an implementation-only dependency. HTTP auth tests live in
  tests/http_auth.rs so [lib] test = false can stay. xtask runs that
  target with native-tls. RPCH, UDP, reauth, Detect, extended-auth packet
  exchange, and TLS channel bindings remain out of scope.

- Add RDG-UDP PDU codecs ([#1718](https://github.com/Devolutions/IronRDP/issues/1718)) ([5c6a28d2a5](https://github.com/Devolutions/IronRDP/commit/5c6a28d2a5e8ad38010998bbe2ce14f9a9f0687e)) 

  Encode and decode MS-TSGU UDP framing (CONNECT_PKT, DATA_PKT, DISC_PKT,
  and UDP_CORRELATION_INFO) plus HTTP_CHANNEL_RESPONSE UDP offer metadata.
  
  A live DTLS side channel is not opened; these helpers only cover the
  packet layouts in [MS-TSGU] 2.2.5.4 and 2.2.11.

- Decode HTTP service, reauth, and channel-close packets ([#1723](https://github.com/Devolutions/IronRDP/issues/1723)) ([e583e3a1bb](https://github.com/Devolutions/IronRDP/commit/e583e3a1bb59b76624b1c25d7155890b29b0f14f)) 

  PktTy already named ChannelClose, ServiceMessage, and ReauthMessage
  without decoding those payloads.
  
  Add encode/decode for HTTP_SERVICE_MESSAGE, HTTP_REAUTH_MESSAGE, and
  HTTP_CLOSE_PACKET, plus common MS-TSGU HRESULT display names. The work
  loop logs service and reauth packets. A server channel-close is answered
  with PKT_TYPE_CLOSE_CHANNEL_RESPONSE, then the work task ends.
  Mid-session reauthentication is not performed.

- Add DCE/RPC fragment codecs ([#1725](https://github.com/Devolutions/IronRDP/issues/1725)) ([438b177645](https://github.com/Devolutions/IronRDP/commit/438b1776459e9cb813f0fad034472e1db3627b85)) 

  Master ironrdp-mstsgu is WebSocket + HTTP Negotiate/NTLM/Basic and has
  no DCE/RPC types. Add the common-header / fragmentation foundation used
  by a later RPC-over-HTTP transport: PDU errors, syntax version, fragment
  sizes, common header, stream framer, response reassembly, and fault
  parse.
  
  This does not add TsProxy NDR, RTS, NTLM packet integrity, or a live
  RPCH client.

- Extract PacketIo from the WebSocket gateway client ([#1717](https://github.com/Devolutions/IronRDP/issues/1717)) ([01aa50bedf](https://github.com/Devolutions/IronRDP/commit/01aa50bedf60b73ed1c681989f399e81b4469b49)) 

  Move the HTTPS WebSocket transport, auth upgrade loop, and byte
  read/write helpers out of lib.rs so handshake/tunnel/channel
  sequencing stays separate from I/O.
  
  GwClient write-side shutdown now closes the outbound sender and
  inbound receiver, then waits for the worker so a local EOF can
  finish even if inbound delivery is blocked. Local shutdown still
  does not send HTTP_CLOSE_PACKET; inbound channel-close handling
  from master is preserved.
  
  This does not include dual-HTTP, RPCH, proxies, or certificate
  policy APIs.

- Add RPC-over-HTTP request framing ([#1727](https://github.com/Devolutions/IronRDP/issues/1727)) ([77689c3439](https://github.com/Devolutions/IronRDP/commit/77689c343999a155ab34b1dba5fdbf7c87925897)) 

  Add internal codecs for raw RPCH HTTP request and response framing.
  
  They validate bounded request and response heads without enabling a live
  RPC-over-HTTP gateway transport.

- Decode tunnel authorization policy fields ([#1730](https://github.com/Devolutions/IronRDP/issues/1730)) ([6d4e78b1af](https://github.com/Devolutions/IronRDP/commit/6d4e78b1afe8a7895f1abd4cfa92f7d232ff7a6b)) 

  Decode optional gateway redirection flags, idle timeout, and SoH
  response.
  Expose them through GwClient without enforcing the reported policy.
  
  ---------

- Fall back to dual HTTP gateway transport ([#1752](https://github.com/Devolutions/IronRDP/issues/1752)) ([6e56bd73e4](https://github.com/Devolutions/IronRDP/commit/6e56bd73e45e50a7ef3484187309238ee0747c3a)) 

  Support authenticated dual HTTP fallback after an RDG_OUT_DATA 200.
  Retain WebSocket on 101 and replay gateway cookies across setup.

- Add smart-card gateway authentication ([#1741](https://github.com/Devolutions/IronRDP/issues/1741)) ([761dae12b0](https://github.com/Devolutions/IronRDP/commit/761dae12b0128be0f9bae52ca59eae8a0ba0b02f)) 

  Add an opt-in Kerberos PKINIT path for HTTP Negotiate gateway
  authentication while retaining the password Negotiate, NTLM, and Basic
  flows.
  
  The public credentials type accepts an application-supplied UPN, redacts
  credentials, and rejects unsupported smart-card feature or challenge
  combinations without exposing them.

- Encode reauth tunnel context ([#1759](https://github.com/Devolutions/IronRDP/issues/1759)) ([18e259cd10](https://github.com/Devolutions/IronRDP/commit/18e259cd108b1debe6598b93f760af5d527ad194)) 

  Encode an optional reauth tunnel context in tunnel-create requests while
  clearing its presence bit when no context is supplied.
  Requests without a context retain their existing wire format.

- Add TsProxy control stub codecs ([#1758](https://github.com/Devolutions/IronRDP/issues/1758)) ([44a3bbb674](https://github.com/Devolutions/IronRDP/commit/44a3bbb674468c3a4aaecd49a6dab494d0d0f004)) 

  Stage bounded NDR32 codecs for the initial TsProxy control sequence.
  Keep the codecs internal and transport-free.

- Handle gateway consent messages ([#1762](https://github.com/Devolutions/IronRDP/issues/1762)) ([d219ddbe16](https://github.com/Devolutions/IronRDP/commit/d219ddbe1664cf6d60d96a4bbb40d760d09e16c8)) 

  Decode gateway consent messages as UTF-16LE during tunnel creation.
  
  Accept consent by default, and let callbacks decline it before tunnel
  authorization and channel setup.

- Add extended-auth packet codecs ([#1761](https://github.com/Devolutions/IronRDP/issues/1761)) ([a14c150e75](https://github.com/Devolutions/IronRDP/commit/a14c150e75ef386657e9a34680bae114f41cf99e)) 

  Preserve advertised handshake extended-auth flags, including unknown
  bits, and encode/decode extended-auth packet blobs with exact lengths.
  
  Add wire vectors for flag combinations, packet round trips, malformed
  blobs, and u16 blob boundaries.

- Preserve gateway HRESULT errors ([#1774](https://github.com/Devolutions/IronRDP/issues/1774)) ([d4369f86db](https://github.com/Devolutions/IronRDP/commit/d4369f86db31a62e4d4457c68e7a9c1acbb391ee)) 

  Preserve nonzero gateway control HRESULTs for caller diagnostics.
  
  Known codes retain established labels; unknown codes remain lossless.

- Apply certificate validation to gateways ([#1775](https://github.com/Devolutions/IronRDP/issues/1775)) ([312d4466e7](https://github.com/Devolutions/IronRDP/commit/312d4466e7cae15d45c73a4ffcb4ae730d5e6a30)) 

  Apply the RDP client's certificate-validation policy and callback to
  every
  gateway HTTPS connection.
  
  Existing gateway callers retain their compatibility default.

- [**breaking**] Record byte offset on decode and encode error variants ([#1266](https://github.com/Devolutions/IronRDP/issues/1266)) ([a1f9189c30](https://github.com/Devolutions/IronRDP/commit/a1f9189c307516361a8faff6ecb7c1690b267998)) 

  ## Summary
  
  Records a byte offset on every `DecodeErrorKind` and `EncodeErrorKind`
  variant that can know one, so decode and encode errors surface the
  position in the input stream where the failure was detected. Reshaped
  twice after review; see "Review history" below if you reviewed an
  earlier shape.
  
  Contributes to the structured-fuzzing roadmap in #1120 by giving
  crash-replay analysis and Wireshark-style malformed-PDU reporting the
  byte-offset dimension that source `Location` ([#1262](https://github.com/Devolutions/IronRDP/issues/1262)) alone does not
  provide.
  
  ## API
  
  Variants that gain `offset: Option<usize>`:
  
  - `DecodeErrorKind::NotEnoughBytes { received, expected, offset }`
  - `DecodeErrorKind::InvalidField { field, reason, offset }`
  - `DecodeErrorKind::UnexpectedMessageType { got, offset }`
  - `DecodeErrorKind::UnsupportedVersion { got, offset }`
  - `DecodeErrorKind::UnsupportedValue { name, value, offset }`
  - `EncodeErrorKind` mirrors the same shape for the encode side

- [**breaking**] Populate decode/encode error offsets from cursor positions ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec)) 

  ## Summary
  
  The workspace sweep that follows #1266. Decode and encode error
  construction sites now pass the cursor, so the reported position is the
  byte the decoder or encoder actually stopped at.
  
  Stacked on #1266 and merges after it.
  
  ## What "no position" means here
  
  #1266 makes `offset` an `Option<usize>` where `None` means the error has
  no position in the input stream at all, rather than a position that
  happened to be unavailable. This PR is the other half of that: it walks
  the workspace and gives a real position to every site that has one, so
  the sites left reporting `None` are the ones that genuinely never had
  one.
  
  Those are constructors validating their arguments, integer conversions,
  cache lookups that missed, accessors on already-decoded structures, and
  the declared-size checks described below. They report nothing rather
  than byte zero, and that is now their permanent answer rather than a gap
  awaiting another sweep.
  
  There are no `at: 0` sites left anywhere in the workspace.
  
  ## The rule
  
  The position is attached where the cursor identifies the bytes being
  complained about. It is omitted where the complaint is about a size the
  peer declared, computed from data already consumed, because there the
  cursor points at a byte that is not the problem.

- Support outbound gateway proxies ([#1776](https://github.com/Devolutions/IronRDP/issues/1776)) ([4222f49d07](https://github.com/Devolutions/IronRDP/commit/4222f49d07410c52b43d085f504e24b1acbb0bbd)) 

  Route all MS-TSGU HTTPS legs through configured HTTP CONNECT or SOCKS5
  proxies while preserving gateway TLS validation and authentication.
  
  Honor HTTPS_PROXY/https_proxy and NO_PROXY/no_proxy with bounded CONNECT
  response handling and credential redaction.

- Authenticate gateway sessions ([#1802](https://github.com/Devolutions/IronRDP/issues/1802)) ([8649c7c69d](https://github.com/Devolutions/IronRDP/commit/8649c7c69db5279c2ab61116179c1cc3994aed81)) 

  Negotiate MS-TSGU SSPI NTLM authentication and reauthenticate the
  original session on a short-lived transport without interrupting
  application data.
  
  Reject unsupported smart-card and pluggable exchanges explicitly.

- Add RPCH v2 setup state ([#1825](https://github.com/Devolutions/IronRDP/issues/1825)) ([4b7ef885f4](https://github.com/Devolutions/IronRDP/commit/4b7ef885f4604105d4fccaf2259afb410577f2c5)) 

  Add codec-only RPCH v2 CONN setup, client ping scheduling, and flow
  control.
  
  Keep client ping scheduling distinct from the proxy-facing CONN/B1
  ClientKeepalive setting and acknowledge reclaimed half windows.
  
  HTTP transport and NTLM signing remain out of scope.

- Add RPCH v2 channel recycling ([#1831](https://github.com/Devolutions/IronRDP/issues/1831)) ([35152e662d](https://github.com/Devolutions/IronRDP/commit/35152e662d67d522d09b4bcbbfb70198daafed67)) 

  Add wire codecs and ordering validation for v2 R1 recycling.
  
  Retain canonical recycle state and reopen channels only after the final
  RTS action is sent. Keep transport execution separate, and use the typed
  output test channel required for workspace test compilation.

- Add unprotected RPC call framing ([#1829](https://github.com/Devolutions/IronRDP/issues/1829)) ([14e5999b7f](https://github.com/Devolutions/IronRDP/commit/14e5999b7f7c23ceebe41b15a75fb72c77c36e44)) 

  Add unprotected bind negotiation and request fragmentation.
  Use context-aware response and fault decoding with existing reassembly.
  Keep authentication, signing, and TsProxy payloads out of this layer.

- Add TsProxy RPC stub codecs ([#1828](https://github.com/Devolutions/IronRDP/issues/1828)) ([bc568e47ec](https://github.com/Devolutions/IronRDP/commit/bc568e47ec3a32e36d1b5922a2c849bd6b302104)) 

  Add internal NDR32 and raw-stub codecs for TsProxy control and channel
  calls without exposing transport or client APIs.

- Add NTLM RPC association setup ([#1838](https://github.com/Devolutions/IronRDP/issues/1838)) ([2b40f47be0](https://github.com/Devolutions/IronRDP/commit/2b40f47be0a4dcb8d916edee1aaf90db31a0ebe6)) 

  Add DCE/RPC NTLM association setup for RD Gateway, including
  header-signing negotiation.
  Keep its SSPI context independent from HTTP authentication contexts.
  
  This excludes live RPC-over-HTTP transport and protected RPC traffic.

- Stage RPCH session engine ([#1856](https://github.com/Devolutions/IronRDP/issues/1856)) ([910eb7cad1](https://github.com/Devolutions/IronRDP/commit/910eb7cad15438987da64549392e5ea9f88e6195)) 

  Compile the RPCH harness as an internal stream-based engine.
  
  Keep live RPC-over-HTTP wiring and signed requests unavailable.

### <!-- 4 -->Bug Fixes

- Report EOF and stop polling a completed work task ([#1709](https://github.com/Devolutions/IronRDP/issues/1709)) ([8bd50100f5](https://github.com/Devolutions/IronRDP/commit/8bd50100f51d8fe2195e0bd4f2fd3d4f1f4bdfb6)) 

  GwClient polled its background work JoinHandle on every read and write,
  which tokio panics on once the task completes, and it never reported
  end-of-stream when the gateway stream closed. A caller that holds a read
  across the connection lifetime (for example a bidirectional relay) hit
  `JoinHandle polled after completion` or hung forever instead of seeing
  EOF.
  
  Track work completion with a flag so the handle is polled at most once,
  and surface UnexpectedEof once the task has ended and no data remains.

- Forward the target host and port to the gateway ([#1710](https://github.com/Devolutions/IronRDP/issues/1710)) ([f43966cade](https://github.com/Devolutions/IronRDP/commit/f43966cadeb460b3bb02625532143d45447ca14a)) 

  The channel-create packet (HTTP_CHANNEL_PACKET) hardcoded port 3389, so
  non-3389 RDP targets and Hyper-V VMConnect (port 2179) could not be
  tunneled through an RD Gateway.

- Support RPCH IN authentication probes ([#1821](https://github.com/Devolutions/IronRDP/issues/1821)) ([1155896fad](https://github.com/Devolutions/IronRDP/commit/1155896fadc5788eef2cc92f0b26c6f4ead0973a)) 

  Allow raw RPCH IN framing to authenticate with a zero-length probe
  before committing the channel-lifetime request body.
  
  Require a declared 401 response body no larger than 16 KiB before
  draining it, so retries occur only on a reusable connection. The
  internal request wrapper exposes only the remaining-length and flush
  operations needed by this framing contract.
  
  Update daemon error-status tests to create the bounded output-event
  receiver that `consume_output` requires.

- Default gateway port to HTTPS ([#1823](https://github.com/Devolutions/IronRDP/issues/1823)) ([29c96f221a](https://github.com/Devolutions/IronRDP/commit/29c96f221afeb6e57b18a87d70c9544243d2ff2d)) 

  Accept host-only gateway endpoints as HTTPS authorities and connect to
  port 443. Explicit ports and bracketed IPv6 literals retain their
  normalized endpoints.

- Accept advertised extended auth ([#1841](https://github.com/Devolutions/IronRDP/issues/1841)) ([c4533fee9c](https://github.com/Devolutions/IronRDP/commit/c4533fee9c45ce01ee456556857faca9e5e9c698)) 

  Treat handshake ExtendedAuth bits as capabilities after HTTP
  authentication.
  
  Require SSPI_NTLM and run its exchange when selected at transport setup.



### Added

- HTTP multi-leg authentication for the WebSocket upgrade: Negotiate SPNEGO (Kerberos with NTLM fallback via KDC discovery / `SSPI_KDC_URL`), pure NTLM when only NTLM is advertised, and Basic fallback.
- RDG-UDP PDU encode/decode helpers (`CONNECT_PKT`, `DATA_PKT`, `DISC_PKT`, and `UDP_CORRELATION_INFO`) without opening a live DTLS side channel.
- Encode and decode for HTTP control packets (`HTTP_SERVICE_MESSAGE`, `HTTP_REAUTH_MESSAGE`, and `HTTP_CLOSE_PACKET`) without performing mid-session reauthentication.
- DCE/RPC common-header fragment codecs and response reassembly, without a live RPC-over-HTTP transport.
- NTLM-authenticated DCE/RPC bind, bind_ack, and rpc_auth_3 association codecs without a live RPC-over-HTTP transport or authenticated request/response traffic.
- Decode and expose tunnel-authorization policy fields (`redirFlags`, `idleTimeout`, and `SoHResponse`) without enforcing redirection restrictions or idle timeouts.
- Decode gateway consent messages and let callers accept or decline them before tunnel authorization.

## [[0.0.1](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-mstsgu-v0.0.1)] - 2026-07-10

Initial release.
