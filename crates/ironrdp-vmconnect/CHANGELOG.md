# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-vmconnect-v0.1.0)] - 2026-08-13

### <!-- 1 -->Features

- Hyper-V vmconnect support ([#1503](https://github.com/Devolutions/IronRDP/issues/1503)) ([a7cc067d50](https://github.com/Devolutions/IronRDP/commit/a7cc067d5069cbbcb13bae3e0561c0611da3bcf6)) 

  Adds Hyper-V VMConnect's direct ordering: PCB → TLS → CredSSP → X.224.
  
  Enhanced Session is the default (`GUID;EnhancedMode=1`), with
  `--vmconnect-basic` for the synthetic console. Kept this separate in
  `ironrdp-vmconnect`; no SPN changes.
  
  Tested against the nested Hyper-V lab:
  - Enhanced: `HYBRID_EX`, rendered 1280×720
  - Basic: `HYBRID`, rendered 1280×720
  - `cargo xtask check fmt/lints/tests -v`
  
  ---------

- Support Hyper-V connection ordering ([#1505](https://github.com/Devolutions/IronRDP/issues/1505)) ([5c1816244e](https://github.com/Devolutions/IronRDP/commit/5c1816244e83187a04249e9d9c240d096cb78f55)) 

  Hyper-V over RDCleanPath needs PCB → TLS on the proxy, then CredSSP →
  X.224 on the client. Ordinary RDCleanPath stays X.224-first.
  
  Still VERSION_1 with the same DER fields. An explicit VMConnect request
  carries a Unicode PCB payload in `preconnection_blob` with no X.224; the
  proxy encodes the binary PCB. Generic PCB requests keep their existing
  X.224-first behavior.
  
  Gateway reference implementation:
  [Devolutions/devolutions-gateway#1372](https://github.com/Devolutions/devolutions-gateway/pull/1372)
  
  Checked locally: Rust builds, formatting, Svelte typecheck, and .NET
  build. Real nested Hyper-V E2E through Gateway: Native rendered 18
  frames, Avalonia connected and rendered its first frame, and Web
  rendered a non-empty 1280×720 canvas.
  
  ---------


