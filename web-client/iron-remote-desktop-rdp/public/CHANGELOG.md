# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.7.0] - 2026-05-26

### <!-- 1 -->Features

- [**breaking**] Extend `DeviceEvent.wheelRotations` event to support passing rotation units other than pixels ([#952](https://github.com/Devolutions/IronRDP/issues/952)) ([23c0cc2c36](https://github.com/Devolutions/IronRDP/commit/23c0cc2c365159d24330a89ec4015121b67bccb6))

- Human-readable descriptions for RDCleanPath errors ([#999](https://github.com/Devolutions/IronRDP/issues/999)) ([18c81ed5d8](https://github.com/Devolutions/IronRDP/commit/18c81ed5d8d3bf13b3d10fe15209233c0c10bb62))

  Web-side error strings for RDCleanPath general/negotiation
  failures, including HTTP, WSA, and TLS error conditions.

- Configurable `alternate_shell` and `work_dir` ([#1095](https://github.com/Devolutions/IronRDP/issues/1095)) ([a33d27fe67](https://github.com/Devolutions/IronRDP/commit/a33d27fe6771a5a155161ef40a04de88803dd84c))

  Expose `ClientInfoPdu` `alternate_shell` and `work_dir` fields for
  RemoteApp, custom shells, and PSM session tokens.

- Negotiate bulk compression with the server ([ebf5da5f33](https://github.com/Devolutions/IronRDP/commit/ebf5da5f3380a3355f6c95814d669f8190425ded))

  Advertise compression in Client Info and decode compressed
  FastPath and ShareData updates (MPPC/NCRUSH/XCRUSH).

- Decode multitransport request PDUs ([#1092](https://github.com/Devolutions/IronRDP/issues/1092), [#1096](https://github.com/Devolutions/IronRDP/issues/1096)) ([4f5fdd3628](https://github.com/Devolutions/IronRDP/commit/4f5fdd3628f4d0d2c2a4116e4e45269d802740f1))

  Advertise the multitransport channel in GCC blocks and dispatch
  `MultitransportRequestPdu` from the IO channel. The web client
  logs the request; UDP transport is not yet wired up.

- Expose granular RDCleanPath error details ([#1117](https://github.com/Devolutions/IronRDP/issues/1117)) ([2911124e8f](https://github.com/Devolutions/IronRDP/commit/2911124e8fe6160bc8ba03a574b67077e6d2cca9))

  Forward HTTP status, WSA, and TLS alert codes from RDCleanPath
  errors so the web client can distinguish specific network
  failures.

- Clipboard file transfer support ([#1064](https://github.com/Devolutions/IronRDP/issues/1064), [#1065](https://github.com/Devolutions/IronRDP/issues/1065), [#1066](https://github.com/Devolutions/IronRDP/issues/1066), [#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9))

  End-to-end clipboard file transfer (upload and download) across
  the CLIPRDR channel per MS-RDPECLIP.

- Web RDPDR virtual printer support ([#1230](https://github.com/Devolutions/IronRDP/issues/1230)) ([14b1cef9cb](https://github.com/Devolutions/IronRDP/commit/14b1cef9cbbd0d8ef5e1fc8c73a3003a5e9f9bc2))

  Announce a redirected printer over RDPDR, receive server print
  jobs, and deliver completed PostScript jobs to a browser
  callback.

### <!-- 4 -->Bug Fixes

- Fix `this.lastSentClipboardData` being nulled ([#992](https://github.com/Devolutions/IronRDP/issues/992)) ([6127e13c83](https://github.com/Devolutions/IronRDP/commit/6127e13c836d06764d483b6b55188fd23a4314a2))

- Handle Auto-Detect Request PDUs from the server ([#1178](https://github.com/Devolutions/IronRDP/issues/1178)) ([4dcad09980](https://github.com/Devolutions/IronRDP/commit/4dcad09980e4f5354e4e435a134cc0956e2fcf9e))

  Fix a session-terminating "unhandled PDU: Auto-Detect Request
  PDU" error when servers send auto-detect requests during the
  active phase.

- Propagate negotiated `share_id` to all outgoing `ShareDataPdu` ([#1147](https://github.com/Devolutions/IronRDP/issues/1147)) ([2b24e9664d](https://github.com/Devolutions/IronRDP/commit/2b24e9664dd05620ff63a24d092377477fdde863))

### <!-- 7 -->Build

- Upgrade sspi and fix NTLM fallback ([#1188](https://github.com/Devolutions/IronRDP/issues/1188)) ([c70d38a9f1](https://github.com/Devolutions/IronRDP/commit/c70d38a9f190d6ad6c84bd9027a388b5db3296ba))
