# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.3...ironrdp-dvc-com-plugin-v0.1.4)] - 2026-08-18

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



## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.2...ironrdp-dvc-com-plugin-v0.1.3)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.



## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.1...ironrdp-dvc-com-plugin-v0.1.2)] - 2026-06-05



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.0...ironrdp-dvc-com-plugin-v0.1.1)] - 2026-05-27

### <!-- 7 -->Build

- Update dependencies.
