# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.3.0...ironrdp-cfg-v0.4.0)] - 2026-10-29

### <!-- 1 -->Features

- Support AVD gateway orchestration, WAM, and OAuth2 auth ([#2247](https://github.com/Devolutions/IronRDP/issues/2247)) ([c58f568711](https://github.com/Devolutions/IronRDP/commit/c58f568711ea30c719812a42607ac978c7513a45)) 

  Azure Virtual Desktop connections using `gatewaybrokeringtype:i:1`
  negotiate session placement and gateway routing through an HTTPS
  orchestration endpoint (`/api/v2/connections` or
  `/api/arm/v2/connections`) before upgrading to a WebSocket gateway
  transport and authenticating the target session host with Entra RDS AAD
  Proof-of-Possession tokens.
  
  Add AVD gateway orchestration, `WWW-Authenticate: Claims:...` challenge
  parsing, Windows Web Account Manager (`WebAuthenticationCoreManager`)
  silent and interactive token acquisition, and a cross-platform
  `OAuthAvdTokenProvider` supporting silent refresh-token exchange,
  loopback PKCE browser login, OAuth2 device-code fallback, persistent
  per-user refresh-token caching, and refresh-to-PoP host token minting.
  Decode `redirectedServerCert` and `redirectedAuthBlob` payloads, wire
  Shortpath UDP peer endpoints (`GatewayConnectionInfo::udp_peer`),
  preserve `load_balance_info` and `target_certificate` across mid-session
  AVD redirections, and add companion `<rdp_file>.json` `UserName` lookup
  and `--prop KEY:TYPE:VALUE` overrides in `ironrdp-daemon`,
  `ironrdp-agent`, and `ironrdp-viewer`.

- Support Remote Credential Guard over RDPEAR ([#2256](https://github.com/Devolutions/IronRDP/issues/2256)) ([376eb8f566](https://github.com/Devolutions/IronRDP/commit/376eb8f56634e5c92d7137886e62ddeb6bbb280f)) 

  Remote Credential Guard redirects Kerberos and NTLMv2 operations over
  the `Microsoft::Windows::RDS::AuthRedirection` dynamic virtual channel
  (MS-RDPEAR) so TGT keys and password hashes remain client-owned instead
  of delegating credentials through CredSSP `TSTicketCreds`.
  
  Add `ironrdp-rdpear` with `no_std`/`alloc` outer envelope framing,
  strict ASN.1 DER `TSRemoteGuardInnerPacket` codecs, and portable NDR32
  type-serialization codecs for all 23 MS-RDPEAR Kerberos and NTLM
  request/response calls under both the published IDL profile and the
  Windows-native wire profile. Bound inbound static and dynamic channel
  reassembly before fragment retention via
  `SvcProcessor::max_message_size`, `DvcProcessor::max_message_size`, and
  `DrdynvcClient::with_max_transport_message_size`.
  
  Wire the opt-in `remote-guard` feature through `ironrdp-connector`, the
  async/blocking/Tokio drivers, `ironrdp-cfg`, `ironrdp-client`,
  `ironrdp-daemon`, and `ironrdp-agent` (`--remote-guard` and
  `ironrdp_remote_guard` with `Disabled`, `Negotiate`, `Ntlm`, and
  `Kerberos` modes) backed by `sspi` 0.23.1. RCG requires `HYBRID` or
  `HYBRID_EX` with `REDIRECTED_AUTHENTICATION_MODE_SUPPORTED`, strict TLS
  validation, and a completed CredSSP context handoff, and omits
  passwords, smart-card PINs, and auto-reconnect cookies from Client Info.



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.2.0...ironrdp-cfg-v0.3.0)] - 2026-10-27

### <!-- 1 -->Features

- Add `PropertySetExt::usb_devices_to_redirect`, `set_usb_devices_to_redirect` and `clear_usb_devices_to_redirect` for the `usbdevicestoredirect` property ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.1.0...ironrdp-cfg-v0.2.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Add the required `PropertySetExt` accessors for `ironrdp_named_pipe` and `ironrdp_sandbox_id` (`named_pipe`, `sandbox_id`) ([#1580](https://github.com/Devolutions/IronRDP/issues/1580)) ([39b020343d](https://github.com/Devolutions/IronRDP/commit/39b020343d962962bfbefc89939be64d5c716196))

- [**breaking**] Add the required `PropertySetExt` accessors for `remoteapplicationmode` (`remote_application_mode`) ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a))

- [**breaking**] Add the required `PropertySetExt` accessors for `audiocapturemode` (`audio_capture_mode`), typed as the new `AudioCaptureMode` with `UnknownAudioCaptureMode` for unrecognized values ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d))

- [**breaking**] Add the required `PropertySetExt` accessors for `redirectwebauthn` (`redirect_webauthn`) ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb))

- [**breaking**] Add the required `PropertySetExt` accessors for `ironrdp_vmconnect`, `ironrdp_vmconnect_basic` and `ironrdp_vmconnect_current_user` (`vmconnect_id`, `vmconnect_basic`, `vmconnect_current_user`) ([#1768](https://github.com/Devolutions/IronRDP/issues/1768)) ([cf9bdef764](https://github.com/Devolutions/IronRDP/commit/cf9bdef764da6b9aff33b436662441e1503dc93a))

- [**breaking**] Add the required `PropertySetExt` accessors for `redirectprinters` (`redirect_printers`) ([#2156](https://github.com/Devolutions/IronRDP/issues/2156)) ([408b86ea91](https://github.com/Devolutions/IronRDP/commit/408b86ea91dc6b953778278ee2ffbd99756688f5))

- [**breaking**] Add the required `PropertySetExt` accessors for `redirectcomports` (`redirect_com_ports`) ([#2187](https://github.com/Devolutions/IronRDP/issues/2187)) ([850839f475](https://github.com/Devolutions/IronRDP/commit/850839f4756f4efb486a47abb89a93b3b4694a40))

- [**breaking**] Add the required `PropertySetExt` accessors for `drivestoredirect` and `redirectdrives` (`drives_to_redirect`, `redirect_drives`), plus `drive_redirection` and `set_drive_redirection`, which resolve both keys into the new `DriveRedirection` (mstsc `drivestoredirect` syntax: `*`, `DynamicDrives`, drive letters) ([#2178](https://github.com/Devolutions/IronRDP/issues/2178)) ([1de38a2ee5](https://github.com/Devolutions/IronRDP/commit/1de38a2ee55346a577ec28f3706c81dd3f7e8316))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-cfg-v0.1.0)] - 2026-07-10

Initial release.
