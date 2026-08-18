# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.1.0...ironrdp-cfg-v0.2.0)] - 2026-08-18

### <!-- 0 -->Security

- Connect to Windows Sandbox named pipes ([#1580](https://github.com/Devolutions/IronRDP/issues/1580)) ([39b020343d](https://github.com/Devolutions/IronRDP/commit/39b020343d962962bfbefc89939be64d5c716196)) 

  Windows Sandbox's default attach path is a local named pipe carrying
  plain TPKT/X.224 with PROTOCOL_RDP and ENCRYPTION_LEVEL_NONE, not
  TCP:3389 or VMConnect. Allow the connector and client to complete that
  sequence only via an explicit opt-in (`enable_standard_rdp_security`;
  NamedPipe enables it), and teach ironrdp-agent to resolve pipe path and
  guest credentials from WindowsSandboxServer after `wsb start`.
  
  Adds Transport::NamedPipe, ironrdp_named_pipe/ironrdp_sandbox_id
  properties, sandbox list/config/stop CLI helpers via an in-process
  h2/gRPC client on the per-user `\\.\pipe\wsandbox\{guid}` pipe (no .NET
  helper), and connect --sandbox-id / --sandbox-pipe. Sandbox-derived
  properties are the merge base; explicit .rdp/--prop/flags override them
  while NamedPipe TLS/CredSSP stay forced off. Local :2179+PCB remains
  unsupported.

### <!-- 1 -->Features

- Add RemoteApp channel support ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a)) 

  Configure and negotiate RAIL connections, then route its static channel
  through the portable client with bounded request queues and server
  control events.

- Wire MS-RDPEAI capture into Windows client and ActiveX ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d)) 

  Add the client MS-RDPEAI capture path on top of hardened RDPSND
  playback: connector CFG + static channel wiring, CPAL PCM capture
  backend, ironrdp-client --audio-capture, and ActiveX
  AudioCaptureRedirectionMode.
  
  PCM capture only accepts encode formats that match the Open capture
  stream, rejects non-16-bit capture (Data PDU size contract), and gates
  the capture backend behind ironrdp-rdpsnd-native/capture.
  
  Depends on #1648 (playback).

- Add native MS-RDPEWA WebAuthn redirection ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb)) 

  Implement the RDPEWA dynamic channel with a Windows WebAuthn backend and
  wire RedirectWebAuthn for ActiveX, the optional client feature, and the
  viewer CLI.
  
  Prefer System32\webauthn.dll via the DVC COM plugin for MSTSC parity.
  The pure-Rust backend forwards ceremonies through a webauthn.dll IWTS
  oneshot so hash-only hosts that omit clientDataJSON still work; public
  WebAuthN* remains a fallback when JSON is present. Recreate
  WebAuthN_Channel opens through shared COM/listener factories because
  Windows opens and closes the channel around each RPC.
  
  Side effects:
  - New crates ironrdp-rdpewa and ironrdp-rdpewa-native
  - Config key redirectwebauthn; ActiveX ExtendedSettings property
  - ironrdp-daemon webauthn feature for ironrdp-agent
  - IRONRDP_WEBAUTHN_FORCE_NATIVE debug switch
  - Viewer --webauthn/--no-webauthn flags; .rdp redirectwebauthn default
  - ActiveX docs note no AdvancedSettings slot and no IPersist persistence



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-cfg-v0.1.0)] - 2026-07-10

Initial release.
