# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.1](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.8.0...iron-remote-desktop-v0.8.1)] - 2026-10-27

### <!-- 1 -->Features

- Add reason error codes ([#2127](https://github.com/Devolutions/IronRDP/issues/2127)) ([0c7e3abfb3](https://github.com/Devolutions/IronRDP/commit/0c7e3abfb3fbbef0027ce74d50c6038c245978e0)) 

  An RDCleanPath error PDU carries a general error code next to an HTTP
  status, WSA error or TLS alert, so a client cannot tell why a proxy
  rejected a connection: an expired credential and a misconfigured one
  both arrive as HTTP 401.
  
  Split the open `error_code` space into ranges, specified in the crate
  README:
  
  - 100 to 16383: standard reasons, for conditions any client can explain
  without knowing which proxy it talks to. This adds `EXPIRED_ERROR_CODE`,
  `NOT_YET_VALID_ERROR_CODE`, `REVOKED_ERROR_CODE`,
  `TARGET_UNREACHABLE_ERROR_CODE`, `DNS_RESOLUTION_FAILED_ERROR_CODE` and
  `TLS_HANDSHAKE_FAILED_ERROR_CODE` (100 to 105), which `Display`
  describes.
  - 16384 to 32767 (`IMPLEMENTATION_SPECIFIC_ERROR_CODES`): reasons each
  proxy defines for checks tied to its own credential format or topology.
  IronRDP never assigns these, and a client interprets them only when it
  knows which proxy it talks to.
  - 32768 and above stay reserved, so that independent implementations
  storing the code as a signed 16-bit integer can represent every code in
  use.
  
  The web client exposes the code as `RDCleanPathDetails.errorCode`, and
  `@devolutions/iron-remote-desktop-rdp` exports the standard codes as
  `RDCleanPathErrorCode`.
  
  The reason travels in the existing `error_code` field, not in a new one.
  A new error field would follow the last one, `[3]`, and IronRDP-based
  peers decode RDCleanPath with der's derived decoder, which rejects
  unknown fields at the end of a SEQUENCE with `TrailingData` despite the
  context-specific tags: deployed IronRDP clients would fail to decode
  error PDUs carrying it. Deployed IronRDP clients decode any code, show
  an unknown one as an unknown error, and still read the HTTP status, WSA
  error and TLS alert, which proxies keep sending alongside the reason.
  The native client and the FFI already treat any code other than the
  negotiation one as a general error, so they need no change.
  
  Proxies can send these codes with any version, since `error_code` is a
  plain integer. The constants and the web client's `errorCode` need
  releases of ironrdp-rdcleanpath and the web packages; the Devolutions
  Gateway follow-up, which will use the standard codes and its own
  implementation-specific ones, needs the crates.io release. That release
  also carries the der 0.8 upgrade already on master, which is breaking
  because the crate re-exports `der`.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.7.1...iron-remote-desktop-v0.8.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Add the required `SessionBuilder::websocket_protocols` method, exposed to JavaScript as `webSocketProtocols`, to offer WebSocket subprotocols ([#1970](https://github.com/Devolutions/IronRDP/issues/1970)) ([860cf6ccb1](https://github.com/Devolutions/IronRDP/commit/860cf6ccb1359e15034cfbf402a308e3ad21b341))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))



## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.7.0...iron-remote-desktop-v0.7.1)] - 2026-05-27

### <!-- 1 -->Features

- Expose granular RDCleanPath error details ([#1117](https://github.com/Devolutions/IronRDP/issues/1117)) ([2911124e8f](https://github.com/Devolutions/IronRDP/commit/2911124e8fe6160bc8ba03a574b67077e6d2cca9)) 

  Add RDCleanPathDetails struct to provide detailed error information for
  RDCleanPath errors, including HTTP status codes, WSA error codes, and
  TLS alert codes.
  
  Allows the web client to distinguish between different types of network
  errors (say, WSAEACCES/10013) instead of showing a generic RDCleanpath
  error message.

- Implement clipboard file transfer support ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9)) 

  Add end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel per MS-RDPECLIP.

## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.6.0...iron-remote-desktop-v0.7.0)] - 2025-09-29

### <!-- 4 -->Bug Fixes

- [**breaking**] Changed onClipboardChanged to not consume the input (#992) ([6127e13c83](https://github.com/Devolutions/IronRDP/commit/6127e13c836d06764d483b6b55188fd23a4314a2)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.5.0...iron-remote-desktop-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- [**breaking**] Extend `DeviceEvent.wheelRotations` event to support passing rotation units other than pixels (#952) ([23c0cc2c36](https://github.com/Devolutions/IronRDP/commit/23c0cc2c365159d24330a89ec4015121b67bccb6)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.4.0...iron-remote-desktop-v0.5.0)] - 2025-08-29

### <!-- 4 -->Bug Fixes

- [**breaking**] Remove the `remote_received_format_list_callback` method from Session common API (#935) ([5b948e2161](https://github.com/Devolutions/IronRDP/commit/5b948e2161b08b13d32bdbb480b26c8fa44d42f7)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.3.0...iron-remote-desktop-v0.4.0)] - 2025-06-27

### <!-- 1 -->Features

- [**breaking**] Add `canvas_resized_callback` method to `SessionBuilder` trait (#842) ([f6285c5989](https://github.com/Devolutions/IronRDP/commit/f6285c598915c8afb07553c765648d85ac4140cb)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/iron-remote-desktop-v0.2.0...iron-remote-desktop-v0.3.0)] - 2025-06-03

### <!-- 4 -->Bug Fixes

- [**breaking**] Rename extension_call to invoke_extension (#803) ([f68cd06ac3](https://github.com/Devolutions/IronRDP/commit/f68cd06ac3705608e6f2ac6bde684d9ae906ea53)) 

