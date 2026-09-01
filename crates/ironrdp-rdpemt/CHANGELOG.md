# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpemt-v0.1.0)] - 2026-09-01

### <!-- 0 -->Security

- Add the RDPEMT multitransport tunnel crate ([#1626](https://github.com/Devolutions/IronRDP/issues/1626)) ([f3b770e9c1](https://github.com/Devolutions/IronRDP/commit/f3b770e9c1582e5c87d6b258319ccaf343bcb339)) 

  # feat(rdpemt): add the RDPEMT multitransport tunnel crate
  
  First of six. Adds `ironrdp-rdpemt`, the tunnel described by [MS-RDPEMT]
  that
  carries dynamic virtual channel traffic over a sideband connection.
  
  This is the smallest and most self-contained piece of the RDP-UDP work
  @mamoreau-devolutions asked for on #140, and it stands alone: nothing
  here
  depends on the UDP transport, and the tunnel is driven by decoded PDUs
  rather
  than by a socket.
  
  ## What is here
  
  **PDUs.** The tunnel create request and response, the data PDU with its
  subheader chain, and the Initiate Multitransport request and response
  from
  [MS-RDPBCGR] 2.2.15 that begin the exchange.
  
  **Tunnel state machine.** Both sides of the handshake: a client that
  sends the
  create request and waits for a response, a server that checks the
  security
  cookie it receives against the one it issued, and the established state
  where
  both sides simply carry data. Sans-I/O, so it takes PDUs in and produces
  events
  out; `no_std` with an `alloc` dependency.
  
  ## Notes for review

### <!-- 1 -->Features

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


