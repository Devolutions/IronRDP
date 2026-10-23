# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## [[0.0.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-mstsgu-v0.0.1...ironrdp-mstsgu-v0.0.2)] - 2026-10-23

### <!-- 1 -->Features

- Authenticate the gateway HTTP upgrade with Negotiate (Kerberos with NTLM fallback), NTLM or Basic, following the gateway's challenge ([#1715](https://github.com/Devolutions/IronRDP/issues/1715)) ([476b897129](https://github.com/Devolutions/IronRDP/commit/476b897129634c7f39d5679a80265258d506c1d3))

- [**breaking**] Add `GwConnectTarget::smart_card`, `GwSmartCardCredentials` and the `smartcard` feature for Kerberos PKINIT smart-card authentication over HTTP Negotiate ([#1741](https://github.com/Devolutions/IronRDP/issues/1741)) ([761dae12b0](https://github.com/Devolutions/IronRDP/commit/761dae12b0128be0f9bae52ca59eae8a0ba0b02f))

- Negotiate MS-TSGU NTLM extended authentication when the gateway requires it, reauthenticate the session without interrupting application data, and report the method through `GwClient::session_authentication` or, when unsupported, `GwErrorKind::UnsupportedExtendedAuthentication` ([#1802](https://github.com/Devolutions/IronRDP/issues/1802)) ([8649c7c69d](https://github.com/Devolutions/IronRDP/commit/8649c7c69db5279c2ab61116179c1cc3994aed81)) ([#1841](https://github.com/Devolutions/IronRDP/issues/1841)) ([c4533fee9c](https://github.com/Devolutions/IronRDP/commit/c4533fee9c45ce01ee456556857faca9e5e9c698))

- Add RDG-UDP PDU codecs (`ConnectPkt`, `ConnectPktResp`, `DataPkt`, `DiscPkt`, `UdpCorrelationInfo`) with the `fragment_connect_pkt` and `encode_connect_request` helpers, without opening a UDP side channel ([#1718](https://github.com/Devolutions/IronRDP/issues/1718)) ([5c6a28d2a5](https://github.com/Devolutions/IronRDP/commit/5c6a28d2a5e8ad38010998bbe2ce14f9a9f0687e))

- Add `GwTunnelPolicy` and `GwClient::tunnel_policy` to expose the redirection flags, idle timeout and SoH response from tunnel authorization, without enforcing them ([#1730](https://github.com/Devolutions/IronRDP/issues/1730)) ([6d4e78b1af](https://github.com/Devolutions/IronRDP/commit/6d4e78b1afe8a7895f1abd4cfa92f7d232ff7a6b))

- Fall back to the dual HTTP transport when the gateway does not upgrade to WebSocket ([#1752](https://github.com/Devolutions/IronRDP/issues/1752)) ([6e56bd73e4](https://github.com/Devolutions/IronRDP/commit/6e56bd73e45e50a7ef3484187309238ee0747c3a))

- Add `GwClient::connect_with_consent`, `GwClient::connect_with_port_and_consent` and `GwConsentCallback` to accept or decline a gateway consent message, which is otherwise accepted ([#1762](https://github.com/Devolutions/IronRDP/issues/1762)) ([d219ddbe16](https://github.com/Devolutions/IronRDP/commit/d219ddbe1664cf6d60d96a4bbb40d760d09e16c8))

- Report the HRESULT a gateway returns during control setup as `GwErrorKind::GatewayCode` ([#1774](https://github.com/Devolutions/IronRDP/issues/1774)) ([d4369f86db](https://github.com/Devolutions/IronRDP/commit/d4369f86db31a62e4d4457c68e7a9c1acbb391ee))

- Add `GwClient::connect_with_certificate_validation` and `GwClient::connect_with_port_and_certificate_validation` to apply a certificate-validation policy and callback to gateway HTTPS connections, while the other `connect` methods keep accepting any certificate ([#1775](https://github.com/Devolutions/IronRDP/issues/1775)) ([312d4466e7](https://github.com/Devolutions/IronRDP/commit/312d4466e7cae15d45c73a4ffcb4ae730d5e6a30))

- Route gateway HTTPS connections through the HTTP CONNECT or SOCKS5 proxy configured by `HTTPS_PROXY` and `NO_PROXY` ([#1776](https://github.com/Devolutions/IronRDP/issues/1776)) ([4222f49d07](https://github.com/Devolutions/IronRDP/commit/4222f49d07410c52b43d085f504e24b1acbb0bbd))

- Add `RpchStream` and `RpchClient`, an opt-in RPC-over-HTTP gateway transport with NTLM authentication and RPC packet integrity ([#2202](https://github.com/Devolutions/IronRDP/issues/2202)) ([57fa394306](https://github.com/Devolutions/IronRDP/commit/57fa39430616e4d6a846dd6801861ef22818e20f))

### <!-- 4 -->Bug Fixes

- Report end-of-stream when the gateway closes the tunnel, instead of panicking or hanging on the next read ([#1709](https://github.com/Devolutions/IronRDP/issues/1709)) ([8bd50100f5](https://github.com/Devolutions/IronRDP/commit/8bd50100f51d8fe2195e0bd4f2fd3d4f1f4bdfb6))

- Forward target ports other than 3389, such as 2179 for Hyper-V VMConnect, through the new `GwClient::connect_with_port`; `GwClient::connect` keeps presenting port 3389 ([#1710](https://github.com/Devolutions/IronRDP/issues/1710)) ([f43966cade](https://github.com/Devolutions/IronRDP/commit/f43966cadeb460b3bb02625532143d45447ca14a))

- Answer a gateway channel close and end the tunnel instead of ignoring the packet ([#1723](https://github.com/Devolutions/IronRDP/issues/1723)) ([e583e3a1bb](https://github.com/Devolutions/IronRDP/commit/e583e3a1bb59b76624b1c25d7155890b29b0f14f))

- Let `GwClient` write-side shutdown complete even when inbound delivery is blocked ([#1717](https://github.com/Devolutions/IronRDP/issues/1717)) ([01aa50bedf](https://github.com/Devolutions/IronRDP/commit/01aa50bedf60b73ed1c681989f399e81b4469b49))

- Connect to port 443 when the gateway endpoint has no port ([#1823](https://github.com/Devolutions/IronRDP/issues/1823)) ([29c96f221a](https://github.com/Devolutions/IronRDP/commit/29c96f221afeb6e57b18a87d70c9544243d2ff2d))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-tls` public dependency to 0.3

## [[0.0.1](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-mstsgu-v0.0.1)] - 2026-07-10

Initial release.
