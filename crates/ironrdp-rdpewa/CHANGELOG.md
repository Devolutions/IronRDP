# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpewa-v0.1.0)] - 2026-09-04

### <!-- 1 -->Features

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



### Added

- Initial MS-RDPEWA protocol crate with CBOR PDU codec, client processor, and server skeleton.
